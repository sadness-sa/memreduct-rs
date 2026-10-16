//! All OS operations are confined here. Native memory-list APIs are undocumented;
//! NTSTATUS is returned verbatim, without assuming a given Windows build supports it.
use crate::{
    model::{CleanReport, MemorySnapshot, Operation, OperationResult, Profile},
    AppResult,
};
use std::{
    ffi::c_void,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, SetLastError, ERROR_NOT_ALL_ASSIGNED, HANDLE, WAIT_ABANDONED,
        WAIT_OBJECT_0,
    },
    Security::{
        AdjustTokenPrivileges, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
    },
    System::{
        Memory::SetSystemFileCacheSize,
        ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION},
        SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        Threading::{
            CreateMutexW, GetCurrentProcess, OpenProcessToken, ReleaseMutex, WaitForSingleObject,
        },
    },
};

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtSetSystemInformation(class: u32, data: *const c_void, length: u32) -> i32;
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
pub fn os_error(context: &str) -> String {
    format!("{context}: {}", std::io::Error::last_os_error())
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct Privilege {
    token: Handle,
    previous: TOKEN_PRIVILEGES,
}
impl Privilege {
    fn enable(name: &str) -> AppResult<Self> {
        // SAFETY: fixed-size zeroed Win32 PODs; token is owned immediately after success.
        unsafe {
            let mut token = null_mut();
            if OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            ) == 0
            {
                return Err(os_error("OpenProcessToken"));
            }
            let token = Handle(token);
            let mut requested: TOKEN_PRIVILEGES = zeroed();
            requested.PrivilegeCount = 1;
            if LookupPrivilegeValueW(
                null(),
                wide(name).as_ptr(),
                &mut requested.Privileges[0].Luid,
            ) == 0
            {
                return Err(os_error("LookupPrivilegeValueW"));
            }
            requested.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
            let mut previous = zeroed();
            let mut returned = 0;
            SetLastError(0);
            if AdjustTokenPrivileges(
                token.0,
                0,
                &requested,
                size_of::<TOKEN_PRIVILEGES>() as u32,
                &mut previous,
                &mut returned,
            ) == 0
            {
                return Err(os_error("AdjustTokenPrivileges"));
            }
            if GetLastError() == ERROR_NOT_ALL_ASSIGNED {
                return Err(format!("缺少 {name}；请以管理员身份运行"));
            }
            Ok(Self { token, previous })
        }
    }
}
impl Drop for Privilege {
    fn drop(&mut self) {
        // SAFETY: previous state came from AdjustTokenPrivileges for this live token.
        unsafe {
            AdjustTokenPrivileges(self.token.0, 0, &self.previous, 0, null_mut(), null_mut());
        }
    }
}

struct CleanerLock(Handle);
impl CleanerLock {
    fn acquire() -> AppResult<Self> {
        // Cross-process exclusion: GUI and CLI cannot clear lists simultaneously.
        unsafe {
            let mutex = CreateMutexW(null(), 0, wide("Local\\MemReductRs.Cleaner.v1").as_ptr());
            if mutex.is_null() {
                return Err(os_error("CreateMutexW"));
            }
            let handle = Handle(mutex);
            match WaitForSingleObject(mutex, 0) {
                WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(Self(handle)),
                _ => Err("另一项清理正在进行，请稍后重试".into()),
            }
        }
    }
}
impl Drop for CleanerLock {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0 .0);
        }
    }
}

pub fn snapshot() -> AppResult<MemorySnapshot> {
    // SAFETY: both APIs receive writable buffers with the exact ABI size.
    unsafe {
        let mut memory: MEMORYSTATUSEX = zeroed();
        memory.dwLength = size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut memory) == 0 {
            return Err(os_error("GlobalMemoryStatusEx"));
        }
        let mut perf: PERFORMANCE_INFORMATION = zeroed();
        if GetPerformanceInfo(&mut perf, size_of::<PERFORMANCE_INFORMATION>() as u32) == 0 {
            return Err(os_error("GetPerformanceInfo"));
        }
        let bytes = |pages: usize| (pages as u64).saturating_mul(perf.PageSize as u64);
        Ok(MemorySnapshot {
            load_percent: memory.dwMemoryLoad,
            physical_total: memory.ullTotalPhys,
            physical_available: memory.ullAvailPhys,
            commit_total: bytes(perf.CommitTotal),
            commit_limit: bytes(perf.CommitLimit),
            system_cache: bytes(perf.SystemCache),
        })
    }
}

fn execute(operation: Operation) -> AppResult<()> {
    if operation == Operation::SystemFileCache {
        let _privilege = Privilege::enable("SeIncreaseQuotaPrivilege")?;
        // Documented Win32 API trims the cache working set using SIZE_T(-1).
        if unsafe { SetSystemFileCacheSize(usize::MAX, usize::MAX, 0) } == 0 {
            return Err(os_error("SetSystemFileCacheSize"));
        }
        return Ok(());
    }
    let _privilege = Privilege::enable("SeProfileSingleProcessPrivilege")?;
    // SYSTEM_MEMORY_LIST_COMMAND is a 32-bit C enum. SystemMemoryListInformation = 80.
    let command: u32 = match operation {
        Operation::WorkingSets => 2,
        Operation::ModifiedPages => 3,
        Operation::Standby => 4,
        Operation::LowPriorityStandby => 5,
        Operation::SystemFileCache => unreachable!(),
    };
    // SAFETY: the kernel copies a valid u32; no Rust allocation crosses the boundary.
    let status = unsafe {
        NtSetSystemInformation(80, (&command as *const u32).cast(), size_of::<u32>() as u32)
    };
    if status < 0 {
        return Err(format!(
            "NtSetSystemInformation: NTSTATUS 0x{:08X}（权限不足或当前系统不支持）",
            status as u32
        ));
    }
    Ok(())
}

pub fn clean(profile: Profile) -> AppResult<CleanReport> {
    let _lock = CleanerLock::acquire()?;
    let before = snapshot()?;
    let started = Instant::now();
    let operations = profile
        .operations()
        .iter()
        .map(|&operation| OperationResult {
            operation,
            error: execute(operation).err(),
        })
        .collect();
    let measured = snapshot();
    let measurement_error = measured.as_ref().err().cloned();
    Ok(CleanReport {
        profile,
        before,
        after: measured.ok(),
        measurement_error,
        elapsed_ms: started.elapsed().as_millis(),
        operations,
    })
}

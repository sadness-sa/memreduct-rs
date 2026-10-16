//! All OS operations are confined here. Native memory-list APIs are undocumented;
//! NTSTATUS is returned verbatim, without assuming a given Windows build supports it.
use crate::{
    config::Config,
    model::{CleanReport, MemorySnapshot, Operation, OperationResult, ProcessTrimStats, Profile},
    selection::{excluded_name, protected_family, ProcessEntry},
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
        CloseHandle, GetLastError, SetLastError, ERROR_NOT_ALL_ASSIGNED, FILETIME, HANDLE,
        INVALID_HANDLE_VALUE, WAIT_ABANDONED, WAIT_OBJECT_0,
    },
    Security::{
        AdjustTokenPrivileges, GetTokenInformation, LookupPrivilegeValueW, TokenElevation,
        SE_PRIVILEGE_ENABLED, TOKEN_ADJUST_PRIVILEGES, TOKEN_ELEVATION, TOKEN_PRIVILEGES,
        TOKEN_QUERY,
    },
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
            TH32CS_SNAPPROCESS,
        },
        Memory::SetSystemFileCacheSize,
        ProcessStatus::{
            EmptyWorkingSet, GetPerformanceInfo, GetProcessMemoryInfo, PERFORMANCE_INFORMATION,
            PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
        },
        RemoteDesktop::ProcessIdToSessionId,
        SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX},
        Threading::{
            CreateMutexW, GetCurrentProcess, GetCurrentProcessId, GetProcessTimes,
            IsProcessCritical, OpenProcess, OpenProcessToken, ReleaseMutex, WaitForSingleObject,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_QUOTA,
        },
    },
    UI::{
        Shell::{
            SHQueryUserNotificationState, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
        },
        WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
    },
};

#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtQuerySystemInformation(
        class: u32,
        data: *mut c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
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
                return Err(if is_elevated() {
                    format!("管理员令牌仍缺少 {name}；请检查本地安全策略的用户权限分配或组织策略")
                } else {
                    format!("缺少 {name}；请以管理员身份运行")
                });
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
        let lists = memory_lists();
        let own = process_memory(GetCurrentProcess()).unwrap_or_else(|| zeroed());
        Ok(MemorySnapshot {
            load_percent: memory.dwMemoryLoad,
            physical_total: memory.ullTotalPhys,
            physical_available: memory.ullAvailPhys,
            commit_total: bytes(perf.CommitTotal),
            commit_limit: bytes(perf.CommitLimit),
            system_cache: bytes(perf.SystemCache),
            standby: lists.as_ref().map(|l| bytes(l.priority.iter().sum())),
            free: lists.as_ref().map(|l| bytes(l.free.saturating_add(l.zero))),
            modified: lists.as_ref().map(|l| bytes(l.modified)),
            app_working_set: own.WorkingSetSize as u64,
            app_private: own.PrivateUsage as u64,
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
        Operation::SystemFileCache | Operation::BackgroundWorkingSets => unreachable!(),
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
    clean_with_config(profile, &Config::default(), None)
}

pub fn clean_with_config(
    profile: Profile,
    config: &Config,
    protected_pid: Option<u32>,
) -> AppResult<CleanReport> {
    config.validate()?;
    if !is_elevated() {
        return Err("尚未执行清理：当前进程未提升权限。请重新启动新版 GUI（会自动请求 UAC），或在管理员终端运行 CLI。".into());
    }
    let _lock = CleanerLock::acquire()?;
    let before = snapshot()?;
    let started = Instant::now();
    let mut operations = Vec::new();
    for &operation in profile.operations() {
        let timer = Instant::now();
        let result = if operation == Operation::BackgroundWorkingSets {
            trim_background(config, protected_pid).map(Some)
        } else {
            execute(operation).map(|()| None)
        };
        let (processes, mut error) = match result {
            Ok(p) => (p, None),
            Err(e) => (None, Some(e)),
        };
        if let Some(stats) = &processes {
            if stats.failed > 0 {
                error = Some(format!(
                    "{} 个进程回收失败，其他进程的结果已保留",
                    stats.failed
                ));
            }
        }
        operations.push(OperationResult {
            operation,
            error,
            elapsed_ms: timer.elapsed().as_millis(),
            processes,
        });
    }
    // Give counters time to settle on the worker, never block the UI thread.
    std::thread::sleep(std::time::Duration::from_millis(500));
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

pub fn is_elevated() -> bool {
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let token = Handle(token);
        let mut elevation: TOKEN_ELEVATION = zeroed();
        let mut returned = 0;
        GetTokenInformation(
            token.0,
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        ) != 0
            && elevation.TokenIsElevated != 0
    }
}

pub fn foreground_pid() -> u32 {
    unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), &mut pid);
        pid
    }
}

pub fn fullscreen_active() -> bool {
    unsafe {
        let mut state = 0;
        SHQueryUserNotificationState(&mut state) >= 0
            && matches!(state, QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE)
    }
}

#[repr(C)]
#[derive(Default)]
struct MemoryLists {
    zero: usize,
    free: usize,
    modified: usize,
    modified_no_write: usize,
    bad: usize,
    priority: [usize; 8],
    repurposed: [usize; 8],
    modified_pagefile: usize,
}
fn memory_lists() -> Option<MemoryLists> {
    let mut value = MemoryLists::default();
    let status = unsafe {
        NtQuerySystemInformation(
            80,
            (&mut value as *mut MemoryLists).cast(),
            size_of::<MemoryLists>() as u32,
            null_mut(),
        )
    };
    (status >= 0).then_some(value)
}

unsafe fn process_memory(handle: HANDLE) -> Option<PROCESS_MEMORY_COUNTERS_EX> {
    let mut info: PROCESS_MEMORY_COUNTERS_EX = zeroed();
    info.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    (GetProcessMemoryInfo(
        handle,
        (&mut info as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
        info.cb,
    ) != 0)
        .then_some(info)
}

unsafe fn cpu_ticks(handle: HANDLE) -> Option<u64> {
    let mut created: FILETIME = zeroed();
    let mut exited = zeroed();
    let mut kernel = zeroed();
    let mut user = zeroed();
    if GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) == 0 {
        return None;
    }
    let ticks = |t: FILETIME| (u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime);
    Some(ticks(kernel).saturating_add(ticks(user)))
}

fn trim_background(
    config: &Config,
    remembered_foreground: Option<u32>,
) -> AppResult<ProcessTrimStats> {
    unsafe {
        let list = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if list == INVALID_HANDLE_VALUE {
            return Err(os_error("CreateToolhelp32Snapshot"));
        }
        let _list = Handle(list);
        let mut entry: PROCESSENTRY32W = zeroed();
        entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(list, &mut entry) == 0 {
            return Err(os_error("Process32FirstW"));
        }
        let mut entries = Vec::new();
        loop {
            let end = entry
                .szExeFile
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(entry.szExeFile.len());
            entries.push(ProcessEntry {
                pid: entry.th32ProcessID,
                parent: entry.th32ParentProcessID,
                name: String::from_utf16_lossy(&entry.szExeFile[..end]),
            });
            if Process32NextW(list, &mut entry) == 0 {
                break;
            }
        }
        let own_pid = GetCurrentProcessId();
        let mut own_session = 0;
        if ProcessIdToSessionId(own_pid, &mut own_session) == 0 {
            return Err(os_error("ProcessIdToSessionId"));
        }
        let mut protected = protected_family(&entries, foreground_pid());
        if let Some(pid) = remembered_foreground {
            protected.extend(protected_family(&entries, pid));
        }
        let mut stats = ProcessTrimStats::default();
        let mut candidates = Vec::new();
        for entry in &entries {
            let mut session = 0;
            if entry.pid <= 4
                || entry.pid == own_pid
                || protected.contains(&entry.pid)
                || excluded_name(&entry.name, &config.process_exclusions)
                || ProcessIdToSessionId(entry.pid, &mut session) == 0
                || session != own_session
                || session == 0
            {
                stats.skipped += 1;
                continue;
            }
            let process = OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA,
                0,
                entry.pid,
            );
            if process.is_null() {
                stats.inaccessible += 1;
                continue;
            }
            let handle = Handle(process);
            let mut critical = 0;
            if IsProcessCritical(process, &mut critical) == 0 {
                stats.inaccessible += 1;
                continue;
            }
            if critical != 0 {
                stats.skipped += 1;
                continue;
            }
            let Some(memory) = process_memory(process) else {
                stats.inaccessible += 1;
                continue;
            };
            if (memory.WorkingSetSize as u64) < u64::from(config.minimum_working_set_mib) * 1048576
            {
                stats.skipped += 1;
                continue;
            }
            let Some(ticks) = cpu_ticks(process) else {
                stats.inaccessible += 1;
                continue;
            };
            candidates.push((entry.pid, handle, ticks));
        }
        if !candidates.is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        for (pid, handle, ticks) in candidates {
            // Protect a newly focused app even if focus changed while candidates were sampled.
            let live_protected = protected_family(&entries, foreground_pid());
            let Some(now) = cpu_ticks(handle.0) else {
                stats.inaccessible += 1;
                continue;
            };
            if live_protected.contains(&pid) || now.saturating_sub(ticks) > 100_000 {
                stats.skipped += 1;
                continue;
            }
            let before = process_memory(handle.0);
            if EmptyWorkingSet(handle.0) == 0 {
                stats.failed += 1;
                continue;
            }
            stats.trimmed += 1;
            if let (Some(before), Some(after)) = (before, process_memory(handle.0)) {
                stats.measured += 1;
                stats.working_set_reduction +=
                    before.WorkingSetSize.saturating_sub(after.WorkingSetSize) as u64;
            }
        }
        Ok(stats)
    }
}

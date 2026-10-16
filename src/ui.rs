//! Native Win32 UI. The UI thread only samples counters; cleanup runs on one worker.
//! RefCell::try_borrow_mut ignores reentrant modal callbacks rather than aliasing App.
use crate::{
    config::{append_report, config_path, Config},
    model::{gib, CleanReport, MemorySnapshot, Profile},
    platform::{clean, os_error, snapshot, wide},
    policy::AutoPolicy,
    AppResult,
};
use std::{
    cell::RefCell,
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::{null, null_mut},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    thread::{self, JoinHandle},
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
    UI::{Controls::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};

const CLASS: &str = "MemReductRs.Window.v1";
const TRAY_MESSAGE: u32 = WM_APP + 1;
const CLEAN: usize = 101;
const AUTO: usize = 102;
const PROFILE: usize = 103;
const OPEN_CONFIG: usize = 104;
const RELOAD: usize = 105;
const ELEVATE: usize = 106;
const QUIT: usize = 107;
const SHOW: usize = 108;
const HOTKEY: i32 = 1;
thread_local! { static APP: RefCell<Option<App>> = const { RefCell::new(None) }; }

struct Worker {
    tx: Option<SyncSender<Profile>>,
    rx: Receiver<AppResult<CleanReport>>,
    join: Option<JoinHandle<()>>,
}
impl Worker {
    fn new() -> AppResult<Self> {
        let (tx, requests) = mpsc::sync_channel(1);
        let (results, rx) = mpsc::channel();
        let join = thread::Builder::new()
            .name("memory-cleaner".into())
            .spawn(move || {
                while let Ok(profile) = requests.recv() {
                    if results.send(clean(profile)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            tx: Some(tx),
            rx,
            join: Some(join),
        })
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(thread) = self.join.take() {
            let _ = thread.join();
        }
    }
}

struct App {
    hwnd: HWND,
    font: HFONT,
    metrics: HWND,
    bar: HWND,
    status: HWND,
    details: HWND,
    auto_check: HWND,
    profiles: HWND,
    clean_button: HWND,
    config: Config,
    path: PathBuf,
    portable: bool,
    policy: AutoPolicy,
    clock: Instant,
    worker: Worker,
    busy: bool,
    tray_added: bool,
    hotkey_registered: bool,
    taskbar_created: u32,
    scale: f64,
    last_load: Option<u32>,
}

pub fn show_error(error: &str) {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(error).as_ptr(),
            wide("MemReduct RS").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

pub fn run() -> AppResult<()> {
    let mut portable = false;
    let mut minimized = false;
    let mut wait_for_instance = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--portable" => portable = true,
            "--minimized" => minimized = true,
            "--wait-for-instance" => wait_for_instance = true,
            _ => {
                return Err(
                    "支持参数：--portable、--minimized；命令行功能请用 memreduct-cli.exe".into(),
                )
            }
        }
    }
    // SAFETY: the instance mutex lives until after the window and worker are destroyed.
    unsafe {
        let mutex = CreateMutexW(null(), 1, wide("Local\\MemReductRs.Instance.v1").as_ptr());
        if mutex.is_null() {
            return Err(os_error("CreateMutexW"));
        }
        let duplicate = GetLastError() == ERROR_ALREADY_EXISTS;
        struct Instance {
            handle: HANDLE,
            owned: bool,
        }
        impl Drop for Instance {
            fn drop(&mut self) {
                unsafe {
                    if self.owned {
                        windows_sys::Win32::System::Threading::ReleaseMutex(self.handle);
                    }
                    CloseHandle(self.handle);
                }
            }
        }
        let mut instance = Instance {
            handle: mutex,
            owned: !duplicate,
        };
        if duplicate {
            if wait_for_instance {
                match windows_sys::Win32::System::Threading::WaitForSingleObject(mutex, 15000) {
                    WAIT_OBJECT_0 | WAIT_ABANDONED => instance.owned = true,
                    _ => return Err("等待旧实例退出超时，请手动以管理员身份启动。".into()),
                }
            } else {
                let hwnd = FindWindowW(wide(CLASS).as_ptr(), null());
                if !hwnd.is_null() {
                    ShowWindow(hwnd, SW_RESTORE);
                    SetForegroundWindow(hwnd);
                }
                return Ok(());
            }
        }
        let path = config_path(portable)?;
        let config = Config::load_or_create(&path)?;
        let start_hidden = minimized || config.start_minimized;
        let module = GetModuleHandleW(null());
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: module,
            hIcon: LoadIconW(null_mut(), IDI_APPLICATION),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
            ..zeroed()
        };
        // Keep the class-name allocation alive across RegisterClassW.
        let class_name = wide(CLASS);
        let class = WNDCLASSW {
            lpszClassName: class_name.as_ptr(),
            ..class
        };
        if RegisterClassW(&class) == 0 {
            return Err(os_error("RegisterClassW"));
        }
        let dc = GetDC(null_mut());
        let dpi = if dc.is_null() {
            96
        } else {
            let d = GetDeviceCaps(dc, LOGPIXELSX as i32);
            ReleaseDC(null_mut(), dc);
            d
        };
        let scale = dpi as f64 / 96.0;
        let sc = |n: i32| (n as f64 * scale).round() as i32;
        let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: sc(620),
            bottom: sc(505),
        };
        if AdjustWindowRectEx(&mut rect, style, 0, 0) == 0 {
            return Err(os_error("AdjustWindowRectEx"));
        }
        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            wide("MemReduct RS · 内存监控").as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            rect.right - rect.left,
            rect.bottom - rect.top,
            null_mut(),
            null_mut(),
            module,
            null(),
        );
        if hwnd.is_null() {
            return Err(os_error("CreateWindowExW"));
        }
        let app = App::new(hwnd, config, path, portable, scale);
        let app = match app {
            Ok(app) => app,
            Err(error) => {
                DestroyWindow(hwnd);
                return Err(error);
            }
        };
        let hidden = start_hidden && app.tray_added;
        APP.with(|state| {
            *state.borrow_mut() = Some(app);
        });
        if SetTimer(hwnd, 1, 1000, None) == 0 {
            DestroyWindow(hwnd);
            APP.with(|s| {
                s.borrow_mut().take();
            });
            return Err(os_error("SetTimer"));
        }
        APP.with(|state| {
            if let Some(app) = state.borrow_mut().as_mut() {
                app.tick();
            }
        });
        if !hidden {
            ShowWindow(hwnd, SW_SHOW);
            UpdateWindow(hwnd);
        }
        let mut message: MSG = zeroed();
        let outcome = loop {
            let code = GetMessageW(&mut message, null_mut(), 0, 0);
            if code == -1 {
                break Err(os_error("GetMessageW"));
            }
            if code == 0 {
                break Ok(());
            }
            // Give TAB / Shift+TAB standard native control navigation.
            if IsDialogMessageW(hwnd, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        };
        if IsWindow(hwnd) != 0 {
            DestroyWindow(hwnd);
        }
        APP.with(|state| {
            state.borrow_mut().take();
        });
        outcome
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_DESTROY {
        PostQuitMessage(0);
        return 0;
    }
    APP.with(|state| {
        if let Ok(mut state) = state.try_borrow_mut() {
            if let Some(app) = state.as_mut() {
                if app.taskbar_created != 0 && message == app.taskbar_created {
                    app.tray_added = false;
                    app.update_tray(NIM_ADD);
                    if !app.tray_added {
                        ShowWindow(hwnd, SW_SHOW);
                        app.set_status("托盘重建失败，已恢复主窗口。");
                    }
                    return 0;
                }
                match message {
                    WM_TIMER if wparam == 1 => {
                        app.tick();
                        return 0;
                    }
                    WM_COMMAND => {
                        app.command(wparam & 0xffff, (wparam >> 16) as u16);
                        return 0;
                    }
                    WM_HOTKEY if wparam == HOTKEY as usize => {
                        app.request(false);
                        return 0;
                    }
                    WM_CLOSE => {
                        app.hide_or_exit();
                        return 0;
                    }
                    WM_SIZE if wparam == SIZE_MINIMIZED as usize && app.tray_added => {
                        ShowWindow(hwnd, SW_HIDE);
                        return 0;
                    }
                    TRAY_MESSAGE => {
                        match lparam as u32 {
                            WM_LBUTTONUP => {
                                ShowWindow(hwnd, SW_RESTORE);
                                SetForegroundWindow(hwnd);
                            }
                            WM_RBUTTONUP | WM_CONTEXTMENU => app.tray_menu(),
                            _ => {}
                        }
                        return 0;
                    }
                    _ => {}
                }
            }
        } else if matches!(
            message,
            WM_TIMER | WM_COMMAND | WM_HOTKEY | WM_CLOSE | TRAY_MESSAGE
        ) {
            // MessageBox/TrackPopupMenu pump nested messages. Do not reenter App.
            return 0;
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    })
}

impl App {
    unsafe fn new(
        hwnd: HWND,
        config: Config,
        path: PathBuf,
        portable: bool,
        scale: f64,
    ) -> AppResult<Self> {
        let mut app = Self {
            hwnd,
            font: null_mut(),
            metrics: null_mut(),
            bar: null_mut(),
            status: null_mut(),
            details: null_mut(),
            auto_check: null_mut(),
            profiles: null_mut(),
            clean_button: null_mut(),
            config,
            path,
            portable,
            policy: AutoPolicy::new(0),
            clock: Instant::now(),
            worker: Worker::new()?,
            busy: false,
            tray_added: false,
            hotkey_registered: false,
            taskbar_created: RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()),
            scale,
            last_load: None,
        };
        app.font = CreateFontW(
            -app.sc(15),
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32,
            wide("Segoe UI").as_ptr(),
        );
        if app.font.is_null() {
            return Err(os_error("CreateFontW"));
        }
        let common = INITCOMMONCONTROLSEX {
            dwSize: size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_PROGRESS_CLASS,
        };
        if InitCommonControlsEx(&common) == 0 {
            return Err(os_error("InitCommonControlsEx"));
        }
        app.child(
            "STATIC",
            "MemReduct RS   |   轻量内存监控",
            0,
            20,
            16,
            580,
            24,
            0,
        )?;
        app.metrics = app.child("STATIC", "正在读取系统内存…", 0, 20, 52, 580, 80, 0)?;
        app.bar = app.child("msctls_progress32", "", PBS_SMOOTH, 20, 140, 580, 16, 0)?;
        SendMessageW(app.bar, PBM_SETRANGE32, 0, 100);
        app.child("STATIC", "手动清理策略", 0, 20, 176, 110, 24, 0)?;
        app.profiles = app.child(
            "COMBOBOX",
            "",
            CBS_DROPDOWNLIST as u32 | WS_TABSTOP | WS_VSCROLL,
            140,
            172,
            275,
            160,
            PROFILE,
        )?;
        for text in [
            "保守：低优先级待机页",
            "均衡：系统缓存 + 低优先级页",
            "强力：工作集 + 缓存 + 全部待机页",
        ] {
            SendMessageW(app.profiles, CB_ADDSTRING, 0, wide(text).as_ptr() as LPARAM);
        }
        SendMessageW(app.profiles, CB_SETCURSEL, app.profile_index(), 0);
        app.clean_button = app.child(
            "BUTTON",
            "立即清理",
            WS_TABSTOP | BS_PUSHBUTTON as u32,
            435,
            170,
            165,
            32,
            CLEAN,
        )?;
        app.auto_check = app.child(
            "BUTTON",
            "启用自动清理（始终使用保守策略）",
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            20,
            218,
            580,
            28,
            AUTO,
        )?;
        SendMessageW(
            app.auto_check,
            BM_SETCHECK,
            if app.config.auto_enabled {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            } as usize,
            0,
        );
        app.child(
            "BUTTON",
            "编辑配置",
            WS_TABSTOP,
            20,
            260,
            120,
            30,
            OPEN_CONFIG,
        )?;
        app.child("BUTTON", "重载配置", WS_TABSTOP, 150, 260, 120, 30, RELOAD)?;
        app.child(
            "BUTTON",
            "管理员重启",
            WS_TABSTOP,
            280,
            260,
            150,
            30,
            ELEVATE,
        )?;
        app.child("BUTTON", "退出", WS_TABSTOP, 440, 260, 160, 30, QUIT)?;
        app.status = app.child(
            "STATIC",
            "就绪。清理需要管理员权限；关闭窗口会隐藏到托盘。",
            0,
            20,
            308,
            580,
            44,
            0,
        )?;
        app.details = app.child(
            "EDIT",
            "清理结果将显示在这里。\r\n可用内存变化包含系统并发活动，不能视为性能收益。",
            WS_TABSTOP
                | WS_VSCROLL
                | ES_MULTILINE as u32
                | ES_READONLY as u32
                | ES_AUTOVSCROLL as u32
                | WS_BORDER,
            20,
            358,
            580,
            100,
            0,
        )?;
        app.child(
            "STATIC",
            "快捷键 Ctrl+Alt+M   ·   托盘右键显示菜单   ·   0.1.0",
            0,
            20,
            474,
            580,
            22,
            0,
        )?;
        app.update_tray(NIM_ADD);
        app.register_hotkey();
        if !app.tray_added {
            app.set_status("托盘创建失败：窗口关闭将退出，最小化保留在任务栏。");
        }
        Ok(app)
    }
    fn sc(&self, n: i32) -> i32 {
        (n as f64 * self.scale).round() as i32
    }
    #[allow(clippy::too_many_arguments)]
    unsafe fn child(
        &self,
        class: &str,
        text: &str,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        id: usize,
    ) -> AppResult<HWND> {
        let child = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            self.sc(x),
            self.sc(y),
            self.sc(w),
            self.sc(h),
            self.hwnd,
            id as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        if child.is_null() {
            return Err(os_error("创建界面控件"));
        }
        SendMessageW(child, WM_SETFONT, self.font as usize, 1);
        Ok(child)
    }
    fn profile_index(&self) -> usize {
        match self.config.manual_profile {
            Profile::Conservative => 0,
            Profile::Balanced => 1,
            Profile::Aggressive => 2,
        }
    }
    unsafe fn set_status(&self, text: &str) {
        SetWindowTextW(self.status, wide(text).as_ptr());
    }

    unsafe fn tick(&mut self) {
        match self.worker.rx.try_recv() {
            Ok(result) => {
                self.busy = false;
                EnableWindow(self.clean_button, 1);
                match result {
                    Ok(report) => {
                        self.set_status(&report.summary());
                        let mut lines = vec![report.summary()];
                        for op in &report.operations {
                            lines.push(format!(
                                "{}：{}",
                                op.operation.label(),
                                op.error.as_deref().unwrap_or("成功")
                            ));
                        }
                        if let Some(e) = &report.measurement_error {
                            lines.push(format!("测量失败：{e}"));
                        }
                        if let Err(e) = append_report(&self.path, &report) {
                            lines.push(format!("日志写入失败：{e}"));
                        }
                        SetWindowTextW(self.details, wide(&lines.join("\r\n")).as_ptr());
                    }
                    Err(e) => {
                        self.set_status(&e);
                        SetWindowTextW(self.details, wide(&e).as_ptr());
                    }
                }
            }
            Err(TryRecvError::Disconnected) => {
                self.busy = false;
                self.worker.tx.take();
                self.config.auto_enabled = false;
                EnableWindow(self.clean_button, 0);
                self.set_status("清理线程已退出，请重启应用。");
            }
            Err(TryRecvError::Empty) => {}
        }
        match snapshot() {
            Ok(memory) => {
                self.render_memory(memory);
                if self
                    .policy
                    .observe(
                        &self.config,
                        self.clock.elapsed().as_secs(),
                        memory.load_percent,
                        self.busy,
                    )
                    .is_some()
                {
                    self.request(true);
                }
            }
            Err(e) => {
                // A measurement gap breaks the sustained-pressure observation window.
                self.policy.measurement_gap();
                self.set_status(&e);
            }
        }
    }

    unsafe fn render_memory(&mut self, m: MemorySnapshot) {
        let text = format!("物理内存：{}%    已用 {} / {}    可用 {}\r\n提交量：{} / {}\r\n系统缓存：{}    自动阈值：{}% / 持续 {} 秒 / 冷却 {} 秒",
            m.load_percent, gib(m.physical_total.saturating_sub(m.physical_available)), gib(m.physical_total), gib(m.physical_available),
            gib(m.commit_total), gib(m.commit_limit), gib(m.system_cache), self.config.threshold_percent, self.config.sustained_seconds, self.config.cooldown_seconds);
        SetWindowTextW(self.metrics, wide(&text).as_ptr());
        SendMessageW(self.bar, PBM_SETPOS, m.load_percent as usize, 0);
        if self.last_load != Some(m.load_percent) {
            self.last_load = Some(m.load_percent);
            self.update_tray(NIM_MODIFY);
        }
    }

    unsafe fn request(&mut self, automatic: bool) {
        if self.busy {
            self.set_status("清理正在进行，请稍候。");
            return;
        }
        let profile = if automatic {
            Profile::Conservative
        } else {
            self.config.manual_profile
        };
        if profile == Profile::Aggressive && MessageBoxW(self.hwnd,
            wide("强力清理将清空工作集、刷新已修改页并清除全部待机页，可能造成磁盘写入、缺页和短暂卡顿。是否继续？").as_ptr(),
            wide("确认强力清理").as_ptr(), MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) != IDYES { return; }
        if let Some(tx) = &self.worker.tx {
            match tx.try_send(profile) {
                Ok(()) => {
                    self.busy = true;
                    EnableWindow(self.clean_button, 0);
                    self.policy.attempted(self.clock.elapsed().as_secs());
                    self.set_status("后台清理中…");
                }
                Err(e) => self.set_status(&format!("无法提交清理任务：{e}")),
            }
        }
    }

    unsafe fn save_config(&mut self, new: Config) {
        match new.save(&self.path) {
            Ok(()) => self.config = new,
            Err(e) => {
                SendMessageW(
                    self.auto_check,
                    BM_SETCHECK,
                    if self.config.auto_enabled {
                        BST_CHECKED
                    } else {
                        BST_UNCHECKED
                    } as usize,
                    0,
                );
                SendMessageW(self.profiles, CB_SETCURSEL, self.profile_index(), 0);
                self.set_status(&e);
            }
        }
    }

    unsafe fn command(&mut self, id: usize, notification: u16) {
        match id {
            CLEAN => self.request(false),
            AUTO => {
                let mut config = self.config.clone();
                config.auto_enabled =
                    SendMessageW(self.auto_check, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
                self.save_config(config);
                self.policy.reconfigure(self.clock.elapsed().as_secs());
            }
            PROFILE if notification == CBN_SELCHANGE as u16 => {
                let mut config = self.config.clone();
                config.manual_profile = match SendMessageW(self.profiles, CB_GETCURSEL, 0, 0) {
                    1 => Profile::Balanced,
                    2 => Profile::Aggressive,
                    _ => Profile::Conservative,
                };
                self.save_config(config);
            }
            OPEN_CONFIG => {
                if let Err(e) = std::process::Command::new("notepad.exe")
                    .arg(&self.path)
                    .spawn()
                {
                    self.set_status(&format!("打开配置失败：{e}"));
                } else {
                    self.set_status("保存配置后点击「重载配置」。");
                }
            }
            RELOAD => {
                if self.busy {
                    self.set_status("请在清理完成后重载配置。");
                    return;
                }
                match Config::load_or_create(&self.path) {
                    Ok(c) => {
                        self.config = c;
                        self.policy.reconfigure(self.clock.elapsed().as_secs());
                        SendMessageW(
                            self.auto_check,
                            BM_SETCHECK,
                            if self.config.auto_enabled {
                                BST_CHECKED
                            } else {
                                BST_UNCHECKED
                            } as usize,
                            0,
                        );
                        SendMessageW(self.profiles, CB_SETCURSEL, self.profile_index(), 0);
                        self.set_status("已重载配置。");
                        self.register_hotkey();
                    }
                    Err(e) => self.set_status(&e),
                }
            }
            ELEVATE => self.elevate(),
            SHOW => {
                ShowWindow(self.hwnd, SW_RESTORE);
                SetForegroundWindow(self.hwnd);
            }
            QUIT => self.exit(),
            _ => {}
        }
    }
    unsafe fn register_hotkey(&mut self) {
        if self.hotkey_registered {
            UnregisterHotKey(self.hwnd, HOTKEY);
            self.hotkey_registered = false;
        }
        if self.config.hotkey_enabled {
            self.hotkey_registered = RegisterHotKey(
                self.hwnd,
                HOTKEY,
                MOD_CONTROL | MOD_ALT | MOD_NOREPEAT,
                b'M' as u32,
            ) != 0;
            if !self.hotkey_registered {
                self.set_status("快捷键 Ctrl+Alt+M 被占用，可使用按钮或托盘菜单。");
            }
        }
    }
    unsafe fn update_tray(&mut self, action: u32) {
        let mut data: NOTIFYICONDATAW = zeroed();
        data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = self.hwnd;
        data.uID = 1;
        data.uFlags = NIF_MESSAGE | NIF_TIP | NIF_ICON;
        data.uCallbackMessage = TRAY_MESSAGE;
        data.hIcon = LoadIconW(null_mut(), IDI_APPLICATION);
        let tip = wide(&format!(
            "MemReduct RS | 物理内存 {}% | Ctrl+Alt+M",
            self.last_load.unwrap_or(0)
        ));
        for (dest, src) in data.szTip.iter_mut().take(127).zip(tip.iter()) {
            *dest = *src;
        }
        if action == NIM_ADD {
            self.tray_added = Shell_NotifyIconW(NIM_ADD, &data) != 0;
        } else if self.tray_added && Shell_NotifyIconW(NIM_MODIFY, &data) == 0 {
            self.tray_added = Shell_NotifyIconW(NIM_ADD, &data) != 0;
            if !self.tray_added {
                ShowWindow(self.hwnd, SW_SHOW);
                self.set_status("托盘不可用，已恢复主窗口。");
            }
        }
    }
    unsafe fn tray_menu(&mut self) {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        AppendMenuW(menu, MF_STRING, SHOW, wide("显示主窗口").as_ptr());
        AppendMenuW(
            menu,
            MF_STRING | if self.busy { MF_GRAYED } else { 0 },
            CLEAN,
            wide("立即清理").as_ptr(),
        );
        AppendMenuW(menu, MF_SEPARATOR, 0, null());
        AppendMenuW(menu, MF_STRING, QUIT, wide("退出").as_ptr());
        let mut point: POINT = zeroed();
        GetCursorPos(&mut point);
        SetForegroundWindow(self.hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON,
            point.x,
            point.y,
            0,
            self.hwnd,
            null(),
        );
        DestroyMenu(menu);
        PostMessageW(self.hwnd, WM_NULL, 0, 0);
        if command != 0 {
            self.command(command as usize, 0);
        }
    }
    unsafe fn elevate(&mut self) {
        if self.busy {
            self.set_status("请在清理完成后重启。");
            return;
        }
        // A short-lived helper waits for this instance mutex, then starts the elevated GUI.
        // Release the mutex only after the current process shuts down; no race/duplicate instance.
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                self.set_status(&e.to_string());
                return;
            }
        };
        let parameters = if self.portable {
            "--wait-for-instance --portable"
        } else {
            "--wait-for-instance"
        };
        let result = ShellExecuteW(
            self.hwnd,
            wide("runas").as_ptr(),
            wide(&exe.to_string_lossy()).as_ptr(),
            wide(parameters).as_ptr(),
            null(),
            SW_SHOWNORMAL,
        );
        if result as isize <= 32 {
            self.set_status("管理员重启取消或失败。");
        } else {
            DestroyWindow(self.hwnd);
        }
    }
    unsafe fn hide_or_exit(&mut self) {
        if self.tray_added {
            ShowWindow(self.hwnd, SW_HIDE);
        } else {
            self.exit();
        }
    }
    unsafe fn exit(&mut self) {
        if self.busy {
            self.set_status("请等待后台清理完成后退出。");
        } else {
            DestroyWindow(self.hwnd);
        }
    }
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            KillTimer(self.hwnd, 1);
            if self.hotkey_registered {
                UnregisterHotKey(self.hwnd, HOTKEY);
            }
            if self.tray_added {
                let mut data: NOTIFYICONDATAW = zeroed();
                data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
                data.hWnd = self.hwnd;
                data.uID = 1;
                Shell_NotifyIconW(NIM_DELETE, &data);
            }
            if !self.font.is_null() {
                DeleteObject(self.font);
            }
        }
    }
}

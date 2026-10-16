//! Native Win32 UI. The UI thread only samples counters; cleanup runs on one worker.
//! RefCell::try_borrow_mut ignores reentrant modal callbacks rather than aliasing App.
use crate::{
    config::{append_report, config_path, Config},
    model::{gib, CleanReport, MemorySnapshot, Profile},
    platform::{
        clean_with_config, foreground_pid, fullscreen_active, is_elevated, os_error, snapshot, wide,
    },
    policy::AutoPolicy,
    settings::{self, SettingsPanel},
    strategy,
    tray::{number_icon, NumberIcon},
    AppResult,
};
use std::{
    cell::RefCell,
    collections::VecDeque,
    mem::{size_of, zeroed},
    path::PathBuf,
    ptr::{null, null_mut},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    thread::{self, JoinHandle},
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    System::{LibraryLoader::GetModuleHandleW, Threading::CreateMutexW},
    UI::{Controls::*, Input::KeyboardAndMouse::*, Shell::*, WindowsAndMessaging::*},
};

const CLASS: &str = "MemReductRs.Window.v1";
const TRAY_MESSAGE: u32 = WM_APP + 1;
const CLEAN: usize = 101;
const AUTO: usize = 102;
const PROFILE_BASE: usize = 120;
const MINIMIZE: usize = 109;
const HIDE: usize = 110;
const OPEN_CONFIG: usize = 104;
const RELOAD: usize = 105;
const AUTO_PROFILE: usize = 106;
const SHUTDOWN: u32 = WM_APP + 4;
const QUIT: usize = 107;
const SHOW: usize = 108;
const HOTKEY: i32 = 1;
thread_local! { static APP: RefCell<Option<App>> = const { RefCell::new(None) }; }

struct Request {
    profile: Profile,
    config: Config,
    protected_pid: Option<u32>,
}

struct Worker {
    tx: Option<SyncSender<Request>>,
    rx: Receiver<AppResult<CleanReport>>,
    join: Option<JoinHandle<()>>,
}
impl Worker {
    fn new() -> AppResult<Self> {
        let (tx, requests) = mpsc::sync_channel::<Request>(1);
        let (results, rx) = mpsc::channel();
        let join = thread::Builder::new()
            .name("memory-cleaner".into())
            .spawn(move || {
                while let Ok(request) = requests.recv() {
                    if results
                        .send(clean_with_config(
                            request.profile,
                            &request.config,
                            request.protected_pid,
                        ))
                        .is_err()
                    {
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
    heading_font: HFONT,
    number_font: HFONT,
    small_font: HFONT,
    background: HBRUSH,
    surface: HBRUSH,
    memory: Option<MemorySnapshot>,
    history: VecDeque<u32>,
    last_sample: Instant,
    remembered_foreground: Option<u32>,
    auto_profiles: HWND,
    status: HWND,
    details: HWND,
    auto_check: HWND,
    profile_buttons: Vec<HWND>,
    overview_controls: Vec<HWND>,
    settings: Option<SettingsPanel>,
    tray_icon: Option<NumberIcon>,
    clean_button: HWND,
    config: Config,
    path: PathBuf,
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
            hIcon: LoadIconW(GetModuleHandleW(null()), 101usize as *const u16),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: null_mut(),
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
        let scale = (dpi as f64 / 96.0)
            .min((GetSystemMetrics(SM_CYSCREEN) - 90) as f64 / 710.0)
            .max(0.75);
        let sc = |n: i32| (n as f64 * scale).round() as i32;
        let style = WS_POPUP | WS_SYSMENU | WS_MINIMIZEBOX;
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: sc(820),
            bottom: sc(710),
        };
        if AdjustWindowRectEx(&mut rect, style, 0, 0) == 0 {
            return Err(os_error("AdjustWindowRectEx"));
        }
        let mut work: RECT = zeroed();
        if SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work as *mut RECT).cast(), 0) == 0 {
            work.right = GetSystemMetrics(SM_CXSCREEN);
            work.bottom = GetSystemMetrics(SM_CYSCREEN);
        }
        let hwnd = CreateWindowExW(
            WS_EX_APPWINDOW,
            class_name.as_ptr(),
            wide("MemReduct RS · 内存监控").as_ptr(),
            style | WS_CLIPCHILDREN,
            work.left + (work.right - work.left - sc(820)) / 2,
            work.top + (work.bottom - work.top - sc(710)) / 2,
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
        let preference: u32 = 2;
        if DwmSetWindowAttribute(
            hwnd,
            33,
            (&preference as *const u32).cast(),
            size_of::<u32>() as u32,
        ) < 0
        {
            let region = CreateRoundRectRgn(0, 0, sc(820) + 1, sc(710) + 1, sc(18), sc(18));
            if !region.is_null() && SetWindowRgn(hwnd, region, 0) == 0 {
                DeleteObject(region);
            }
        }
        let margins = MARGINS {
            cxLeftWidth: 1,
            cxRightWidth: 1,
            cyTopHeight: 1,
            cyBottomHeight: 1,
        };
        DwmExtendFrameIntoClientArea(hwnd, &margins);
        let app = App::new(hwnd, config, path, scale);
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
    if message == WM_PRINT {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
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
                    WM_NCHITTEST => {
                        let mut point = POINT {
                            x: lparam as u16 as i16 as i32,
                            y: (lparam >> 16) as u16 as i16 as i32,
                        };
                        ScreenToClient(hwnd, &mut point);
                        return if point.y < app.sc(80) && point.x < app.sc(712) {
                            HTCAPTION as LRESULT
                        } else {
                            HTCLIENT as LRESULT
                        };
                    }
                    WM_NCLBUTTONDBLCLK if wparam == HTCAPTION as usize => return 0,
                    WM_PAINT => {
                        app.paint();
                        return 0;
                    }
                    WM_PRINTCLIENT => {
                        app.paint_to(wparam as HDC);
                        return 0;
                    }
                    WM_ERASEBKGND => return 1,
                    WM_DRAWITEM if lparam != 0 => {
                        app.draw_button(&*(lparam as *const DRAWITEMSTRUCT));
                        return 1;
                    }
                    WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT | WM_CTLCOLORBTN => {
                        let dc = wparam as HDC;
                        SetTextColor(dc, rgb(38, 48, 68));
                        SetBkColor(dc, rgb(255, 255, 255));
                        return app.surface as LRESULT;
                    }
                    SHUTDOWN => {
                        app.exit();
                        return 0;
                    }
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
    unsafe fn new(hwnd: HWND, config: Config, path: PathBuf, scale: f64) -> AppResult<Self> {
        let mut app = Self {
            hwnd,
            font: null_mut(),
            heading_font: null_mut(),
            number_font: null_mut(),
            small_font: null_mut(),
            background: CreateSolidBrush(rgb(255, 255, 255)),
            surface: CreateSolidBrush(rgb(255, 255, 255)),
            memory: None,
            history: VecDeque::with_capacity(60),
            last_sample: Instant::now(),
            remembered_foreground: None,
            auto_profiles: null_mut(),
            status: null_mut(),
            details: null_mut(),
            auto_check: null_mut(),
            profile_buttons: Vec::new(),
            overview_controls: Vec::new(),
            settings: None,
            tray_icon: None,
            clean_button: null_mut(),
            config,
            path,
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
        app.font = app.make_font(15, FW_NORMAL);
        app.heading_font = app.make_font(24, FW_SEMIBOLD);
        app.number_font = app.make_font(58, FW_SEMIBOLD);
        app.small_font = app.make_font(13, FW_NORMAL);
        if [
            app.font,
            app.heading_font,
            app.number_font,
            app.small_font,
            app.background,
            app.surface,
        ]
        .iter()
        .any(|p| p.is_null())
        {
            return Err(os_error("创建界面资源"));
        }
        for (i, profile) in Profile::ALL.iter().enumerate() {
            let control = app.child(
                "BUTTON",
                profile.label(),
                WS_TABSTOP | BS_OWNERDRAW as u32,
                44 + (i % 4) as i32 * 120,
                330 + (i / 4) as i32 * 36,
                112,
                28,
                PROFILE_BASE + i,
            )?;
            app.profile_buttons.push(control);
        }
        app.clean_button = app.child(
            "BUTTON",
            "智能优化",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            546,
            330,
            230,
            64,
            CLEAN,
        )?;
        app.auto_check = app.child(
            "BUTTON",
            "自动优化",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            44,
            448,
            228,
            32,
            AUTO,
        )?;
        app.auto_profiles = app.child(
            "BUTTON",
            "",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            508,
            448,
            268,
            32,
            AUTO_PROFILE,
        )?;
        app.sync_config_controls();
        app.status = app.child(
            "STATIC",
            "就绪 · 智能选策会根据当前内存状态选择回收方式。",
            0,
            44,
            550,
            732,
            36,
            0,
        )?;
        app.details = app.child(
            "EDIT",
            "待机页本来已计入可用内存；结果会同时显示内存占用、缓存及工作集变化。",
            WS_TABSTOP
                | WS_VSCROLL
                | ES_MULTILINE as u32
                | ES_READONLY as u32
                | ES_AUTOVSCROLL as u32,
            44,
            590,
            732,
            46,
            0,
        )?;
        SendMessageW(app.details, WM_SETFONT, app.small_font as usize, 1);
        for (text, x, w, id) in [
            ("优化设置", 24, 114, OPEN_CONFIG),
            ("刷新状态", 150, 114, RELOAD),
            ("退出程序", 682, 114, QUIT),
        ] {
            let control = app.child(
                "BUTTON",
                text,
                WS_TABSTOP | BS_OWNERDRAW as u32,
                x,
                666,
                w,
                30,
                id,
            )?;
            app.overview_controls.push(control);
        }
        app.overview_controls
            .extend(app.profile_buttons.iter().copied());
        app.overview_controls.extend([
            app.clean_button,
            app.auto_check,
            app.auto_profiles,
            app.status,
            app.details,
        ]);
        app.child(
            "BUTTON",
            "最小化",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            718,
            20,
            36,
            32,
            MINIMIZE,
        )?;
        app.child(
            "BUTTON",
            "关闭到托盘",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            766,
            20,
            36,
            32,
            HIDE,
        )?;
        app.update_tray(NIM_ADD);
        app.register_hotkey();
        if !app.tray_added {
            app.set_status("托盘创建失败：窗口关闭将退出，最小化保留在任务栏。");
        }
        Ok(app)
    }
    unsafe fn make_font(&self, height: i32, weight: u32) -> HFONT {
        CreateFontW(
            -self.sc(height),
            0,
            0,
            0,
            weight as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            DEFAULT_PITCH as u32,
            wide("Segoe UI").as_ptr(),
        )
    }
    unsafe fn sync_config_controls(&self) {
        SetWindowTextW(
            self.auto_check,
            wide(if self.config.auto_enabled {
                "自动优化 · 已开启"
            } else {
                "自动优化 · 已关闭"
            })
            .as_ptr(),
        );
        SetWindowTextW(
            self.clean_button,
            wide(if self.config.manual_profile == Profile::Smart {
                "智能优化"
            } else {
                "开始回收"
            })
            .as_ptr(),
        );
        SetWindowTextW(
            self.auto_profiles,
            wide(&format!(
                "自动策略：{}  ›",
                self.config.auto_profile.label()
            ))
            .as_ptr(),
        );
        InvalidateRect(self.hwnd, null(), 0);
        for &control in self.profile_buttons.iter().chain([self.auto_check].iter()) {
            InvalidateRect(control, null(), 0);
        }
    }
    unsafe fn close_settings(&mut self) {
        self.settings.take();
        for &control in &self.overview_controls {
            ShowWindow(control, SW_SHOW);
        }
        self.sync_config_controls();
    }
    fn rect(&self, x: i32, y: i32, width: i32, height: i32) -> RECT {
        RECT {
            left: self.sc(x),
            top: self.sc(y),
            right: self.sc(x + width),
            bottom: self.sc(y + height),
        }
    }
    unsafe fn text(&self, dc: HDC, text: &str, rect: RECT, font: HFONT, color: COLORREF) {
        let old = SelectObject(dc, font);
        SetTextColor(dc, color);
        SetBkMode(dc, TRANSPARENT as i32);
        let mut rect = rect;
        DrawTextW(
            dc,
            wide(text).as_ptr(),
            -1,
            &mut rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
        );
        SelectObject(dc, old);
    }
    unsafe fn paint(&self) {
        let mut ps: PAINTSTRUCT = zeroed();
        let target = BeginPaint(self.hwnd, &mut ps);
        self.paint_to(target);
        EndPaint(self.hwnd, &ps);
    }
    unsafe fn paint_to(&self, target: HDC) {
        let mut client: RECT = zeroed();
        GetClientRect(self.hwnd, &mut client);
        let buffer = CreateCompatibleDC(target);
        let bitmap = CreateCompatibleBitmap(target, client.right, client.bottom);
        if buffer.is_null() || bitmap.is_null() {
            if !bitmap.is_null() {
                DeleteObject(bitmap);
            }
            if !buffer.is_null() {
                DeleteDC(buffer);
            }
            return;
        }
        let old = SelectObject(buffer, bitmap);
        FillRect(buffer, &client, self.background);
        let ink = rgb(29, 40, 63);
        let muted = rgb(111, 123, 143);
        let blue = rgb(59, 100, 245);
        self.text(
            buffer,
            "MemReduct",
            self.rect(66, 17, 430, 34),
            self.heading_font,
            ink,
        );
        self.text(
            buffer,
            "内存监控与智能回收",
            self.rect(67, 50, 400, 20),
            self.small_font,
            muted,
        );
        rounded(
            buffer,
            self.rect(532, 24, 162, 28),
            self.sc(14),
            rgb(225, 244, 236),
            rgb(225, 244, 236),
        );
        self.text(
            buffer,
            if is_elevated() {
                "  管理员权限已就绪"
            } else {
                "  需要管理员权限"
            },
            self.rect(538, 24, 150, 28),
            self.small_font,
            rgb(27, 128, 90),
        );
        DrawIconEx(
            buffer,
            self.sc(25),
            self.sc(26),
            LoadIconW(GetModuleHandleW(null()), 101usize as *const u16),
            self.sc(28),
            self.sc(28),
            0,
            null_mut(),
            DI_NORMAL,
        );
        line(
            buffer,
            self.sc(24),
            self.sc(80),
            self.sc(796),
            self.sc(80),
            rgb(235, 239, 246),
            1,
        );
        if self.settings.is_some() {
            self.text(
                buffer,
                "优化设置",
                self.rect(44, 96, 680, 30),
                self.heading_font,
                ink,
            );
            BitBlt(
                target,
                0,
                0,
                client.right,
                client.bottom,
                buffer,
                0,
                0,
                SRCCOPY,
            );
            SelectObject(buffer, old);
            DeleteObject(bitmap);
            DeleteDC(buffer);
            return;
        }
        for y in [278, 428, 522, 650] {
            line(
                buffer,
                self.sc(44),
                self.sc(y),
                self.sc(776),
                self.sc(y),
                rgb(235, 239, 246),
                1,
            );
        }
        self.text(
            buffer,
            "物理内存占用",
            self.rect(44, 101, 245, 24),
            self.font,
            muted,
        );
        let percent = self
            .memory
            .map(|m| format!("{}%", m.load_percent))
            .unwrap_or_else(|| "--%".into());
        self.text(
            buffer,
            &percent,
            self.rect(39, 123, 275, 80),
            self.number_font,
            ink,
        );
        let subtitle = self
            .memory
            .map(|m| {
                format!(
                    "已用 {} / {}",
                    gib(m.physical_total.saturating_sub(m.physical_available)),
                    gib(m.physical_total)
                )
            })
            .unwrap_or_else(|| "正在读取系统状态…".into());
        self.text(
            buffer,
            &subtitle,
            self.rect(44, 207, 285, 24),
            self.small_font,
            muted,
        );
        self.text(
            buffer,
            "内存使用趋势",
            self.rect(340, 103, 220, 20),
            self.small_font,
            muted,
        );
        for y in [140, 160, 180] {
            line(
                buffer,
                self.sc(340),
                self.sc(y),
                self.sc(774),
                self.sc(y),
                rgb(238, 242, 248),
                1,
            );
        }
        let values: Vec<_> = self.history.iter().copied().collect();
        for (i, pair) in values.windows(2).enumerate() {
            let x = |n: usize| self.sc(340) + self.sc(434) * n as i32 / 59;
            let y = |load: u32| self.sc(193) - self.sc(62) * load as i32 / 100;
            line(
                buffer,
                x(i),
                y(pair[0]),
                x(i + 1),
                y(pair[1]),
                blue,
                self.sc(2).max(1),
            );
        }
        rounded(
            buffer,
            self.rect(340, 205, 434, 8),
            self.sc(4),
            rgb(232, 238, 252),
            rgb(232, 238, 252),
        );
        if let Some(m) = self.memory {
            let width = (434 * m.load_percent as i32 / 100).max(4);
            rounded(
                buffer,
                self.rect(340, 205, width, 8),
                self.sc(4),
                blue,
                blue,
            );
            self.text(
                buffer,
                &format!("可用 {}", gib(m.physical_available)),
                self.rect(44, 240, 245, 20),
                self.small_font,
                ink,
            );
            let cache = m.standby.map(gib).unwrap_or_else(|| "不可查询".into());
            self.text(
                buffer,
                &format!("待机缓存 {}", cache),
                self.rect(306, 240, 250, 20),
                self.small_font,
                ink,
            );
            self.text(
                buffer,
                &format!("本程序 {:.1} MiB", m.app_working_set as f64 / 1048576.0),
                self.rect(583, 240, 191, 20),
                self.small_font,
                ink,
            );
        }
        self.text(
            buffer,
            "优化策略",
            self.rect(44, 302, 240, 25),
            self.font,
            ink,
        );
        let description = if self.config.manual_profile == Profile::Smart {
            self.memory
                .map(|m| {
                    let plan = strategy::select(Profile::Smart, &m, &self.config);
                    format!(
                        "当前建议：{} · {}",
                        if plan.operations.is_empty() {
                            "无需回收"
                        } else {
                            plan.selected.label()
                        },
                        plan.reason
                    )
                })
                .unwrap_or_else(|| "采样后自动选择合适的回收策略".into())
        } else {
            match self.config.manual_profile {
                Profile::Conservative => "仅处理低优先级待机页，保留应用工作集。",
                Profile::Background => "仅回收空闲后台进程；排除前台进程树和用户列表。",
                Profile::Cache => "仅收缩系统文件缓存工作集。",
                Profile::Standby => "处理全部待机缓存，已计入可用内存的页面不重复计算。",
                Profile::Balanced => "空闲后台工作集 + 系统缓存 + 低优先级待机页。",
                Profile::Recovery => "内存压力回收：空闲后台工作集 + 系统缓存 + 全部待机页。",
                Profile::Aggressive => "全局工作集 + 修改页 + 全部缓存；每次确认，不使用进程排除。",
                Profile::Smart => unreachable!(),
            }
            .into()
        };
        self.text(
            buffer,
            &description,
            self.rect(44, 399, 732, 22),
            self.small_font,
            muted,
        );
        let auto = format!(
            "≥ {}% 持续 {} 秒 · 冷却 {} 秒 · {}",
            self.config.threshold_percent,
            self.config.sustained_seconds,
            self.config.cooldown_seconds,
            if self.config.skip_fullscreen {
                "全屏时暂停自动优化"
            } else {
                "全屏暂停已关闭"
            }
        );
        self.text(
            buffer,
            &auto,
            self.rect(44, 484, 732, 22),
            self.small_font,
            muted,
        );
        self.text(
            buffer,
            "v0.3.0  ·  Ctrl+Alt+M  ·  关闭窗口隐藏到托盘",
            self.rect(284, 668, 385, 24),
            self.small_font,
            muted,
        );
        BitBlt(
            target,
            0,
            0,
            client.right,
            client.bottom,
            buffer,
            0,
            0,
            SRCCOPY,
        );
        SelectObject(buffer, old);
        DeleteObject(bitmap);
        DeleteDC(buffer);
    }
    unsafe fn draw_button(&self, item: &DRAWITEMSTRUCT) {
        if item.CtlType != ODT_BUTTON {
            return;
        }
        let id = item.CtlID as usize;
        let primary = id == CLEAN || id == settings::SAVE;
        let selected = (PROFILE_BASE..PROFILE_BASE + Profile::ALL.len()).contains(&id)
            && Profile::ALL[id - PROFILE_BASE] == self.config.manual_profile;
        let active = selected || (id == AUTO && self.config.auto_enabled);
        let disabled = item.itemState & ODS_DISABLED != 0;
        let pressed = item.itemState & ODS_SELECTED != 0;
        let chrome = matches!(id, MINIMIZE | HIDE);
        let fill = if disabled {
            rgb(193, 204, 227)
        } else if chrome && pressed && id == HIDE {
            rgb(254, 226, 226)
        } else if primary && pressed {
            rgb(39, 73, 199)
        } else if primary {
            rgb(59, 100, 245)
        } else if active {
            rgb(232, 238, 255)
        } else if pressed {
            rgb(226, 233, 246)
        } else if chrome {
            rgb(255, 255, 255)
        } else {
            rgb(246, 248, 252)
        };
        FillRect(item.hDC, &item.rcItem, self.background);
        rounded(item.hDC, item.rcItem, self.sc(8), fill, fill);
        if chrome {
            let rect = item.rcItem;
            let cx = (rect.left + rect.right) / 2;
            let cy = (rect.top + rect.bottom) / 2;
            let d = self.sc(5);
            let color = rgb(70, 82, 104);
            if id == MINIMIZE {
                line(
                    item.hDC,
                    cx - d,
                    cy,
                    cx + d + 1,
                    cy,
                    color,
                    self.sc(1).max(1),
                );
            } else {
                line(
                    item.hDC,
                    cx - d,
                    cy - d,
                    cx + d + 1,
                    cy + d + 1,
                    color,
                    self.sc(1).max(1),
                );
                line(
                    item.hDC,
                    cx - d,
                    cy + d,
                    cx + d + 1,
                    cy - d - 1,
                    color,
                    self.sc(1).max(1),
                );
            }
        } else {
            let mut label = [0u16; 128];
            GetWindowTextW(item.hwndItem, label.as_mut_ptr(), label.len() as i32);
            let old = SelectObject(item.hDC, self.font);
            SetBkMode(item.hDC, TRANSPARENT as i32);
            SetTextColor(
                item.hDC,
                if primary {
                    rgb(255, 255, 255)
                } else if active {
                    rgb(59, 100, 245)
                } else {
                    rgb(74, 85, 108)
                },
            );
            let mut rect = item.rcItem;
            DrawTextW(
                item.hDC,
                label.as_ptr(),
                -1,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE,
            );
            SelectObject(item.hDC, old);
        }
        if item.itemState & ODS_FOCUS != 0 {
            let mut rect = item.rcItem;
            InflateRect(&mut rect, -self.sc(4), -self.sc(4));
            DrawFocusRect(item.hDC, &rect);
        }
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
                        let mut lines = vec![
                            format!(
                                "选用{}：{}",
                                report.selected_profile.label(),
                                report.decision
                            ),
                            report.measurement_details(),
                        ];
                        lines.extend(report.operations.iter().map(|op| op.detail()));
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
        let foreground = foreground_pid();
        if foreground != 0 && foreground != std::process::id() {
            self.remembered_foreground = Some(foreground);
        }
        if self.memory.is_some()
            && IsWindowVisible(self.hwnd) == 0
            && self.last_sample.elapsed().as_secs() < 3
        {
            return;
        }
        self.last_sample = Instant::now();
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
                    if self.config.skip_fullscreen && fullscreen_active() {
                        self.policy.measurement_gap();
                    } else {
                        self.request(true);
                    }
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
        self.memory = Some(m);
        if self.history.len() == 60 {
            self.history.pop_front();
        }
        self.history.push_back(m.load_percent);
        let rect = self.rect(24, 84, 772, 188);
        if IsWindowVisible(self.hwnd) != 0 && self.settings.is_none() {
            InvalidateRect(self.hwnd, &rect, 0);
            if self.config.manual_profile == Profile::Smart {
                let reason = self.rect(44, 399, 732, 22);
                InvalidateRect(self.hwnd, &reason, 0);
            }
        }
        if self.last_load != Some(m.load_percent) {
            self.last_load = Some(m.load_percent);
            self.update_tray(NIM_MODIFY);
        }
    }

    unsafe fn request(&mut self, automatic: bool) {
        if !is_elevated() {
            self.set_status("清理尚未执行：请重新启动程序并同意管理员权限请求。");
            return;
        }
        if self.busy {
            self.set_status("清理正在进行，请稍候。");
            return;
        }
        let profile = if automatic {
            self.config.auto_profile
        } else {
            self.config.manual_profile
        };
        if profile == Profile::Aggressive && MessageBoxW(self.hwnd,
            wide("强力清理将清空工作集、刷新已修改页并清除全部待机页，可能造成磁盘写入、缺页和短暂卡顿。是否继续？").as_ptr(),
            wide("确认强力清理").as_ptr(), MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) != IDYES { return; }
        if let Some(tx) = &self.worker.tx {
            match tx.try_send(Request {
                profile,
                config: self.config.clone(),
                protected_pid: self.remembered_foreground,
            }) {
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
                self.sync_config_controls();
                self.set_status(&e);
            }
        }
    }

    unsafe fn command(&mut self, id: usize, _notification: u16) {
        match id {
            CLEAN => self.request(false),
            MINIMIZE => {
                ShowWindow(self.hwnd, SW_MINIMIZE);
            }
            HIDE => self.hide_or_exit(),
            AUTO => {
                let mut config = self.config.clone();
                config.auto_enabled = !config.auto_enabled;
                self.save_config(config);
                self.sync_config_controls();
                self.policy.reconfigure(self.clock.elapsed().as_secs());
            }
            AUTO_PROFILE => {
                const OPTIONS: [Profile; 4] = [
                    Profile::Smart,
                    Profile::Conservative,
                    Profile::Balanced,
                    Profile::Recovery,
                ];
                let mut config = self.config.clone();
                let index = OPTIONS
                    .iter()
                    .position(|p| *p == config.auto_profile)
                    .unwrap_or(3);
                config.auto_profile = OPTIONS[(index + 1) % OPTIONS.len()];
                self.save_config(config);
                self.sync_config_controls();
            }
            id if (PROFILE_BASE..PROFILE_BASE + Profile::ALL.len()).contains(&id) => {
                let mut config = self.config.clone();
                config.manual_profile = Profile::ALL[id - PROFILE_BASE];
                if config.manual_profile == Profile::Smart {
                    config.auto_profile = Profile::Smart;
                }
                self.save_config(config);
                self.sync_config_controls();
            }
            OPEN_CONFIG => {
                if self.busy {
                    self.set_status("请在回收完成后调整设置。");
                    return;
                }
                if self.settings.is_none() {
                    match SettingsPanel::new(self.hwnd, self.font, self.scale, &self.config) {
                        Ok(panel) => {
                            for &control in &self.overview_controls {
                                ShowWindow(control, SW_HIDE);
                            }
                            self.settings = Some(panel);
                            InvalidateRect(self.hwnd, null(), 0);
                        }
                        Err(error) => self.set_status(&error),
                    }
                }
            }
            settings::SAVE | 1 => {
                if let Some(panel) = &self.settings {
                    match panel.read().and_then(|config| {
                        config.save(&self.path)?;
                        Ok(config)
                    }) {
                        Ok(config) => {
                            self.config = config;
                            self.policy.reconfigure(self.clock.elapsed().as_secs());
                            self.close_settings();
                            self.update_tray(NIM_MODIFY);
                            self.set_status("设置已保存。");
                        }
                        Err(error) => panel.error(&error),
                    }
                }
            }
            settings::CANCEL | 2 if self.settings.is_some() => self.close_settings(),
            RELOAD => {
                if let Ok(memory) = snapshot() {
                    self.render_memory(memory);
                }
            }
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
        if let Ok(icon) = number_icon(
            self.last_load,
            GetSystemMetrics(SM_CXSMICON),
            self.config.threshold_percent,
        ) {
            self.tray_icon = Some(icon);
        }
        data.hIcon = self
            .tray_icon
            .as_ref()
            .map(|i| i.0)
            .unwrap_or_else(|| LoadIconW(GetModuleHandleW(null()), 101usize as *const u16));
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
            for object in [
                self.font,
                self.heading_font,
                self.number_font,
                self.small_font,
                self.background,
                self.surface,
            ] {
                if !object.is_null() {
                    DeleteObject(object);
                }
            }
        }
    }
}

fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16)
}
unsafe fn rounded(dc: HDC, rect: RECT, radius: i32, fill: COLORREF, border: COLORREF) {
    let brush = CreateSolidBrush(fill);
    let pen = CreatePen(PS_SOLID, 1, border);
    let old_brush = SelectObject(dc, brush);
    let old_pen = SelectObject(dc, pen);
    RoundRect(
        dc,
        rect.left,
        rect.top,
        rect.right,
        rect.bottom,
        radius * 2,
        radius * 2,
    );
    SelectObject(dc, old_pen);
    SelectObject(dc, old_brush);
    DeleteObject(pen);
    DeleteObject(brush);
}
unsafe fn line(dc: HDC, x1: i32, y1: i32, x2: i32, y2: i32, color: COLORREF, width: i32) {
    let pen = CreatePen(PS_SOLID, width, color);
    let old = SelectObject(dc, pen);
    MoveToEx(dc, x1, y1, null_mut());
    LineTo(dc, x2, y2);
    SelectObject(dc, old);
    DeleteObject(pen);
}

//! Small native settings dialog; Cancel leaves the current configuration intact.
use crate::{config::Config, platform::wide};
use std::{
    mem::zeroed,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::*,
        Input::KeyboardAndMouse::{EnableWindow, SetFocus},
        WindowsAndMessaging::*,
    },
};

struct Dialog {
    original: Config,
    result: Option<Config>,
    edits: Vec<HWND>,
    fullscreen: HWND,
    errors: HWND,
}

pub unsafe fn edit_settings(owner: HWND, font: HFONT, config: &Config) -> Option<Config> {
    let class_name = wide("MemReductRs.Settings.v2");
    let class = WNDCLASSW {
        lpfnWndProc: Some(dialog_proc),
        hInstance: GetModuleHandleW(null()),
        lpszClassName: class_name.as_ptr(),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
        ..zeroed()
    };
    RegisterClassW(&class); // Already registered is harmless on subsequent opens.
    let dc = GetDC(owner);
    let dpi = if dc.is_null() {
        96
    } else {
        let dpi = GetDeviceCaps(dc, LOGPIXELSX as i32);
        ReleaseDC(owner, dc);
        dpi
    };
    let scale = (dpi as f64 / 96.0)
        .min((GetSystemMetrics(SM_CYSCREEN) - 90) as f64 / 550.0)
        .max(0.75);
    let sc = |n: i32| (n as f64 * scale).round() as i32;
    let mut owner_rect: RECT = zeroed();
    GetWindowRect(owner, &mut owner_rect);
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: sc(600),
        bottom: sc(540),
    };
    let style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
    AdjustWindowRectEx(&mut rect, style, 0, WS_EX_DLGMODALFRAME);
    let hwnd = CreateWindowExW(
        WS_EX_DLGMODALFRAME,
        class_name.as_ptr(),
        wide("优化设置 · MemReduct").as_ptr(),
        style,
        (owner_rect.left + owner_rect.right - (rect.right - rect.left)) / 2,
        (owner_rect.top + owner_rect.bottom - (rect.bottom - rect.top)) / 2,
        rect.right - rect.left,
        rect.bottom - rect.top,
        owner,
        null_mut(),
        GetModuleHandleW(null()),
        null(),
    );
    if hwnd.is_null() {
        return None;
    }
    let child = |class: &str, text: &str, style: u32, x: i32, y: i32, w: i32, h: i32, id: usize| {
        let control = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            sc(x),
            sc(y),
            sc(w),
            sc(h),
            hwnd,
            id as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        SendMessageW(control, WM_SETFONT, font as usize, 1);
        control
    };
    let mut dialog = Dialog {
        original: config.clone(),
        result: None,
        edits: Vec::new(),
        fullscreen: null_mut(),
        errors: null_mut(),
    };
    let fields = [
        (
            "触发占用阈值（50–99%）",
            config.threshold_percent.to_string(),
        ),
        (
            "持续高占用（3–300 秒）",
            config.sustained_seconds.to_string(),
        ),
        (
            "清理冷却（60–86400 秒）",
            config.cooldown_seconds.to_string(),
        ),
        ("回落幅度（1–30%）", config.hysteresis_percent.to_string()),
        (
            "后台进程最小工作集（MiB）",
            config.minimum_working_set_mib.to_string(),
        ),
        (
            "定时间隔（秒，0 为关闭）",
            config.interval_seconds.to_string(),
        ),
    ];
    for (i, (label, value)) in fields.iter().enumerate() {
        let x = 24 + (i % 2) as i32 * 282;
        let y = 20 + (i / 2) as i32 * 78;
        child("STATIC", label, 0, x, y, 266, 24, 0);
        dialog.edits.push(child(
            "EDIT",
            value,
            WS_TABSTOP | WS_BORDER | ES_NUMBER as u32 | ES_AUTOHSCROLL as u32,
            x,
            y + 28,
            266,
            28,
            200 + i,
        ));
    }
    child(
        "STATIC",
        "排除进程（每行一个文件名，例如 game.exe）",
        0,
        24,
        262,
        550,
        24,
        0,
    );
    let exclusions = child(
        "EDIT",
        &config.process_exclusions.join("\r\n"),
        WS_TABSTOP | WS_BORDER | WS_VSCROLL | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32,
        24,
        292,
        550,
        80,
        210,
    );
    SendMessageW(exclusions, EM_SETLIMITTEXT, 32768, 0);
    dialog.edits.push(exclusions);
    dialog.fullscreen = child(
        "BUTTON",
        "全屏游戏或演示期间暂停自动优化",
        WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        24,
        388,
        550,
        26,
        211,
    );
    SendMessageW(
        dialog.fullscreen,
        BM_SETCHECK,
        if config.skip_fullscreen {
            BST_CHECKED
        } else {
            BST_UNCHECKED
        } as usize,
        0,
    );
    dialog.errors = child(
        "STATIC",
        "前台应用及其进程树会自动排除；繁忙进程也会跳过。",
        0,
        24,
        426,
        550,
        42,
        0,
    );
    child(
        "BUTTON",
        "取消",
        WS_TABSTOP | BS_PUSHBUTTON as u32,
        322,
        488,
        116,
        32,
        IDCANCEL as usize,
    );
    child(
        "BUTTON",
        "保存设置",
        WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
        450,
        488,
        124,
        32,
        IDOK as usize,
    );
    if dialog.edits.iter().any(|p| p.is_null())
        || dialog.fullscreen.is_null()
        || dialog.errors.is_null()
    {
        DestroyWindow(hwnd);
        return None;
    }
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&mut dialog as *mut Dialog) as isize);
    EnableWindow(owner, 0);
    ShowWindow(hwnd, SW_SHOW);
    SetFocus(dialog.edits[0]);
    let mut msg: MSG = zeroed();
    while IsWindow(hwnd) != 0 {
        let code = GetMessageW(&mut msg, null_mut(), 0, 0);
        if code <= 0 {
            if code == 0 {
                PostQuitMessage(msg.wParam as i32);
            }
            break;
        }
        if IsDialogMessageW(hwnd, &msg) == 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    if IsWindow(hwnd) != 0 {
        DestroyWindow(hwnd);
    }
    EnableWindow(owner, 1);
    SetForegroundWindow(owner);
    dialog.result
}

unsafe extern "system" fn dialog_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_COMMAND {
        match (wp & 0xffff) as i32 {
            IDCANCEL => {
                DestroyWindow(hwnd);
                return 0;
            }
            IDOK => {
                let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Dialog;
                if ptr.is_null() {
                    return 0;
                }
                let dialog = &mut *ptr;
                match read_config(dialog) {
                    Ok(config) => {
                        dialog.result = Some(config);
                        DestroyWindow(hwnd);
                    }
                    Err(error) => {
                        SetWindowTextW(dialog.errors, wide(&error).as_ptr());
                    }
                }
                return 0;
            }
            _ => {}
        }
    }
    if msg == WM_CLOSE {
        DestroyWindow(hwnd);
        return 0;
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

unsafe fn read_config(dialog: &Dialog) -> Result<Config, String> {
    let text = |hwnd: HWND| {
        let length = GetWindowTextLengthW(hwnd) as usize;
        let mut value = vec![0; length + 1];
        GetWindowTextW(hwnd, value.as_mut_ptr(), value.len() as i32);
        String::from_utf16_lossy(&value[..length])
    };
    let values: Vec<u64> = dialog.edits[..6]
        .iter()
        .map(|&e| {
            text(e)
                .parse::<u64>()
                .map_err(|_| "请输入有效整数".to_string())
        })
        .collect::<Result<_, _>>()?;
    let mut config = dialog.original.clone();
    config.threshold_percent = u32::try_from(values[0]).map_err(|_| "阈值过大")?;
    config.sustained_seconds = values[1];
    config.cooldown_seconds = values[2];
    config.hysteresis_percent = u32::try_from(values[3]).map_err(|_| "回落幅度过大")?;
    config.minimum_working_set_mib = u32::try_from(values[4]).map_err(|_| "工作集阈值过大")?;
    config.interval_seconds = values[5];
    config.process_exclusions = text(dialog.edits[6])
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    config.skip_fullscreen =
        SendMessageW(dialog.fullscreen, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
    config.validate()?;
    Ok(config)
}

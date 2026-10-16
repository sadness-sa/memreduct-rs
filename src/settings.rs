//! Settings live in the main surface; no secondary framed dialog.
use crate::{
    config::Config,
    platform::{os_error, wide},
    AppResult,
};
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Controls::*, Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
};

pub const SAVE: usize = 310;
pub const CANCEL: usize = 311;

pub struct SettingsPanel {
    original: Config,
    controls: Vec<HWND>,
    edits: Vec<HWND>,
    fullscreen: HWND,
    errors: HWND,
}
impl SettingsPanel {
    pub unsafe fn new(owner: HWND, font: HFONT, scale: f64, config: &Config) -> AppResult<Self> {
        let mut panel = Self {
            original: config.clone(),
            controls: Vec::new(),
            edits: Vec::new(),
            fullscreen: null_mut(),
            errors: null_mut(),
        };
        let sc = |n: i32| (n as f64 * scale).round() as i32;
        let mut child = |class: &str,
                         text: &str,
                         style: u32,
                         x: i32,
                         y: i32,
                         w: i32,
                         h: i32,
                         id: usize|
         -> AppResult<HWND> {
            let control = CreateWindowExW(
                0,
                wide(class).as_ptr(),
                wide(text).as_ptr(),
                WS_CHILD | WS_VISIBLE | style,
                sc(x),
                sc(y),
                sc(w),
                sc(h),
                owner,
                id as HMENU,
                GetModuleHandleW(null()),
                null(),
            );
            if control.is_null() {
                return Err(os_error("创建设置控件"));
            }
            panel.controls.push(control);
            SendMessageW(control, WM_SETFONT, font as usize, 1);
            Ok(control)
        };
        let fields = [
            ("触发阈值（50–99%）", config.threshold_percent.to_string()),
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
            let x = 44 + (i % 2) as i32 * 374;
            let y = 138 + (i / 2) as i32 * 76;
            child("STATIC", label, 0, x, y, 348, 24, 0)?;
            panel.edits.push(child(
                "EDIT",
                value,
                WS_TABSTOP | WS_BORDER | ES_NUMBER as u32 | ES_AUTOHSCROLL as u32,
                x,
                y + 29,
                348,
                28,
                200 + i,
            )?);
        }
        child(
            "STATIC",
            "进程排除列表 · 每行一个文件名，例如 game.exe",
            0,
            44,
            376,
            732,
            24,
            0,
        )?;
        panel.edits.push(child(
            "EDIT",
            &config.process_exclusions.join("\r\n"),
            WS_TABSTOP | WS_BORDER | WS_VSCROLL | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32,
            44,
            410,
            732,
            112,
            210,
        )?);
        SendMessageW(*panel.edits.last().unwrap(), EM_SETLIMITTEXT, 32768, 0);
        panel.fullscreen = child(
            "BUTTON",
            "全屏游戏或演示期间暂停自动优化",
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            44,
            542,
            732,
            28,
            211,
        )?;
        SendMessageW(
            panel.fullscreen,
            BM_SETCHECK,
            if config.skip_fullscreen {
                BST_CHECKED
            } else {
                BST_UNCHECKED
            } as usize,
            0,
        );
        panel.errors = child(
            "STATIC",
            "前台进程树与繁忙进程会自动排除。智能模式不会选择全局深度清理。",
            0,
            44,
            586,
            732,
            48,
            220,
        )?;
        child(
            "BUTTON",
            "返回总览",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            44,
            656,
            150,
            36,
            CANCEL,
        )?;
        child(
            "BUTTON",
            "保存设置",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            586,
            656,
            190,
            36,
            SAVE,
        )?;
        SetFocus(panel.edits[0]);
        Ok(panel)
    }
    pub unsafe fn read(&self) -> Result<Config, String> {
        let text = |hwnd: HWND| {
            let length = GetWindowTextLengthW(hwnd) as usize;
            let mut value = vec![0; length + 1];
            GetWindowTextW(hwnd, value.as_mut_ptr(), value.len() as i32);
            String::from_utf16_lossy(&value[..length])
        };
        let values: Vec<u64> = self.edits[..6]
            .iter()
            .map(|&e| {
                text(e)
                    .parse::<u64>()
                    .map_err(|_| "请输入有效整数".to_string())
            })
            .collect::<Result<_, _>>()?;
        let mut config = self.original.clone();
        config.threshold_percent = u32::try_from(values[0]).map_err(|_| "阈值过大")?;
        config.sustained_seconds = values[1];
        config.cooldown_seconds = values[2];
        config.hysteresis_percent = u32::try_from(values[3]).map_err(|_| "回落幅度过大")?;
        config.minimum_working_set_mib = u32::try_from(values[4]).map_err(|_| "工作集阈值过大")?;
        config.interval_seconds = values[5];
        config.process_exclusions = text(self.edits[6])
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        config.skip_fullscreen =
            SendMessageW(self.fullscreen, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
        config.validate()?;
        Ok(config)
    }
    pub unsafe fn error(&self, text: &str) {
        SetWindowTextW(self.errors, wide(text).as_ptr());
    }
}
impl Drop for SettingsPanel {
    fn drop(&mut self) {
        unsafe {
            for &hwnd in &self.controls {
                if IsWindow(hwnd) != 0 {
                    DestroyWindow(hwnd);
                }
            }
        }
    }
}

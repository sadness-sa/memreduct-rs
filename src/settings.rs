//! Unified settings with separate automation, tray appearance and process-protection pages.
use crate::{
    config::{color_hex, parse_color, AutoTrigger, Config},
    platform::{os_error, wide},
    AppResult,
};
use std::{
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Controls::{Dialogs::*, *},
        Input::KeyboardAndMouse::{EnableWindow, SetFocus},
        WindowsAndMessaging::*,
    },
};

pub const SAVE: usize = 310;
pub const CANCEL: usize = 311;
pub const TAB_BASE: usize = 400;
pub const TRIGGER_BASE: usize = 340;
pub const COLOR_BASE: usize = 350;

pub struct SettingsPanel {
    original: Config,
    owner: HWND,
    font: HFONT,
    scale: f64,
    controls: Vec<HWND>,
    groups: [Vec<HWND>; 3],
    tab_buttons: Vec<HWND>,
    trigger_buttons: Vec<HWND>,
    edits: Vec<HWND>,
    fullscreen: HWND,
    enabled: HWND,
    dynamic: HWND,
    warning: HWND,
    danger: HWND,
    colors: Vec<HWND>,
    color_buttons: Vec<HWND>,
    custom_colors: [u32; 16],
    errors: HWND,
    tab: usize,
    trigger: AutoTrigger,
}
impl SettingsPanel {
    pub unsafe fn new(owner: HWND, font: HFONT, scale: f64, config: &Config) -> AppResult<Self> {
        let mut p = Self {
            original: config.clone(),
            owner,
            font,
            scale,
            controls: Vec::new(),
            groups: [Vec::new(), Vec::new(), Vec::new()],
            tab_buttons: Vec::new(),
            trigger_buttons: Vec::new(),
            edits: vec![null_mut(); 7],
            fullscreen: null_mut(),
            enabled: null_mut(),
            dynamic: null_mut(),
            warning: null_mut(),
            danger: null_mut(),
            colors: Vec::new(),
            color_buttons: Vec::new(),
            custom_colors: [0xffffff; 16],
            errors: null_mut(),
            tab: 0,
            trigger: config.auto_trigger,
        };
        for (i, label) in ["自动清理", "托盘外观", "进程保护"].iter().enumerate() {
            let hwnd = p.child(
                3,
                "BUTTON",
                label,
                WS_TABSTOP | BS_OWNERDRAW as u32,
                44 + i as i32 * 250,
                138,
                232,
                30,
                TAB_BASE + i,
            )?;
            p.tab_buttons.push(hwnd);
        }
        p.enabled = p.child(
            0,
            "BUTTON",
            "启用自动清理",
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            44,
            188,
            348,
            28,
            330,
        )?;
        SendMessageW(p.enabled, BM_SETCHECK, usize::from(config.auto_enabled), 0);
        p.child(0, "STATIC", "触发方式", 0, 44, 224, 348, 22, 0)?;
        for (i, trigger) in AutoTrigger::ALL.iter().enumerate() {
            let hwnd = p.child(
                0,
                "BUTTON",
                trigger.label(),
                WS_TABSTOP | BS_OWNERDRAW as u32,
                44 + i as i32 * 250,
                250,
                232,
                30,
                TRIGGER_BASE + i,
            )?;
            p.trigger_buttons.push(hwnd);
        }
        for (index, label, value, x, y) in [
            (
                0,
                "内存占用阈值（50–99%）",
                config.threshold_percent.to_string(),
                44,
                302,
            ),
            (
                1,
                "持续高占用（3–300 秒）",
                config.sustained_seconds.to_string(),
                418,
                302,
            ),
            (
                5,
                "定时间隔（秒，至少 60）",
                config.interval_seconds.to_string(),
                44,
                386,
            ),
            (
                2,
                "清理冷却（60–86400 秒）",
                config.cooldown_seconds.to_string(),
                418,
                386,
            ),
            (
                3,
                "回落幅度（1–30%）",
                config.hysteresis_percent.to_string(),
                44,
                470,
            ),
        ] {
            p.child(0, "STATIC", label, 0, x, y, 348, 24, 0)?;
            p.edits[index] = p.child(
                0,
                "EDIT",
                &value,
                WS_TABSTOP | WS_BORDER | ES_NUMBER as u32 | ES_AUTOHSCROLL as u32,
                x,
                y + 29,
                348,
                28,
                200 + index,
            )?;
        }
        p.fullscreen = p.child(
            0,
            "BUTTON",
            "全屏游戏或演示时暂停",
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            418,
            496,
            348,
            28,
            211,
        )?;
        SendMessageW(
            p.fullscreen,
            BM_SETCHECK,
            usize::from(config.skip_fullscreen),
            0,
        );
        p.dynamic = p.child(
            1,
            "BUTTON",
            "根据内存占用自动改变背景色",
            WS_TABSTOP | BS_AUTOCHECKBOX as u32,
            44,
            188,
            732,
            28,
            331,
        )?;
        SendMessageW(
            p.dynamic,
            BM_SETCHECK,
            usize::from(config.tray.dynamic_background),
            0,
        );
        p.child(1, "STATIC", "警告阈值（%）", 0, 44, 226, 348, 24, 0)?;
        p.warning = p.child(
            1,
            "EDIT",
            &config.tray.warning_percent.to_string(),
            WS_TABSTOP | WS_BORDER | ES_NUMBER as u32,
            44,
            255,
            348,
            28,
            230,
        )?;
        p.child(1, "STATIC", "危险阈值（%）", 0, 418, 226, 348, 24, 0)?;
        p.danger = p.child(
            1,
            "EDIT",
            &config.tray.danger_percent.to_string(),
            WS_TABSTOP | WS_BORDER | ES_NUMBER as u32,
            418,
            255,
            348,
            28,
            231,
        )?;
        for (i, (label, color)) in [
            ("常规背景", &config.tray.normal_background),
            ("警告背景", &config.tray.warning_background),
            ("危险背景", &config.tray.danger_background),
            ("数字文字颜色", &config.tray.text_color),
        ]
        .iter()
        .enumerate()
        {
            let y = 322 + i as i32 * 60;
            p.child(1, "STATIC", label, 0, 44, y, 180, 28, 0)?;
            let edit = p.child(
                1,
                "EDIT",
                color,
                WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
                238,
                y,
                310,
                28,
                240 + i,
            )?;
            SendMessageW(edit, EM_SETLIMITTEXT, 7, 0);
            p.colors.push(edit);
            let button = p.child(
                1,
                "BUTTON",
                "点击选色",
                WS_TABSTOP | BS_OWNERDRAW as u32,
                574,
                y,
                202,
                30,
                COLOR_BASE + i,
            )?;
            p.color_buttons.push(button);
        }
        p.child(
            2,
            "STATIC",
            "后台进程最小工作集（16–4096 MiB）",
            0,
            44,
            196,
            732,
            24,
            0,
        )?;
        p.edits[4] = p.child(
            2,
            "EDIT",
            &config.minimum_working_set_mib.to_string(),
            WS_TABSTOP | WS_BORDER | ES_NUMBER as u32,
            44,
            228,
            348,
            28,
            204,
        )?;
        p.child(
            2,
            "STATIC",
            "排除进程 · 每行一个文件名，例如 game.exe",
            0,
            44,
            282,
            732,
            24,
            0,
        )?;
        p.edits[6] = p.child(
            2,
            "EDIT",
            &config.process_exclusions.join("\r\n"),
            WS_TABSTOP | WS_BORDER | WS_VSCROLL | ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32,
            44,
            314,
            732,
            220,
            210,
        )?;
        SendMessageW(p.edits[6], EM_SETLIMITTEXT, 32768, 0);
        p.errors = p.child(3, "STATIC", "", 0, 44, 568, 732, 64, 220)?;
        p.child(
            3,
            "BUTTON",
            "返回总览",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            44,
            656,
            150,
            36,
            CANCEL,
        )?;
        p.child(
            3,
            "BUTTON",
            "保存设置",
            WS_TABSTOP | BS_OWNERDRAW as u32,
            586,
            656,
            190,
            36,
            SAVE,
        )?;
        p.show_tab(0);
        Ok(p)
    }
    #[allow(clippy::too_many_arguments)]
    unsafe fn child(
        &mut self,
        group: usize,
        class: &str,
        text: &str,
        style: u32,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        id: usize,
    ) -> AppResult<HWND> {
        let sc = |n: i32| (n as f64 * self.scale).round() as i32;
        let hwnd = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(text).as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            sc(x),
            sc(y),
            sc(w),
            sc(h),
            self.owner,
            id as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        if hwnd.is_null() {
            return Err(os_error("创建设置控件"));
        }
        self.controls.push(hwnd);
        if group < 3 {
            self.groups[group].push(hwnd);
        }
        SendMessageW(hwnd, WM_SETFONT, self.font as usize, 1);
        Ok(hwnd)
    }
    pub fn is_selected(&self, id: usize) -> bool {
        id == TAB_BASE + self.tab
            || AutoTrigger::ALL
                .iter()
                .enumerate()
                .any(|(i, t)| id == TRIGGER_BASE + i && *t == self.trigger)
    }
    pub unsafe fn preview_color(&self, id: usize) -> Option<u32> {
        if !(COLOR_BASE..COLOR_BASE + self.colors.len()).contains(&id) {
            return None;
        }
        parse_color(&text(self.colors[id - COLOR_BASE])).ok()
    }
    unsafe fn show_tab(&mut self, tab: usize) {
        self.tab = tab;
        for (i, group) in self.groups.iter().enumerate() {
            for &hwnd in group {
                ShowWindow(hwnd, if i == tab { SW_SHOW } else { SW_HIDE });
            }
        }
        self.refresh_trigger();
        for &button in &self.tab_buttons {
            InvalidateRect(button, null(), 0);
        }
        self.error(match tab {
            0 => "按时间需非零间隔；任一条件中 0 表示仅按占用。触发受冷却与全屏暂停约束，智能策略可能跳过无需回收的时段。",
            1 => "阈值独立于清理触发阈值。关闭自动变色后始终使用常规背景；输入 #RRGGBB 或点击右侧选色。",
            _ => "前台进程树、繁忙进程与系统关键进程会自动排除。深度清理不使用后台进程排除规则。",
        });
        SetFocus(match tab {
            0 => self.enabled,
            1 => self.dynamic,
            _ => self.edits[4],
        });
        InvalidateRect(self.owner, null(), 0);
    }
    unsafe fn refresh_trigger(&self) {
        let memory = self.trigger != AutoTrigger::Interval;
        for index in [0, 1, 3] {
            EnableWindow(self.edits[index], i32::from(memory));
        }
        EnableWindow(
            self.edits[5],
            i32::from(self.trigger != AutoTrigger::Memory),
        );
        for &button in &self.trigger_buttons {
            InvalidateRect(button, null(), 0);
        }
    }
    pub unsafe fn command(&mut self, id: usize, notification: u16) -> bool {
        if (TAB_BASE..TAB_BASE + 3).contains(&id) {
            self.show_tab(id - TAB_BASE);
            return true;
        }
        if (TRIGGER_BASE..TRIGGER_BASE + 3).contains(&id) {
            self.trigger = AutoTrigger::ALL[id - TRIGGER_BASE];
            if self.trigger != AutoTrigger::Memory && text(self.edits[5]).trim() == "0" {
                SetWindowTextW(self.edits[5], wide("300").as_ptr());
            }
            self.refresh_trigger();
            return true;
        }
        if (240..244).contains(&id) && notification == EN_CHANGE as u16 {
            InvalidateRect(self.color_buttons[id - 240], null(), 0);
            return true;
        }
        if (COLOR_BASE..COLOR_BASE + 4).contains(&id) {
            let index = id - COLOR_BASE;
            let mut chooser: CHOOSECOLORW = zeroed();
            chooser.lStructSize = size_of::<CHOOSECOLORW>() as u32;
            chooser.hwndOwner = self.owner;
            chooser.rgbResult = parse_color(&text(self.colors[index])).unwrap_or(0xffffff);
            chooser.lpCustColors = self.custom_colors.as_mut_ptr();
            chooser.Flags = CC_FULLOPEN | CC_RGBINIT;
            if ChooseColorW(&mut chooser) != 0 {
                SetWindowTextW(
                    self.colors[index],
                    wide(&color_hex(chooser.rgbResult)).as_ptr(),
                );
            }
            InvalidateRect(self.owner, null(), 0);
            return true;
        }
        false
    }
    pub unsafe fn read(&self) -> AppResult<Config> {
        let values: Vec<u64> = self.edits[..6]
            .iter()
            .map(|&e| {
                text(e)
                    .parse::<u64>()
                    .map_err(|_| "请输入有效整数".to_string())
            })
            .collect::<Result<_, _>>()?;
        let mut c = self.original.clone();
        c.threshold_percent = u32::try_from(values[0]).map_err(|_| "阈值过大")?;
        c.sustained_seconds = values[1];
        c.cooldown_seconds = values[2];
        c.hysteresis_percent = u32::try_from(values[3]).map_err(|_| "回落幅度过大")?;
        c.minimum_working_set_mib = u32::try_from(values[4]).map_err(|_| "工作集阈值过大")?;
        c.interval_seconds = values[5];
        c.auto_trigger = self.trigger;
        c.auto_enabled = SendMessageW(self.enabled, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
        c.skip_fullscreen =
            SendMessageW(self.fullscreen, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
        c.process_exclusions = text(self.edits[6])
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        c.tray.dynamic_background =
            SendMessageW(self.dynamic, BM_GETCHECK, 0, 0) == BST_CHECKED as isize;
        c.tray.warning_percent = text(self.warning)
            .parse()
            .map_err(|_| "托盘警告阈值须为整数")?;
        c.tray.danger_percent = text(self.danger)
            .parse()
            .map_err(|_| "托盘危险阈值须为整数")?;
        c.tray.normal_background = text(self.colors[0]).to_uppercase();
        c.tray.warning_background = text(self.colors[1]).to_uppercase();
        c.tray.danger_background = text(self.colors[2]).to_uppercase();
        c.tray.text_color = text(self.colors[3]).to_uppercase();
        c.validate()?;
        Ok(c)
    }
    pub unsafe fn error(&self, value: &str) {
        SetWindowTextW(self.errors, wide(value).as_ptr());
    }
}
unsafe fn text(hwnd: HWND) -> String {
    let length = GetWindowTextLengthW(hwnd) as usize;
    let mut value = vec![0; length + 1];
    GetWindowTextW(hwnd, value.as_mut_ptr(), value.len() as i32);
    String::from_utf16_lossy(&value[..length])
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

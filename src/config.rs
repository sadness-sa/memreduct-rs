use crate::{model::Profile, AppResult};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AutoTrigger {
    Memory,
    Interval,
    // Older configurations already used OR semantics when interval_seconds > 0.
    #[default]
    Either,
}

impl AutoTrigger {
    pub const ALL: [Self; 3] = [Self::Memory, Self::Interval, Self::Either];
    pub fn label(self) -> &'static str {
        match self {
            Self::Memory => "按内存占用",
            Self::Interval => "按时间间隔",
            Self::Either => "任一条件",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TraySettings {
    pub dynamic_background: bool,
    pub warning_percent: u32,
    pub danger_percent: u32,
    pub normal_background: String,
    pub warning_background: String,
    pub danger_background: String,
    pub text_color: String,
}
impl Default for TraySettings {
    fn default() -> Self {
        Self {
            dynamic_background: true,
            warning_percent: 85,
            danger_percent: 94,
            normal_background: "#3F69ED".into(),
            warning_background: "#C5880D".into(),
            danger_background: "#E04C34".into(),
            text_color: "#FFFFFF".into(),
        }
    }
}
impl TraySettings {
    pub fn validate(&self) -> AppResult<()> {
        if !(1..=99).contains(&self.warning_percent)
            || self.danger_percent > 100
            || self.danger_percent <= self.warning_percent
        {
            return Err("托盘阈值须满足 1 ≤ 警告 < 危险 ≤ 100".into());
        }
        for color in [
            &self.normal_background,
            &self.warning_background,
            &self.danger_background,
            &self.text_color,
        ] {
            parse_color(color)?;
        }
        Ok(())
    }
    pub fn background(&self, load: Option<u32>) -> AppResult<u32> {
        let color = if self.dynamic_background && load.is_some_and(|p| p >= self.danger_percent) {
            &self.danger_background
        } else if self.dynamic_background && load.is_some_and(|p| p >= self.warning_percent) {
            &self.warning_background
        } else {
            &self.normal_background
        };
        parse_color(color)
    }
}

/// Parse #RRGGBB into the COLORREF byte order required by Win32, on any host.
pub fn parse_color(color: &str) -> AppResult<u32> {
    let value = color
        .strip_prefix('#')
        .filter(|v| v.len() == 6 && v.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or_else(|| format!("颜色须为 #RRGGBB，例如 #3F69ED：{color}"))?;
    let rgb = u32::from_str_radix(value, 16).map_err(|e| e.to_string())?;
    Ok(((rgb >> 16) & 255) | (rgb & 0xff00) | ((rgb & 255) << 16))
}
pub fn color_hex(colorref: u32) -> String {
    format!(
        "#{:02X}{:02X}{:02X}",
        colorref & 255,
        (colorref >> 8) & 255,
        (colorref >> 16) & 255
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub auto_enabled: bool,
    #[serde(default)]
    pub auto_trigger: AutoTrigger,
    pub threshold_percent: u32,
    pub hysteresis_percent: u32,
    pub sustained_seconds: u64,
    pub cooldown_seconds: u64,
    pub interval_seconds: u64,
    pub manual_profile: Profile,
    pub auto_profile: Profile,
    pub skip_fullscreen: bool,
    pub minimum_working_set_mib: u32,
    pub process_exclusions: Vec<String>,
    pub start_minimized: bool,
    pub hotkey_enabled: bool,
    pub tray: TraySettings,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auto_enabled: false,
            auto_trigger: AutoTrigger::Memory,
            threshold_percent: 85,
            hysteresis_percent: 10,
            sustained_seconds: 10,
            cooldown_seconds: 300,
            interval_seconds: 0,
            manual_profile: Profile::Smart,
            auto_profile: Profile::Smart,
            skip_fullscreen: true,
            minimum_working_set_mib: 32,
            process_exclusions: vec!["obs64.exe".into(), "obs32.exe".into()],
            start_minimized: false,
            hotkey_enabled: true,
            tray: TraySettings::default(),
        }
    }
}

impl Config {
    pub fn validate(&self) -> AppResult<()> {
        self.tray.validate()?;
        if self.auto_trigger == AutoTrigger::Interval && self.interval_seconds == 0 {
            return Err("按时间清理需要设置至少 60 秒的间隔".into());
        }
        if self.auto_profile == Profile::Aggressive {
            return Err("自动清理不允许 aggressive 全局深度清理".into());
        }
        if !(16..=4096).contains(&self.minimum_working_set_mib) {
            return Err("minimum_working_set_mib 必须为 16..4096".into());
        }
        if self.process_exclusions.len() > 128
            || self
                .process_exclusions
                .iter()
                .any(|s| s.is_empty() || s.len() > 260 || s.contains(['/', '\\', '\0']))
        {
            return Err("process_exclusions 最多 128 个非空进程文件名，不支持路径".into());
        }
        if !(50..=99).contains(&self.threshold_percent) {
            return Err("threshold_percent 必须为 50..99".into());
        }
        if !(1..=30).contains(&self.hysteresis_percent) {
            return Err("hysteresis_percent 必须为 1..30".into());
        }
        if !(3..=300).contains(&self.sustained_seconds) {
            return Err("sustained_seconds 必须为 3..300".into());
        }
        if !(60..=86400).contains(&self.cooldown_seconds) {
            return Err("cooldown_seconds 必须为 60..86400".into());
        }
        if self.interval_seconds != 0 && !(60..=86400).contains(&self.interval_seconds) {
            return Err("interval_seconds 必须为 0（关闭）或 60..86400".into());
        }
        Ok(())
    }

    pub fn load_or_create(path: &Path) -> AppResult<Self> {
        match fs::read_to_string(path) {
            Ok(text) => {
                let config: Self =
                    toml::from_str(&text).map_err(|e| format!("配置解析失败：{e}"))?;
                config.validate()?;
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::default();
                config.save(path)?;
                Ok(config)
            }
            Err(e) => Err(format!("无法读取配置 {}：{e}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> AppResult<()> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temp = path.with_extension(format!(
            "{}.{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| -> std::io::Result<()> {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            fs::rename(&temp, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result.map_err(|e| format!("无法保存配置（原文件保留）：{e}"))
    }
}

pub fn config_path(portable: bool) -> AppResult<PathBuf> {
    if portable {
        return Ok(std::env::current_exe()
            .map_err(|e| e.to_string())?
            .with_file_name("memreduct-rs.toml"));
    }
    let root = std::env::var_os("LOCALAPPDATA").ok_or("未设置 LOCALAPPDATA；可使用 --portable")?;
    Ok(PathBuf::from(root).join("MemReductRs").join("config.toml"))
}

pub fn append_report(path: &Path, report: &crate::model::CleanReport) -> AppResult<()> {
    let log = path.with_extension("log");
    if fs::metadata(&log).is_ok_and(|m| m.len() > 1024 * 1024) {
        let old = log.with_extension("log.1");
        if old.exists() {
            fs::remove_file(&old).map_err(|e| e.to_string())?;
        }
        fs::rename(&log, old).map_err(|e| e.to_string())?;
    }
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|e| e.to_string())?;
    writeln!(
        file,
        "{}",
        serde_json::json!({"unix_seconds": time, "report": report})
    )
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tray_palette_thresholds_and_color_formats_are_validated() {
        let mut tray = TraySettings::default();
        assert_eq!(
            tray.background(Some(84)).unwrap(),
            parse_color("#3F69ED").unwrap()
        );
        assert_eq!(
            tray.background(Some(85)).unwrap(),
            parse_color("#C5880D").unwrap()
        );
        assert_eq!(
            tray.background(Some(94)).unwrap(),
            parse_color("#E04C34").unwrap()
        );
        tray.dynamic_background = false;
        assert_eq!(
            tray.background(Some(100)).unwrap(),
            tray.background(None).unwrap()
        );
        tray.danger_percent = tray.warning_percent;
        assert!(tray.validate().is_err());
        for invalid in ["FFFFFF", "#FFF", "#ZZ0000", "#0000000"] {
            assert!(parse_color(invalid).is_err());
        }
        assert_eq!(color_hex(parse_color("#AbCDeF").unwrap()), "#ABCDEF");
    }
    #[test]
    fn legacy_config_gets_safe_new_options_and_aggressive_auto_is_rejected() {
        let mut config: Config = toml::from_str("manual_profile = 'conservative'").unwrap();
        assert_eq!(config.manual_profile, Profile::Conservative);
        assert_eq!(config.auto_profile, Profile::Smart);
        assert!(config.skip_fullscreen);
        assert_eq!(config.auto_trigger, AutoTrigger::Either);
        let legacy: Config = toml::from_str("interval_seconds = 60").unwrap();
        assert_eq!(legacy.auto_trigger, AutoTrigger::Either);
        config.auto_profile = Profile::Aggressive;
        assert!(config.validate().is_err());
        config.auto_profile = Profile::Balanced;
        config.process_exclusions = vec!["C:\\game.exe".into()];
        assert!(config.validate().is_err());
    }
    #[test]
    fn invalid_config_is_rejected_and_preserved() {
        let dir = std::env::temp_dir().join(format!("memreduct-config-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let mut config = Config::default();
        config.save(&path).unwrap();
        config.auto_enabled = true;
        config.save(&path).unwrap();
        assert!(Config::load_or_create(&path).unwrap().auto_enabled);
        config.threshold_percent = 100;
        assert!(config.save(&path).is_err());
        assert_eq!(Config::load_or_create(&path).unwrap().threshold_percent, 85);
        fs::write(&path, "threshold_percent = 'bad'").unwrap();
        assert!(Config::load_or_create(&path).is_err());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "threshold_percent = 'bad'"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

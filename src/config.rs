use crate::{model::Profile, AppResult};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub auto_enabled: bool,
    pub threshold_percent: u32,
    pub hysteresis_percent: u32,
    pub sustained_seconds: u64,
    pub cooldown_seconds: u64,
    pub interval_seconds: u64,
    pub manual_profile: Profile,
    pub start_minimized: bool,
    pub hotkey_enabled: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            auto_enabled: false,
            threshold_percent: 85,
            hysteresis_percent: 10,
            sustained_seconds: 10,
            cooldown_seconds: 300,
            interval_seconds: 0,
            manual_profile: Profile::Conservative,
            start_minimized: false,
            hotkey_enabled: true,
        }
    }
}

impl Config {
    pub fn validate(&self) -> AppResult<()> {
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

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct MemorySnapshot {
    pub load_percent: u32,
    pub physical_total: u64,
    pub physical_available: u64,
    pub commit_total: u64,
    pub commit_limit: u64,
    pub system_cache: u64,
    pub standby: Option<u64>,
    pub free: Option<u64>,
    pub modified: Option<u64>,
    pub app_working_set: u64,
    pub app_private: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    #[default]
    Conservative,
    Balanced,
    Aggressive,
}

impl Profile {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "conservative" => Ok(Self::Conservative),
            "balanced" => Ok(Self::Balanced),
            "aggressive" => Ok(Self::Aggressive),
            _ => Err("profile must be conservative, balanced or aggressive".into()),
        }
    }

    pub fn operations(self) -> &'static [Operation] {
        use Operation::*;
        match self {
            Self::Conservative => &[LowPriorityStandby],
            Self::Balanced => &[BackgroundWorkingSets, SystemFileCache, LowPriorityStandby],
            Self::Aggressive => &[WorkingSets, SystemFileCache, ModifiedPages, Standby],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    LowPriorityStandby,
    SystemFileCache,
    WorkingSets,
    BackgroundWorkingSets,
    ModifiedPages,
    Standby,
}

impl Operation {
    pub fn label(self) -> &'static str {
        match self {
            Self::LowPriorityStandby => "低优先级待机页",
            Self::SystemFileCache => "系统文件缓存",
            Self::WorkingSets => "进程工作集",
            Self::BackgroundWorkingSets => "后台进程工作集（排除前台应用）",
            Self::ModifiedPages => "已修改页（可能产生磁盘写入）",
            Self::Standby => "全部待机页",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct OperationResult {
    pub operation: Operation,
    pub error: Option<String>,
    pub elapsed_ms: u128,
    pub processes: Option<ProcessTrimStats>,
}

#[derive(Debug, Default, Serialize)]
pub struct ProcessTrimStats {
    pub trimmed: u32,
    pub skipped: u32,
    pub inaccessible: u32,
    pub failed: u32,
    pub measured: u32,
    // A sum of per-process WS deltas; shared pages can be counted more than once.
    pub working_set_reduction: u64,
}

impl OperationResult {
    pub fn detail(&self) -> String {
        let state = self.error.as_deref().unwrap_or("完成");
        let mut text = format!(
            "{}：{}（{} ms）",
            self.operation.label(),
            state,
            self.elapsed_ms
        );
        if let Some(p) = &self.processes {
            text.push_str(&format!("；回收 {} 个，跳过 {} 个，无权访问 {} 个，失败 {} 个；工作集减少 {:.1} MiB（{} 个可测进程，共享页可重复）",
                p.trimmed, p.skipped, p.inaccessible, p.failed,
                p.working_set_reduction as f64 / 1048576.0, p.measured));
        }
        text
    }
}

#[derive(Debug, Serialize)]
pub struct CleanReport {
    pub profile: Profile,
    pub before: MemorySnapshot,
    // A failed follow-up query does not erase the already executed operation results.
    pub after: Option<MemorySnapshot>,
    pub measurement_error: Option<String>,
    pub elapsed_ms: u128,
    pub operations: Vec<OperationResult>,
}

impl CleanReport {
    pub fn available_delta(&self) -> Option<i128> {
        self.after
            .map(|a| a.physical_available as i128 - self.before.physical_available as i128)
    }
    pub fn succeeded(&self) -> bool {
        self.operations.iter().all(|r| r.error.is_none())
    }
    pub fn summary(&self) -> String {
        let success = self.operations.iter().filter(|r| r.error.is_none()).count();
        let change = self
            .available_delta()
            .map(|b| format!("{:+.1} MiB", b as f64 / 1048576.0))
            .unwrap_or_else(|| "无法测量".into());
        if success == 0 {
            return format!(
                "未完成清理（0/{} 项）；请查看下方错误。耗时 {} ms",
                self.operations.len(),
                self.elapsed_ms
            );
        }
        format!(
            "清理 {}/{} 项成功；可用内存变化 {}；耗时 {} ms",
            success,
            self.operations.len(),
            change,
            self.elapsed_ms
        )
    }

    pub fn measurement_details(&self) -> String {
        let Some(after) = self.after else {
            return "清理后采样失败，无法比较。".into();
        };
        let mut text = format!(
            "物理内存：{}% → {}%；可用：{} → {}；提交量：{} → {}。",
            self.before.load_percent,
            after.load_percent,
            gib(self.before.physical_available),
            gib(after.physical_available),
            gib(self.before.commit_total),
            gib(after.commit_total)
        );
        if let (Some(b), Some(a)) = (self.before.standby, after.standby) {
            text.push_str(&format!(" 待机缓存：{} → {}。", gib(b), gib(a)));
        }
        if self.succeeded() && self.available_delta().is_some_and(|v| v < 1048576) {
            text.push_str(
                " 可用内存未明显增加：待机页原本已可用，或没有可回收工作集；不代表修复了应用泄漏。",
            );
        }
        text
    }
}

pub fn gib(bytes: u64) -> String {
    format!("{:.2} GiB", bytes as f64 / 1073741824.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_partial_failure_and_keeps_negative_measurements() {
        let mut report = CleanReport {
            profile: Profile::Balanced,
            before: MemorySnapshot {
                physical_available: 4096,
                ..Default::default()
            },
            after: Some(MemorySnapshot {
                physical_available: 2048,
                ..Default::default()
            }),
            measurement_error: None,
            elapsed_ms: 1,
            operations: vec![
                OperationResult {
                    operation: Operation::SystemFileCache,
                    error: Some("denied".into()),
                    elapsed_ms: 0,
                    processes: None,
                },
                OperationResult {
                    operation: Operation::LowPriorityStandby,
                    error: None,
                    elapsed_ms: 0,
                    processes: None,
                },
            ],
        };
        assert_eq!(report.available_delta(), Some(-2048));
        assert!(!report.succeeded());
        assert!(report.summary().contains("1/2"));
        report.after = None;
        assert_eq!(report.available_delta(), None);
        assert!(report.summary().contains("无法测量"));
        report.operations[1].error = Some("privilege missing".into());
        let summary = report.summary();
        assert!(summary.contains("未完成清理"));
        assert!(!summary.contains("+0.0"));
    }
}

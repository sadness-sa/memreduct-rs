use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct MemorySnapshot {
    pub load_percent: u32,
    pub physical_total: u64,
    pub physical_available: u64,
    pub commit_total: u64,
    pub commit_limit: u64,
    pub system_cache: u64,
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
            Self::Balanced => &[SystemFileCache, LowPriorityStandby],
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
    ModifiedPages,
    Standby,
}

impl Operation {
    pub fn label(self) -> &'static str {
        match self {
            Self::LowPriorityStandby => "低优先级待机页",
            Self::SystemFileCache => "系统文件缓存",
            Self::WorkingSets => "进程工作集",
            Self::ModifiedPages => "已修改页（可能产生磁盘写入）",
            Self::Standby => "全部待机页",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct OperationResult {
    pub operation: Operation,
    pub error: Option<String>,
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
        format!(
            "清理 {}/{} 项成功；可用内存变化 {}；耗时 {} ms",
            success,
            self.operations.len(),
            change,
            self.elapsed_ms
        )
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
                },
                OperationResult {
                    operation: Operation::LowPriorityStandby,
                    error: None,
                },
            ],
        };
        assert_eq!(report.available_delta(), Some(-2048));
        assert!(!report.succeeded());
        assert!(report.summary().contains("1/2"));
        report.after = None;
        assert_eq!(report.available_delta(), None);
        assert!(report.summary().contains("无法测量"));
    }
}

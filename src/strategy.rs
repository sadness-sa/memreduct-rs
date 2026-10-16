//! Deterministic, explainable selection. Smart never picks global working sets or disk flushes.
use crate::{
    config::Config,
    model::{MemorySnapshot, Operation, Profile},
};

#[derive(Debug)]
pub struct Plan {
    pub selected: Profile,
    pub operations: Vec<Operation>,
    pub reason: String,
}

pub fn select(requested: Profile, memory: &MemorySnapshot, config: &Config) -> Plan {
    if requested != Profile::Smart {
        return Plan {
            selected: requested,
            operations: requested.operations().to_vec(),
            reason: format!("指定策略：{}", requested.label()),
        };
    }
    let total = memory.physical_total;
    let gib = 1_073_741_824;
    let mib = 1_048_576;
    let (selected, reason) = if total == 0 {
        (Profile::Smart, "采样不完整，跳过清理")
    } else if memory.load_percent >= 94 || memory.physical_available <= (total / 25).max(512 * mib)
    {
        (
            Profile::Recovery,
            "可用内存严重不足，回收空闲后台工作集与待机缓存",
        )
    } else if memory.load_percent >= config.threshold_percent
        || memory.physical_available <= (total / 10).max(gib)
    {
        (
            Profile::Balanced,
            "内存压力较高，组合回收后台工作集与系统缓存",
        )
    } else if memory.load_percent < 70 {
        (Profile::Smart, "当前内存充足，保留缓存与应用工作集")
    } else if memory
        .standby
        .is_some_and(|b| b >= (total / 8).max(512 * mib))
    {
        (Profile::Standby, "占用偏高且待机缓存较多，优先处理待机页")
    } else if memory.system_cache >= (total / 5).max(gib) {
        (
            Profile::Cache,
            "占用偏高且系统缓存较大，优先收缩文件缓存工作集",
        )
    } else {
        (Profile::Background, "占用偏高，优先回收空闲后台进程工作集")
    };
    let mut reason = reason.to_string();
    if memory.commit_limit > 0
        && memory.commit_total as u128 * 100 >= memory.commit_limit as u128 * 90
    {
        reason.push_str("；提交量偏高，工作集回收不能解除应用分配");
    }
    Plan {
        selected,
        operations: selected.operations().to_vec(),
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn memory(load: u32) -> MemorySnapshot {
        MemorySnapshot {
            physical_total: 16 << 30,
            physical_available: (16u64 << 30) * u64::from(100 - load) / 100,
            load_percent: load,
            standby: Some(0),
            ..Default::default()
        }
    }
    #[test]
    fn picks_distinct_plans_and_skips_healthy_memory() {
        let c = Config::default();
        assert!(select(Profile::Smart, &memory(50), &c)
            .operations
            .is_empty());
        assert_eq!(
            select(Profile::Smart, &memory(75), &c).selected,
            Profile::Background
        );
        let mut m = memory(75);
        m.standby = Some(3 << 30);
        assert_eq!(select(Profile::Smart, &m, &c).selected, Profile::Standby);
        m.standby = None;
        m.system_cache = 4 << 30;
        assert_eq!(select(Profile::Smart, &m, &c).selected, Profile::Cache);
        assert_eq!(
            select(Profile::Smart, &memory(88), &c).selected,
            Profile::Balanced
        );
        assert_eq!(
            select(Profile::Smart, &memory(95), &c).selected,
            Profile::Recovery
        );
        assert!(select(Profile::Smart, &MemorySnapshot::default(), &c)
            .operations
            .is_empty());
    }
    #[test]
    fn smart_never_chooses_global_trim_or_modified_page_flush() {
        let c = Config::default();
        for load in 0..=100 {
            for standby in [None, Some(0), Some(8 << 30)] {
                let mut m = memory(load);
                m.standby = standby;
                let plan = select(Profile::Smart, &m, &c);
                assert!(!plan.operations.contains(&Operation::WorkingSets));
                assert!(!plan.operations.contains(&Operation::ModifiedPages));
            }
        }
        assert_eq!(
            select(Profile::Aggressive, &memory(20), &c).operations,
            Profile::Aggressive.operations()
        );
    }
    #[test]
    fn low_available_memory_wins_and_commit_pressure_is_explained() {
        let mut m = memory(50);
        m.physical_available = 256 << 20;
        m.commit_total = u64::MAX;
        m.commit_limit = u64::MAX;
        let p = select(Profile::Smart, &m, &Config::default());
        assert_eq!(p.selected, Profile::Recovery);
        assert!(p.reason.contains("提交量"));
    }
}

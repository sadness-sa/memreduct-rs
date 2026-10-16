//! Pure, deterministic policy; caller supplies monotonic seconds, never wall-clock time.
use crate::config::{AutoTrigger, Config};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Pressure,
    Interval,
}

#[derive(Debug)]
pub struct AutoPolicy {
    high_since: Option<u64>,
    last_attempt: Option<u64>,
    interval_anchor: u64,
    armed: bool,
}

impl AutoPolicy {
    pub fn new(now: u64) -> Self {
        Self {
            high_since: None,
            last_attempt: None,
            interval_anchor: now,
            armed: true,
        }
    }
    pub fn observe(&mut self, config: &Config, now: u64, load: u32, busy: bool) -> Option<Trigger> {
        if !config.auto_enabled {
            self.high_since = None;
            self.interval_anchor = now;
            self.armed = true;
            return None;
        }
        if config.auto_trigger == AutoTrigger::Interval {
            self.high_since = None;
        } else if load
            <= config
                .threshold_percent
                .saturating_sub(config.hysteresis_percent)
        {
            self.armed = true;
        }
        if config.auto_trigger == AutoTrigger::Interval || load < config.threshold_percent {
            self.high_since = None;
        } else if self.high_since.is_none() {
            self.high_since = Some(now);
        }
        if busy
            || self
                .last_attempt
                .is_some_and(|t| now.saturating_sub(t) < config.cooldown_seconds)
        {
            return None;
        }
        if self.armed
            && self
                .high_since
                .is_some_and(|t| now.saturating_sub(t) >= config.sustained_seconds)
        {
            return Some(Trigger::Pressure);
        }
        if config.auto_trigger != AutoTrigger::Memory
            && config.interval_seconds > 0
            && now.saturating_sub(self.interval_anchor) >= config.interval_seconds
        {
            return Some(Trigger::Interval);
        }
        None
    }
    // Record every accepted attempt, including manual and unsuccessful ones, to prevent retry storms.
    pub fn attempted(&mut self, now: u64) {
        self.last_attempt = Some(now);
        self.interval_anchor = now;
        self.high_since = None;
        self.armed = false;
    }
    pub fn measurement_gap(&mut self) {
        self.high_since = None;
    }
    pub fn reconfigure(&mut self, now: u64) {
        self.high_since = None;
        self.interval_anchor = now;
        self.armed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn config() -> Config {
        Config {
            auto_enabled: true,
            ..Config::default()
        }
    }
    #[test]
    fn pressure_requires_sustained_load_cooldown_and_recovery() {
        let c = config();
        let mut p = AutoPolicy::new(0);
        assert_eq!(p.observe(&c, 0, 90, false), None);
        assert_eq!(p.observe(&c, 9, 90, false), None);
        assert_eq!(p.observe(&c, 10, 90, false), Some(Trigger::Pressure));
        p.attempted(10);
        assert_eq!(p.observe(&c, 400, 90, false), None);
        p.observe(&c, 401, 75, false);
        assert_eq!(p.observe(&c, 402, 90, false), None);
        assert_eq!(p.observe(&c, 412, 90, true), None);
        assert_eq!(p.observe(&c, 413, 90, false), Some(Trigger::Pressure));
    }
    #[test]
    fn spikes_disabled_and_failures_do_not_loop() {
        let mut c = config();
        let mut p = AutoPolicy::new(0);
        p.observe(&c, 0, 99, false);
        p.observe(&c, 5, 80, false);
        assert_eq!(p.observe(&c, 10, 99, false), None);
        c.auto_enabled = false;
        assert_eq!(p.observe(&c, 100, 99, false), None);
        c.auto_enabled = true;
        c.auto_trigger = AutoTrigger::Either;
        c.interval_seconds = 60;
        assert_eq!(p.observe(&c, 159, 20, false), None);
        assert_eq!(p.observe(&c, 160, 20, false), Some(Trigger::Interval));
        p.attempted(160);
        assert_eq!(p.observe(&c, 220, 20, false), None);
        assert_eq!(p.observe(&c, 460, 20, false), Some(Trigger::Interval));
    }
    #[test]
    fn memory_only_ignores_timer_and_interval_only_ignores_pressure() {
        let mut c = config();
        c.interval_seconds = 60;
        c.auto_trigger = AutoTrigger::Memory;
        let mut policy = AutoPolicy::new(0);
        assert_eq!(policy.observe(&c, 300, 20, false), None);
        c.auto_trigger = AutoTrigger::Interval;
        let mut policy = AutoPolicy::new(0);
        policy.observe(&c, 0, 99, false);
        assert_eq!(policy.observe(&c, 10, 99, false), None);
        assert_eq!(policy.observe(&c, 60, 99, false), Some(Trigger::Interval));
        c.auto_trigger = AutoTrigger::Either;
        let mut policy = AutoPolicy::new(0);
        policy.observe(&c, 0, 99, false);
        assert_eq!(policy.observe(&c, 10, 99, false), Some(Trigger::Pressure));
    }
    #[test]
    fn manual_attempt_delays_auto_and_recovery_does_not_bypass_cooldown() {
        let c = config();
        let mut p = AutoPolicy::new(0);
        p.attempted(1);
        p.observe(&c, 2, 70, false);
        p.observe(&c, 3, 90, false);
        assert_eq!(p.observe(&c, 300, 90, false), None);
        assert_eq!(p.observe(&c, 301, 90, false), Some(Trigger::Pressure));
    }
    #[test]
    fn missing_measurement_breaks_sustained_window_without_erasing_cooldown() {
        let c = config();
        let mut p = AutoPolicy::new(0);
        p.observe(&c, 0, 90, false);
        p.measurement_gap();
        assert_eq!(p.observe(&c, 20, 90, false), None);
        assert_eq!(p.observe(&c, 29, 90, false), None);
        assert_eq!(p.observe(&c, 30, 90, false), Some(Trigger::Pressure));
        p.attempted(30);
        p.reconfigure(31);
        p.observe(&c, 32, 90, false);
        assert_eq!(p.observe(&c, 42, 90, false), None);
        assert_eq!(p.observe(&c, 330, 90, false), Some(Trigger::Pressure));
    }
}

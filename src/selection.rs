//! Pure process selection shared by the Windows cleaner and deterministic tests.
use std::collections::HashSet;

#[derive(Clone, Debug)]
pub struct ProcessEntry {
    pub pid: u32,
    pub parent: u32,
    pub name: String,
}

pub fn protected_family(entries: &[ProcessEntry], foreground: u32) -> HashSet<u32> {
    let mut root = foreground;
    let mut seen = HashSet::new();
    while seen.insert(root) {
        let Some(entry) = entries.iter().find(|e| e.pid == root) else {
            break;
        };
        let Some(parent) = entries.iter().find(|e| e.pid == entry.parent) else {
            break;
        };
        if parent.pid == 0
            || matches!(
                parent.name.to_ascii_lowercase().as_str(),
                "explorer.exe" | "wininit.exe" | "services.exe" | "svchost.exe"
            )
        {
            break;
        }
        root = parent.pid;
    }
    let foreground_name = entries
        .iter()
        .find(|e| e.pid == foreground)
        .map(|e| e.name.to_ascii_lowercase());
    let mut protected = HashSet::from([foreground, root]);
    if let Some(name) = foreground_name {
        protected.extend(
            entries
                .iter()
                .filter(|e| e.name.eq_ignore_ascii_case(&name))
                .map(|e| e.pid),
        );
    }
    loop {
        let children: Vec<_> = entries
            .iter()
            .filter(|e| protected.contains(&e.parent))
            .map(|e| e.pid)
            .collect();
        let before = protected.len();
        protected.extend(children);
        if before == protected.len() {
            break;
        }
    }
    protected
}

pub fn excluded_name(name: &str, exclusions: &[String]) -> bool {
    const SYSTEM: &[&str] = &[
        "system",
        "registry",
        "memory compression",
        "smss.exe",
        "csrss.exe",
        "wininit.exe",
        "winlogon.exe",
        "services.exe",
        "lsass.exe",
        "svchost.exe",
        "dwm.exe",
        "audiodg.exe",
        "fontdrvhost.exe",
        "sihost.exe",
        "explorer.exe",
        "msmpeng.exe",
        "securityhealthservice.exe",
    ];
    SYSTEM.iter().any(|s| name.eq_ignore_ascii_case(s))
        || exclusions.iter().any(|s| name.eq_ignore_ascii_case(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(pid: u32, parent: u32, name: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent,
            name: name.into(),
        }
    }
    #[test]
    fn foreground_tree_and_same_browser_name_are_protected() {
        let entries = vec![
            entry(1, 0, "explorer.exe"),
            entry(2, 1, "browser.exe"),
            entry(3, 2, "browser.exe"),
            entry(4, 3, "helper.exe"),
            entry(5, 1, "other.exe"),
            entry(6, 1, "browser.exe"),
        ];
        let protected = protected_family(&entries, 3);
        assert!([2, 3, 4, 6].iter().all(|p| protected.contains(p)));
        assert!(!protected.contains(&1));
        assert!(!protected.contains(&5));
        assert!(excluded_name("LSASS.EXE", &[]));
        assert!(excluded_name("Game.EXE", &["game.exe".into()]));
    }
    #[test]
    fn missing_parents_and_cycles_terminate() {
        let entries = vec![entry(1, 2, "a.exe"), entry(2, 1, "b.exe")];
        assert_eq!(protected_family(&entries, 1).len(), 2);
        assert!(protected_family(&[], 99).contains(&99));
    }
}

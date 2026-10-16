#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(windows))]
pub fn snapshot() -> crate::AppResult<crate::model::MemorySnapshot> {
    Err("仅支持 Windows 10/11".into())
}
#[cfg(not(windows))]
pub fn clean(_: crate::model::Profile) -> crate::AppResult<crate::model::CleanReport> {
    Err("仅支持 Windows 10/11".into())
}

#[cfg(not(windows))]
pub fn clean_with_config(
    _: crate::model::Profile,
    _: &crate::config::Config,
    _: Option<u32>,
) -> crate::AppResult<crate::model::CleanReport> {
    Err("仅支持 Windows 10/11".into())
}

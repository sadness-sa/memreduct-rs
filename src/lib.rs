pub mod config;
pub mod model;
pub mod platform;
pub mod policy;
#[cfg(windows)]
pub mod ui;

pub type AppResult<T> = Result<T, String>;

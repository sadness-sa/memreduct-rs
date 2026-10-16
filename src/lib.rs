pub mod config;
pub mod model;
pub mod platform;
pub mod policy;
pub mod selection;
#[cfg(windows)]
mod settings;
#[cfg(windows)]
pub mod ui;

pub type AppResult<T> = Result<T, String>;

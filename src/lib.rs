pub mod config;
pub mod model;
pub mod platform;
pub mod policy;
pub mod selection;
#[cfg(windows)]
mod settings;
pub mod strategy;
#[cfg(windows)]
pub mod tray;
#[cfg(windows)]
pub mod ui;

pub type AppResult<T> = Result<T, String>;

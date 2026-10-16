#![cfg_attr(windows, windows_subsystem = "windows")]

fn main() {
    #[cfg(windows)]
    if let Err(error) = memreduct_rs::ui::run() {
        memreduct_rs::ui::show_error(&error);
    }
    #[cfg(not(windows))]
    {
        eprintln!("MemReduct RS GUI 仅支持 Windows 10/11");
        std::process::exit(1);
    }
}

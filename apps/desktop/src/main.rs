// Prevents additional console window on Windows in release mode
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(e) = rove_desktop::run() {
        eprintln!("Fatal error: {}", e);
        std::process::exit(1);
    }
}

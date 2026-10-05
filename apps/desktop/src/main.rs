// Prevents additional console window on Windows in release mode
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if rove_desktop::run().is_err() {
        let log_path = rove_desktop::config::record_startup_failure()
            .unwrap_or_else(|_| std::path::PathBuf::from("the Rove application logs directory"));
        let message = rove_desktop::startup_failure_message(&log_path);
        let _ = rfd::MessageDialog::new()
            .set_title("Rove could not start")
            .set_description(message)
            .set_level(rfd::MessageLevel::Error)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
        std::process::exit(1);
    }
}

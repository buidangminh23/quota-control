#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = quota_control_lib::run() {
        eprintln!("Quota Control failed: {error}");
        std::process::exit(1);
    }
}

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = usage_control_lib::run() {
        eprintln!("Usage Control failed: {error}");
        std::process::exit(1);
    }
}

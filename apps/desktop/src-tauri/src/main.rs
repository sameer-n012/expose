// Do not open a console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = expose_desktop::run() {
        eprintln!("Expose stopped because of an error: {error}");
        std::process::exit(1);
    }
}

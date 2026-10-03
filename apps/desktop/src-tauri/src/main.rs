// Do not open a console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Write log events to stderr. This is the first subscriber, so
    // `try_init` can fail only if a library set one before; report it.
    if let Err(error) = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .try_init()
    {
        eprintln!("Expose cannot start logging: {error}");
    }
    if let Err(error) = expose_desktop::run() {
        eprintln!("Expose stopped because of an error: {error}");
        std::process::exit(1);
    }
}

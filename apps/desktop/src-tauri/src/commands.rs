//! Tauri commands that the UI can call with `invoke`.
//!
//! Commands are in this module and not at the crate root. A `pub` command at
//! the crate root makes the `#[tauri::command]` macro define its helper
//! macros two times, and the crate does not compile.

/// Returns the version of the application.
///
/// The UI calls this command with `invoke("app_version")`.
#[tauri::command]
#[must_use]
pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::app_version;

    #[test]
    fn app_version_matches_crate_version() {
        assert_eq!(app_version(), env!("CARGO_PKG_VERSION"));
    }
}

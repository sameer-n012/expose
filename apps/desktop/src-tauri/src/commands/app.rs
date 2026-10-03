//! Application information commands.

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

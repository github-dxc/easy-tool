use std::path::{Path, PathBuf};

pub fn application_root() -> PathBuf {
    application_root_from_exe_path(std::env::current_exe().ok())
}

pub fn application_root_from_exe_path(exe_path: Option<PathBuf>) -> PathBuf {
    exe_path
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

pub fn clipboard_history_dir() -> PathBuf {
    clipboard_history_dir_from_root(&application_root())
}

pub fn clipboard_history_dir_from_root(root: &Path) -> PathBuf {
    root.join("temp").join("screenshot")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn resolves_application_root_from_executable_parent() {
        let dir = tempdir().expect("tempdir should be created");
        let exe_path = dir.path().join("easy-tool.exe");

        assert_eq!(application_root_from_exe_path(Some(exe_path)), dir.path());
    }

    #[test]
    fn clipboard_history_dir_is_under_temp_screenshot() {
        let root = PathBuf::from("application-root");

        assert_eq!(
            clipboard_history_dir_from_root(&root),
            root.join("temp").join("screenshot")
        );
    }
}

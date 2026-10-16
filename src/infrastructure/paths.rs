use std::path::{Path, PathBuf};

/// 根据当前可执行文件位置获取应用运行目录。
pub fn application_root() -> PathBuf {
    application_root_from_exe_path(std::env::current_exe().ok())
}

/// 提取可执行文件的父目录，无法获取时使用当前工作目录。
pub fn application_root_from_exe_path(exe_path: Option<PathBuf>) -> PathBuf {
    exe_path
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// 返回当前应用的剪贴板历史缓存目录。
pub fn clipboard_history_dir() -> PathBuf {
    clipboard_history_dir_from_root(&application_root())
}

/// 保留旧版缓存目录名称，兼容已保存的历史图片绝对路径。
pub fn clipboard_history_dir_from_root(root: &Path) -> PathBuf {
    // Keep the historical cache directory so existing clipboard records remain valid.
    root.join("temp").join("screenshot")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    /// 验证运行目录取自可执行文件的父目录。
    fn resolves_application_root_from_executable_parent() {
        let dir = tempdir().expect("tempdir should be created");
        let exe_path = dir.path().join("easy-tool.exe");

        assert_eq!(application_root_from_exe_path(Some(exe_path)), dir.path());
    }

    #[test]
    /// 验证历史缓存继续使用旧版目录，避免已有图片记录失效。
    fn clipboard_history_dir_is_under_temp_screenshot() {
        let root = PathBuf::from("application-root");

        assert_eq!(
            clipboard_history_dir_from_root(&root),
            root.join("temp").join("screenshot")
        );
    }
}

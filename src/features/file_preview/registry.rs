//! OS integration for adding the file preview action to the file context menu.

/// Registers the file-preview context menu entry when supported by the OS.
pub fn register_file_context_menu() -> Result<(), String> {
    register_file_context_menu_impl()
}

#[cfg(target_os = "windows")]
fn register_file_context_menu_impl() -> Result<(), String> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let exe_path = std::env::current_exe()
        .map_err(|err| format!("get current exe failed: {err}"))?
        .display()
        .to_string();
    let command = format!("\"{exe_path}\" \"%1\"");

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (shell_key, _) = hkcu
        .create_subkey("Software\\Classes\\*\\shell\\easy-tool-preview")
        .map_err(|err| format!("create context menu key failed: {err}"))?;
    shell_key
        .set_value("", &"用 easytool 打开")
        .map_err(|err| format!("set context menu name failed: {err}"))?;
    shell_key
        .set_value("Icon", &exe_path)
        .map_err(|err| format!("set context menu icon failed: {err}"))?;

    let (command_key, _) = shell_key
        .create_subkey("command")
        .map_err(|err| format!("create context menu command key failed: {err}"))?;
    command_key
        .set_value("", &command)
        .map_err(|err| format!("set context menu command failed: {err}"))?;

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn register_file_context_menu_impl() -> Result<(), String> {
    Ok(())
}

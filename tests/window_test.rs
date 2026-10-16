use easy_tool::*;

// Smoke test that verifies tray construction works with default settings.
#[test]
#[ignore = "需要真实 Windows 桌面会话，使用 --ignored 手动运行"]
/// 在真实桌面会话中手动验证默认配置可以创建托盘图标。
fn test_init_tray_icon() {
    let _ti = init_tray_icon(&settings::AppSettings::default());
}

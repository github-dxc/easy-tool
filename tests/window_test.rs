use easy_tool::*;

// Smoke test that verifies tray construction works with default settings.
#[test]
fn test_init_tray_icon() {
    let _ti = init_tray_icon(&settings::AppSettings::default());
}

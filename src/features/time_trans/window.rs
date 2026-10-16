//! Slint window setup and callbacks for the timestamp converter.

use arboard::Clipboard;
use chrono::{DateTime, Utc};
use i_slint_backend_winit::WinitWindowAccessor;
use slint::ComponentHandle;

use crate::TimeTrans;
use crate::config::{TIMEZONE_LABELS, TIMEZONES};
use crate::features::time_trans::converter::trans_string_timestamp;
use crate::platform::window::{hide_taskbar_icon, show_centered_tool_window};

/// 创建时间转换浮窗，绑定显示、复制、转换、拖动及时区回调。
pub fn init_time_trans_window() -> TimeTrans {
    let time_window = TimeTrans::new().unwrap();

    time_window.set_timezone_index(0);
    time_window.set_timezone_label(TIMEZONE_LABELS[0].into());

    bind_visibility_callbacks(&time_window);
    bind_clipboard_callbacks(&time_window);
    bind_conversion_callbacks(&time_window);
    bind_window_drag_callback(&time_window);
    bind_timezone_callbacks(&time_window);
    bind_current_time_callback(&time_window);

    time_window
}

/// 绑定显示与关闭操作，关闭时隐藏浮窗而不销毁应用。
fn bind_visibility_callbacks(time_window: &TimeTrans) {
    let weak = time_window.as_weak();
    time_window.on_close_window(move || {
        if let Some(ui) = weak.upgrade() {
            next_display_generation(&ui);
            ui.set_live_clock(false);
            ui.set_auto_close(true);
            let _ = ui.hide();
        }
    });
}

/// 绑定结果复制操作，将转换后的文本写入剪贴板。
fn bind_clipboard_callbacks(time_window: &TimeTrans) {
    time_window.on_copy_to_clipboard(|text| {
        let mut clipboard = Clipboard::new().unwrap();
        clipboard.set_text(text.to_string()).unwrap();
    });
}

/// 输入、单位或时区变化时重新转换，并更新结果与复制状态。
fn bind_conversion_callbacks(time_window: &TimeTrans) {
    let weak = time_window.as_weak();
    time_window.on_update_result(move |input_value, unit, timezone_index| {
        let (result, unit) = trans_string_timestamp(
            input_value.as_str(),
            unit,
            TIMEZONES[timezone_index as usize],
        );

        if let Some(ui) = weak.upgrade() {
            match result {
                Ok(result_value) => {
                    ui.set_result_value(result_value.into());
                    ui.set_has_copy(false);
                    if let Some(unit) = unit {
                        ui.set_timestamp_unit(unit);
                    }
                }
                Err(message) => {
                    ui.set_result_value(message.into());
                }
            }
        }
    });
}

/// 将浮窗拖动操作转交原生窗口处理。
fn bind_window_drag_callback(time_window: &TimeTrans) {
    let weak = time_window.as_weak();
    time_window.on_move_window(move || {
        if let Some(ui) = weak.upgrade() {
            ui.window().with_winit_window(|winit_window| {
                let _ = winit_window.drag_window();
            });
        }
    });
}

/// 绑定前后切换时区操作，并在列表边界循环切换。
fn bind_timezone_callbacks(time_window: &TimeTrans) {
    let weak = time_window.as_weak();
    time_window.on_last_timezone(move |mut index| {
        if index == 0 {
            index = (TIMEZONES.len() - 1) as i32;
        } else {
            index -= 1;
        }

        if let Some(ui) = weak.upgrade() {
            ui.set_timezone_index(index);
            ui.set_timezone_label(TIMEZONE_LABELS[index as usize].into());
        }
    });

    let weak = time_window.as_weak();
    time_window.on_next_timezone(move |mut index| {
        if index as usize >= TIMEZONES.len() - 1 {
            index = 0;
        } else {
            index += 1;
        }

        if let Some(ui) = weak.upgrade() {
            ui.set_timezone_index(index);
            ui.set_timezone_label(TIMEZONE_LABELS[index as usize].into());
        }
    });
}

/// 显示复制内容的转换结果，并将浮窗放到鼠标附近的可见位置。
pub fn show_copied_timestamp(window: &TimeTrans, text: &str, position: (f64, f64)) {
    let generation = next_display_generation(window);
    window.set_live_clock(false);
    window.set_auto_close(true);
    window.set_input_value(text.trim().into());
    window.set_close_time(3);
    // 显式完成转换后再决定是否显示，相同文本再次复制也能重新触发定位。
    window.invoke_update_result(
        window.get_input_value(),
        window.get_timestamp_unit(),
        window.get_timezone_index(),
    );
    if window.get_result_value() == "Error" {
        return;
    }

    let weak = window.as_weak();
    if let Err(err) = slint::spawn_local(async move {
        let Some(window) = weak.upgrade() else {
            return;
        };
        if window.get_display_generation() != generation {
            return;
        }
        // 首次使用时只创建隐藏的原生窗口，禁止在定位完成前显示默认位置。
        let native = match window.window().winit_window().await {
            Ok(native) => native,
            Err(err) => {
                log::error!("prepare copied timestamp window failed: {err}");
                return;
            }
        };
        if window.get_display_generation() != generation {
            return;
        }
        let was_visible = window.window().is_visible();
        if !was_visible {
            hide_taskbar_icon(&window);
            window.window().set_size(slint::LogicalSize::new(
                window.get_initial_width(),
                window.get_initial_height(),
            ));
        }
        if !was_visible || !window.get_has_hover() {
            let (x, y) = next_window_position(&native, position);
            native.set_outer_position(winit::dpi::PhysicalPosition::new(x, y));
        }
        window.set_close_time(3);
        if let Err(err) = window.show() {
            log::error!("show copied timestamp window failed: {err}");
        }
    }) {
        log::error!("schedule copied timestamp window failed: {err}");
    }
}

/// 从主页打开时填入当前时间并居中显示，开启每秒刷新及手动关闭模式。
pub fn show_current_timestamp(window: &TimeTrans) {
    next_display_generation(window);
    window.set_auto_close(false);
    window.set_live_clock(true);
    window.invoke_refresh_current_time();
    show_centered_tool_window(
        window,
        slint::LogicalSize::new(window.get_initial_width(), window.get_initial_height()),
    );
}

/// 递增显示代次，让等待原生窗口的旧复制请求无法重新弹出浮窗。
fn next_display_generation(window: &TimeTrans) -> i32 {
    let generation = window.get_display_generation().wrapping_add(1);
    window.set_display_generation(generation);
    generation
}

/// 绑定实时刷新回调，按当前选中的时区生成可直接转换的时间字符串。
fn bind_current_time_callback(window: &TimeTrans) {
    let weak = window.as_weak();
    window.on_refresh_current_time(move || {
        if let Some(window) = weak.upgrade() {
            let zone = TIMEZONES[window.get_timezone_index() as usize];
            window.set_input_value(current_time_string(Utc::now(), zone).into());
        }
    });
}

/// 将当前时刻转换为选定时区的日期时间，保持与输入解析格式一致。
fn current_time_string(now: DateTime<Utc>, zone: &str) -> String {
    let timezone: chrono_tz::Tz = zone.parse().unwrap_or(chrono_tz::Asia::Shanghai);
    now.with_timezone(&timezone)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

/// 按鼠标所在屏幕与浮窗实际物理尺寸定位，兼容缩放、多屏及负坐标。
fn next_window_position(window: &winit::window::Window, (x, y): (f64, f64)) -> (f64, f64) {
    let monitor = window
        .available_monitors()
        .find(|monitor| {
            let origin = monitor.position();
            let size = monitor.size();
            x >= origin.x as f64
                && y >= origin.y as f64
                && x < origin.x as f64 + size.width as f64
                && y < origin.y as f64 + size.height as f64
        })
        .or_else(|| window.current_monitor())
        .or_else(|| window.primary_monitor());
    let Some(monitor) = monitor else {
        return (x + 20.0, y + 10.0);
    };
    let origin = monitor.position();
    let size = monitor.size();
    let outer = window.outer_size();
    // 移到另一块屏幕时缩放比例可能改变，按目标屏幕估算最终物理尺寸。
    let scale = monitor.scale_factor() / window.scale_factor();
    clamp_popup_position(
        (x, y),
        (origin.x as f64, origin.y as f64),
        (size.width as f64, size.height as f64),
        (
            (outer.width as f64 * scale).round(),
            (outer.height as f64 * scale).round(),
        ),
    )
}

/// 优先放到鼠标右下方；靠近屏幕边缘时收回屏内，屏幕过小时对齐其左上角。
fn clamp_popup_position(
    (x, y): (f64, f64),
    (left, top): (f64, f64),
    (screen_width, screen_height): (f64, f64),
    (window_width, window_height): (f64, f64),
) -> (f64, f64) {
    (
        (x + 20.0).clamp(left, (left + screen_width - window_width).max(left)),
        (y + 10.0).clamp(top, (top + screen_height - window_height).max(top)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证右下偏移、缩放后的边缘限制，以及位于主屏左侧的负坐标屏幕。
    #[test]
    fn popup_position_respects_cursor_screen_and_physical_size() {
        assert_eq!(
            clamp_popup_position((640.0, 360.0), (0.0, 0.0), (1920.0, 1080.0), (315.0, 150.0)),
            (660.0, 370.0)
        );
        assert_eq!(
            clamp_popup_position(
                (1900.0, 1060.0),
                (0.0, 0.0),
                (1920.0, 1080.0),
                (315.0, 150.0)
            ),
            (1605.0, 930.0)
        );
        assert_eq!(
            clamp_popup_position(
                (-1910.0, 100.0),
                (-1920.0, 0.0),
                (1920.0, 1080.0),
                (315.0, 150.0)
            ),
            (-1890.0, 110.0)
        );
        assert_eq!(
            clamp_popup_position((50.0, 50.0), (0.0, 0.0), (100.0, 80.0), (315.0, 150.0)),
            (0.0, 0.0)
        );
    }

    /// 验证实时输入按所选时区显示，并能换算回同一个当前时间戳。
    #[test]
    fn live_time_matches_selected_timezone_and_conversion() {
        let now = DateTime::parse_from_rfc3339("2026-10-16T12:34:56Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            current_time_string(now, "Asia/Shanghai"),
            "2026-10-16 20:34:56"
        );
        assert_eq!(current_time_string(now, "Etc/UTC"), "2026-10-16 12:34:56");
        assert_eq!(
            current_time_string(now, "America/New_York"),
            "2026-10-16 08:34:56"
        );
        for zone in TIMEZONES {
            let (converted, _) =
                trans_string_timestamp(&current_time_string(now, zone), false, zone);
            assert_eq!(converted.unwrap(), now.timestamp().to_string());
        }
    }
}

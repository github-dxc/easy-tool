use arboard::Clipboard;
use i_slint_backend_winit::WinitWindowAccessor;
use slint::ComponentHandle;

use crate::TimeTrans;
use crate::assets::load_slint_image;
use crate::config::{TIMEZONE_LABELS, TIMEZONES};
use crate::features::time_trans::converter::trans_string_timestamp;
use crate::platform::window::hide_taskbar_icon;

pub fn init_time_trans_window() -> TimeTrans {
    let time_window = TimeTrans::new().unwrap();

    time_window.set_timezone_index(0);
    time_window.set_timezone_label(TIMEZONE_LABELS[0].into());
    time_window.set_close_img(load_close_image());

    bind_visibility_callbacks(&time_window);
    bind_clipboard_callbacks(&time_window);
    bind_conversion_callbacks(&time_window);
    bind_window_drag_callback(&time_window);
    bind_timezone_callbacks(&time_window);

    time_window
}

fn load_close_image() -> slint::Image {
    const CLOSE_IMG: &[u8] = include_bytes!("../../../assets/icons/close.png");
    load_slint_image(CLOSE_IMG)
}

fn bind_visibility_callbacks(time_window: &TimeTrans) {
    let weak = time_window.as_weak();
    time_window.on_close_window(move || {
        if let Some(ui) = weak.upgrade() {
            let _ = ui.hide();
        }
    });

    let weak = time_window.as_weak();
    time_window.on_show_window(move || {
        if let Some(ui) = weak.upgrade() {
            let _ = ui.show();
            hide_taskbar_icon(&ui);
        }
    });
}

fn bind_clipboard_callbacks(time_window: &TimeTrans) {
    time_window.on_copy_to_clipboard(|text| {
        let mut clipboard = Clipboard::new().unwrap();
        clipboard.set_text(text.to_string()).unwrap();
    });
}

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

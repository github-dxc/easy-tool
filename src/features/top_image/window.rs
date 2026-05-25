//! Standalone pinned-image windows created from screenshot selections.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use image::RgbaImage;
use slint::{CloseRequestResponse, ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::ScreenshotWindow;
use crate::TopImageWindow;
use crate::features::screenshot::window::show_screenshot_window;
use crate::platform::window::{
    cursor_position, set_window_position, show_without_taskbar_icon, window_position,
};

const BORDER_WIDTH: u32 = 1;
const SHADOW_PADDING: u32 = 8;

thread_local! {
    static PINNED_IMAGE_WINDOWS: RefCell<Vec<PinnedImageWindow>> = const { RefCell::new(Vec::new()) };
    static NEXT_PINNED_IMAGE_WINDOW_ID: Cell<u64> = const { Cell::new(1) };
}

struct PinnedImageWindow {
    id: u64,
    window: TopImageWindow,
    restore_after_screenshot: bool,
}

#[derive(Clone, Copy)]
struct DragState {
    cursor: (f64, f64),
    window_pos: (f64, f64),
}

/// Opens a new pinned-image window for a cropped screenshot selection.
pub fn open_pinned_image(
    image: RgbaImage,
    screen_x: i32,
    screen_y: i32,
    screenshot_window: slint::Weak<ScreenshotWindow>,
) -> Result<(), String> {
    let image_width = image.width();
    let image_height = image.height();
    let window = TopImageWindow::new().map_err(|err| format!("create top image failed: {err}"))?;
    let scale_factor = window.window().scale_factor().max(1.0);
    let logical_image_width = image_width as f32 / scale_factor;
    let logical_image_height = image_height as f32 / scale_factor;
    let logical_window_width =
        logical_image_width + BORDER_WIDTH as f32 * 2.0 + SHADOW_PADDING as f32 * 2.0;
    let logical_window_height =
        logical_image_height + BORDER_WIDTH as f32 * 2.0 + SHADOW_PADDING as f32 * 2.0;
    let window_x = f64::from(screen_x - BORDER_WIDTH as i32 - SHADOW_PADDING as i32);
    let window_y = f64::from(screen_y - BORDER_WIDTH as i32 - SHADOW_PADDING as i32);

    window.set_image(image_from_rgba(&image));
    window.set_image_width(logical_image_width);
    window.set_image_height(logical_image_height);
    window.window().set_size(slint::LogicalSize::new(
        logical_window_width,
        logical_window_height,
    ));

    let id = next_window_id();
    bind_drag_callbacks(&window);
    bind_close_callbacks(&window, id);
    bind_screenshot_shortcut(&window, screenshot_window);

    show_without_taskbar_icon(&window).map_err(|err| format!("show top image failed: {err}"))?;
    window.invoke_focus_keyboard();
    set_window_position(&window, window_x, window_y);
    schedule_position_fix(window.as_weak(), window_x, window_y);

    PINNED_IMAGE_WINDOWS.with(|windows| {
        windows.borrow_mut().push(PinnedImageWindow {
            id,
            window,
            restore_after_screenshot: false,
        });
    });
    Ok(())
}

/// Temporarily hides visible pinned images so the next screenshot overlay is unobstructed.
pub fn hide_pinned_images_for_screenshot() {
    PINNED_IMAGE_WINDOWS.with(|windows| {
        for pinned in windows.borrow_mut().iter_mut() {
            let is_visible = pinned.window.window().is_visible();
            pinned.restore_after_screenshot = is_visible;

            if is_visible {
                let _ = pinned.window.hide();
            }
        }
    });
}

fn schedule_position_fix(window: slint::Weak<TopImageWindow>, x: f64, y: f64) {
    slint::Timer::single_shot(Duration::from_millis(0), move || {
        if let Some(window) = window.upgrade() {
            set_window_position(&window, x, y);
        }
    });
}

/// Restores pinned images hidden by `hide_pinned_images_for_screenshot`.
pub fn restore_pinned_images_after_screenshot() {
    PINNED_IMAGE_WINDOWS.with(|windows| {
        for pinned in windows.borrow_mut().iter_mut() {
            if pinned.restore_after_screenshot {
                let _ = show_without_taskbar_icon(&pinned.window);
                pinned.window.invoke_focus_keyboard();
            }
            pinned.restore_after_screenshot = false;
        }
    });
}

fn next_window_id() -> u64 {
    NEXT_PINNED_IMAGE_WINDOW_ID.with(|next_id| {
        let id = next_id.get();
        next_id.set(id.saturating_add(1));
        id
    })
}

fn bind_drag_callbacks(window: &TopImageWindow) {
    let drag_state = Rc::new(RefCell::new(None::<DragState>));

    let weak = window.as_weak();
    let state = Rc::clone(&drag_state);
    window.on_start_drag(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Some(cursor) = cursor_position() else {
            return;
        };
        let Some(window_pos) = window_position(&ui) else {
            return;
        };
        *state.borrow_mut() = Some(DragState { cursor, window_pos });
    });

    let weak = window.as_weak();
    let state = Rc::clone(&drag_state);
    window.on_drag(move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Some(state) = *state.borrow() else {
            return;
        };
        let Some((cursor_x, cursor_y)) = cursor_position() else {
            return;
        };
        let dx = cursor_x - state.cursor.0;
        let dy = cursor_y - state.cursor.1;
        set_window_position(&ui, state.window_pos.0 + dx, state.window_pos.1 + dy);
    });

    let state = Rc::clone(&drag_state);
    window.on_end_drag(move || {
        *state.borrow_mut() = None;
    });
}

fn bind_close_callbacks(window: &TopImageWindow, id: u64) {
    window.on_close_image(move || {
        close_pinned_image(id);
    });

    window.window().on_close_requested(move || {
        close_pinned_image(id);
        CloseRequestResponse::HideWindow
    });
}

fn bind_screenshot_shortcut(
    window: &TopImageWindow,
    screenshot_window: slint::Weak<ScreenshotWindow>,
) {
    window.on_screenshot_shortcut(move || {
        let screenshot_window = screenshot_window.clone();
        let _ = screenshot_window.upgrade_in_event_loop(|window| {
            show_screenshot_window(&window);
        });
    });
}

fn close_pinned_image(id: u64) {
    PINNED_IMAGE_WINDOWS.with(|windows| {
        let mut windows = windows.borrow_mut();
        if let Some(index) = windows.iter().position(|pinned| pinned.id == id) {
            let pinned = windows.remove(index);
            let _ = pinned.window.hide();
        }
    });
}

fn image_from_rgba(source: &RgbaImage) -> Image {
    let buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
        source.as_raw(),
        source.width(),
        source.height(),
    );
    Image::from_rgba8(buffer)
}

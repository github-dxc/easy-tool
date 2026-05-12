use std::io;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use clipboard_master::{CallbackResult, ClipboardHandler, Master};
use slint::ComponentHandle;

use crate::ClipboardHistoryWindow;
use crate::features::clipboard_history::clipboard::capture_clipboard_item;
use crate::features::clipboard_history::history::ClipboardHistory;
use crate::features::clipboard_history::window::refresh_clipboard_history_window;
use crate::settings::AppSettings;

pub fn start_clipboard_history_listener(
    history: Arc<Mutex<ClipboardHistory>>,
    settings: Arc<Mutex<AppSettings>>,
    history_window: slint::Weak<ClipboardHistoryWindow>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("clipboard-master-listener".into())
        .spawn(move || {
            let handler = HistoryClipboardHandler {
                history,
                settings,
                history_window,
            };

            let mut master = match Master::new(handler) {
                Ok(master) => master,
                Err(err) => {
                    log::error!("create clipboard listener failed: {err}");
                    return;
                }
            };

            if let Err(err) = master.run() {
                log::error!("clipboard listener error: {err}");
            }
        })
        .map_err(|err| format!("spawn clipboard listener failed: {err}"))?;

    Ok(())
}

struct HistoryClipboardHandler {
    history: Arc<Mutex<ClipboardHistory>>,
    settings: Arc<Mutex<AppSettings>>,
    history_window: slint::Weak<ClipboardHistoryWindow>,
}

impl ClipboardHandler for HistoryClipboardHandler {
    fn on_clipboard_change(&mut self) -> CallbackResult {
        if !self.settings.lock().unwrap().clipboard_history.enabled {
            return CallbackResult::Next;
        }

        // Give the source application a brief moment to finish publishing all formats.
        thread::sleep(Duration::from_millis(80));

        let Some(item) = capture_clipboard_item() else {
            return CallbackResult::Next;
        };

        self.history.lock().unwrap().push(item);

        let history = Arc::clone(&self.history);
        if let Err(err) = self.history_window.upgrade_in_event_loop(move |window| {
            if window.window().is_visible() {
                refresh_clipboard_history_window(&window, &history);
            }
        }) {
            log::error!("refresh clipboard history window failed: {err}");
        }

        CallbackResult::Next
    }

    fn on_clipboard_error(&mut self, error: io::Error) -> CallbackResult {
        log::error!("clipboard listener callback error: {error}");
        CallbackResult::Next
    }
}

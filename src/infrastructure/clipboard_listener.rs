//! Clipboard watcher that captures changed content into clipboard history.

use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use clipboard_master::{CallbackResult, ClipboardHandler, Master};
use slint::ComponentHandle;

use crate::ClipboardHistoryWindow;
use crate::features::clipboard_history::clipboard::capture_clipboard_item;
use crate::features::clipboard_history::history::{ClipboardHistory, ClipboardHistoryItem};
use crate::features::clipboard_history::store::{
    delete_image_files, item_from_capture, save_history,
};
use crate::features::clipboard_history::window::refresh_clipboard_history_window;
use crate::settings::AppSettings;

/// Spawns the OS clipboard listener on a background thread.
pub fn start_clipboard_history_listener(
    history: Arc<Mutex<ClipboardHistory>>,
    settings: Arc<Mutex<AppSettings>>,
    history_window: slint::Weak<ClipboardHistoryWindow>,
    suppress_next_clipboard_history: Arc<Mutex<Option<ClipboardHistoryItem>>>,
    history_dir: PathBuf,
) -> Result<(), String> {
    thread::Builder::new()
        .name("clipboard-master-listener".into())
        .spawn(move || {
            let handler = HistoryClipboardHandler {
                history,
                settings,
                history_window,
                suppress_next_clipboard_history,
                history_dir,
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

// Bridges clipboard-master callbacks to the shared history model and UI refresh.
struct HistoryClipboardHandler {
    history: Arc<Mutex<ClipboardHistory>>,
    settings: Arc<Mutex<AppSettings>>,
    history_window: slint::Weak<ClipboardHistoryWindow>,
    suppress_next_clipboard_history: Arc<Mutex<Option<ClipboardHistoryItem>>>,
    history_dir: PathBuf,
}

impl ClipboardHandler for HistoryClipboardHandler {
    fn on_clipboard_change(&mut self) -> CallbackResult {
        if !self.settings.lock().unwrap().clipboard_history.enabled {
            return CallbackResult::Next;
        }

        // Give the source application a brief moment to finish publishing all formats.
        thread::sleep(Duration::from_millis(80));

        let Some(captured) = capture_clipboard_item() else {
            return CallbackResult::Next;
        };

        let item = match item_from_capture(&self.history_dir, captured) {
            Ok(item) => item,
            Err(err) => {
                log::error!("persist clipboard history item failed: {err}");
                return CallbackResult::Next;
            }
        };

        if self
            .suppress_next_clipboard_history
            .lock()
            .unwrap()
            .take()
            .is_some_and(|expected| expected.same_content(&item))
        {
            if let Some(path) = item.image_path() {
                delete_image_files(&self.history_dir, vec![path.to_path_buf()]);
            }
            return CallbackResult::Next;
        }

        let cleanup_paths = {
            let mut history = self.history.lock().unwrap();
            let mut next_history = history.clone();
            let cleanup_paths = next_history.push(item.clone());
            if let Err(err) = save_history(&self.history_dir, &next_history) {
                log::error!("save clipboard history failed: {err}");
                if let Some(path) = item.image_path() {
                    delete_image_files(&self.history_dir, vec![path.to_path_buf()]);
                }
                Vec::new()
            } else {
                *history = next_history;
                cleanup_paths
            }
        };
        delete_image_files(&self.history_dir, cleanup_paths);

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

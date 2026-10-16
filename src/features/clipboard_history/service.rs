//! 历史记录服务：集中管理采集、持久化、删除和粘贴抑制。

use super::clipboard::{capture_clipboard_item, put_clipboard_item};
use super::history::{ClipboardHistory, ClipboardHistoryItem};
use super::store::{
    CapturedClipboardItem, delete_image_files, item_from_capture, load_history, save_history,
};
use crate::infrastructure::clipboard_listener::start_clipboard_listener;
use crate::settings::AppSettings;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub struct ClipboardHistoryService {
    history: Mutex<ClipboardHistory>,
    directory: PathBuf,
    suppress_next: Mutex<Option<ClipboardHistoryItem>>,
    started: AtomicBool,
}

impl ClipboardHistoryService {
    /// 恢复磁盘历史；文件损坏时记录错误并以空历史继续运行。
    pub fn load(directory: PathBuf) -> Self {
        let history = load_history(&directory).unwrap_or_else(|err| {
            log::error!("load clipboard history failed: {err}");
            ClipboardHistory::default()
        });
        Self {
            history: Mutex::new(history),
            directory,
            suppress_next: Mutex::new(None),
            started: AtomicBool::new(false),
        }
    }

    /// 返回历史快照，图片解码等耗时操作应在释放历史锁之后进行。
    pub fn items(&self) -> Vec<ClipboardHistoryItem> {
        self.history.lock().unwrap().items()
    }

    /// 启动一次后台监听；关闭开关时跳过采集，监听退出后允许重试。
    pub fn start_listener(
        self: &Arc<Self>,
        settings: Arc<Mutex<AppSettings>>,
        on_changed: impl Fn() + Send + 'static,
    ) -> Result<(), String> {
        if self.started.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let service = Arc::clone(self);
        let stopped_service = Arc::clone(self);
        let result = start_clipboard_listener(
            move || {
                if !settings.lock().unwrap().clipboard_history.enabled {
                    return;
                }
                let Some(captured) = capture_clipboard_item() else {
                    return;
                };
                match service.record(captured) {
                    Ok(true) => on_changed(),
                    Ok(false) => {}
                    Err(err) => log::error!("record clipboard history failed: {err}"),
                }
            },
            move || {
                stopped_service.started.store(false, Ordering::SeqCst);
            },
        );
        if result.is_err() {
            self.started.store(false, Ordering::SeqCst);
        }
        result
    }

    /// 保存采集结果，提交成功后再替换内存数据并清理淘汰图片。
    fn record(&self, captured: CapturedClipboardItem) -> Result<bool, String> {
        let item = item_from_capture(&self.directory, captured)?;
        if self
            .suppress_next
            .lock()
            .unwrap()
            .take()
            .is_some_and(|expected| expected.same_content(&item))
        {
            if let Some(path) = item.image_path() {
                delete_image_files(&self.directory, [path.to_path_buf()]);
            }
            return Ok(false);
        }
        let cleanup = {
            let mut history = self.history.lock().unwrap();
            let mut next = history.clone();
            let cleanup = next.push(item.clone());
            // 落盘失败时不提交内存修改，也不删除仍被旧历史引用的图片。
            if let Err(err) = save_history(&self.directory, &next) {
                if let Some(path) = item.image_path() {
                    delete_image_files(&self.directory, [path.to_path_buf()]);
                }
                return Err(err);
            }
            *history = next;
            cleanup
        };
        delete_image_files(&self.directory, cleanup);
        Ok(true)
    }

    /// 将指定记录写回剪贴板，并抑制此次写回产生的重复历史记录。
    pub fn prepare_paste(&self, index: usize) -> Result<bool, String> {
        let Some(item) = self.history.lock().unwrap().get(index) else {
            return Ok(false);
        };
        *self.suppress_next.lock().unwrap() = Some(item.clone());
        if let Err(err) = put_clipboard_item(&item) {
            self.suppress_next.lock().unwrap().take();
            return Err(err);
        }
        Ok(true)
    }

    /// 删除记录并保存索引；保存失败时保留原数据及图片文件。
    pub fn delete(&self, index: usize) -> Result<bool, String> {
        let removed = {
            let mut history = self.history.lock().unwrap();
            let mut next = history.clone();
            let Some(removed) = next.remove(index) else {
                return Ok(false);
            };
            save_history(&self.directory, &next)?;
            *history = next;
            removed
        };
        if let Some(path) = removed.image_path() {
            delete_image_files(&self.directory, [path.to_path_buf()]);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// 验证无需创建窗口也能记录、恢复并删除历史。
    #[test]
    fn records_and_deletes_without_a_window() {
        let dir = tempdir().unwrap();
        let service = ClipboardHistoryService::load(dir.path().to_path_buf());
        assert!(
            service
                .record(CapturedClipboardItem::Text {
                    text: "first".into()
                })
                .unwrap()
        );
        assert!(
            service
                .record(CapturedClipboardItem::Text {
                    text: "second".into()
                })
                .unwrap()
        );
        assert!(service.delete(0).unwrap());
        let restored = ClipboardHistoryService::load(dir.path().to_path_buf());
        assert_eq!(restored.items()[0].full_text(), "first");
        assert!(!service.delete(99).unwrap());
    }

    /// 验证持久化失败时，新增和删除操作都不会破坏原内存历史。
    #[test]
    fn preserves_history_when_persistence_fails() {
        let dir = tempdir().unwrap();
        let directory = dir.path().join("history");
        let service = ClipboardHistoryService::load(directory.clone());
        service
            .record(CapturedClipboardItem::Text {
                text: "retained".into(),
            })
            .unwrap();
        // 移走服务目录并以普通文件占位，稳定模拟目录不可写。
        std::fs::rename(&directory, dir.path().join("backup")).unwrap();
        std::fs::write(&directory, "occupied").unwrap();
        assert!(
            service
                .record(CapturedClipboardItem::Text { text: "new".into() })
                .is_err()
        );
        assert!(service.delete(0).is_err());
        assert_eq!(service.items().len(), 1);
        assert_eq!(service.items()[0].full_text(), "retained");
    }
}

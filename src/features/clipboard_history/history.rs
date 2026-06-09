//! In-memory clipboard history model and display helpers.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use arboard::ImageData;

const MAX_HISTORY_ITEMS: usize = 20;

/// One clipboard entry captured by the history listener.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ClipboardHistoryItem {
    Text {
        text: String,
    },
    Image {
        width: usize,
        height: usize,
        path: PathBuf,
        byte_len: u64,
    },
    Files {
        paths: Vec<PathBuf>,
    },
}

/// Fixed-size, newest-first clipboard history store.
#[derive(Debug, Default)]
pub struct ClipboardHistory {
    items: VecDeque<ClipboardHistoryItem>,
}

impl ClipboardHistory {
    /// Inserts a newest item and returns image paths that should be deleted.
    ///
    /// Returned paths may come from evicted old entries or from a discarded
    /// duplicate image item that was already persisted before dedupe.
    pub fn push(&mut self, item: ClipboardHistoryItem) -> Vec<PathBuf> {
        if self
            .items
            .front()
            .is_some_and(|latest| latest.same_content(&item))
        {
            return item
                .image_path()
                .map(|path| vec![path.to_path_buf()])
                .unwrap_or_default();
        }

        self.items.push_front(item);
        self.trim_to_limit()
    }

    pub fn get(&self, index: usize) -> Option<ClipboardHistoryItem> {
        self.items.get(index).cloned()
    }

    pub fn remove(&mut self, index: usize) -> Option<ClipboardHistoryItem> {
        self.items.remove(index)
    }

    pub fn items(&self) -> Vec<ClipboardHistoryItem> {
        self.items.iter().cloned().collect()
    }

    pub fn retain_valid_images(&mut self) -> Vec<PathBuf> {
        let mut removed = Vec::new();
        self.items.retain(|item| match item {
            ClipboardHistoryItem::Image { path, .. } if !path.exists() => {
                removed.push(path.clone());
                false
            }
            _ => true,
        });
        removed
    }

    pub fn trim_to_limit(&mut self) -> Vec<PathBuf> {
        let mut removed = Vec::new();
        while self.items.len() > MAX_HISTORY_ITEMS {
            if let Some(item) = self.items.pop_back() {
                if let Some(path) = item.image_path() {
                    removed.push(path.to_path_buf());
                }
            }
        }
        removed
    }
}

impl ClipboardHistoryItem {
    /// Short text shown in the history list.
    pub fn title(&self) -> String {
        match self {
            Self::Text { text } => summarize_text(text, 20),
            Self::Image { width, height, .. } => format!("图片 {width} x {height}"),
            Self::Files { paths } => {
                if paths.len() == 1 {
                    paths[0].display().to_string()
                } else {
                    format!("{} 个文件", paths.len())
                }
            }
        }
    }

    /// Longer text shown in the detail panel.
    pub fn detail(&self) -> String {
        match self {
            Self::Text { text } => summarize_text(text, 180),
            Self::Image {
                width,
                height,
                byte_len,
                ..
            } => {
                format!(
                    "RGBA 图片，{} KB，原始尺寸 {width} x {height}",
                    byte_len / 1024
                )
            }
            Self::Files { paths } => paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    /// Full textual representation used for copy/preview display.
    pub fn full_text(&self) -> String {
        match self {
            Self::Text { text } => text.clone(),
            Self::Image { .. } | Self::Files { .. } => self.detail(),
        }
    }

    /// Stable item kind string consumed by the Slint UI.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text",
            Self::Image { .. } => "image",
            Self::Files { .. } => "file",
        }
    }

    pub fn image_path(&self) -> Option<&Path> {
        match self {
            Self::Image { path, .. } => Some(path.as_path()),
            _ => None,
        }
    }

    /// Converts stored image bytes back into clipboard/image preview data.
    pub fn image_data(&self) -> Option<ImageData<'static>> {
        match self {
            Self::Image {
                width,
                height,
                path,
                ..
            } => Some(ImageData {
                width: *width,
                height: *height,
                bytes: image::open(path).ok()?.into_rgba8().into_raw().into(),
            }),
            _ => None,
        }
    }

    pub(crate) fn same_content(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Text { text: left }, Self::Text { text: right }) => left == right,
            (
                Self::Image {
                    width: left_width,
                    height: left_height,
                    path: left_path,
                    byte_len: left_byte_len,
                },
                Self::Image {
                    width: right_width,
                    height: right_height,
                    path: right_path,
                    byte_len: right_byte_len,
                },
            ) => same_image_content(
                *left_width,
                *left_height,
                left_path,
                *left_byte_len,
                *right_width,
                *right_height,
                right_path,
                *right_byte_len,
            ),
            (Self::Files { paths: left }, Self::Files { paths: right }) => left == right,
            _ => false,
        }
    }
}

fn same_image_content(
    left_width: usize,
    left_height: usize,
    left_path: &Path,
    left_byte_len: u64,
    right_width: usize,
    right_height: usize,
    right_path: &Path,
    right_byte_len: u64,
) -> bool {
    if left_width != right_width || left_height != right_height || left_byte_len != right_byte_len {
        return false;
    }

    if left_path == right_path {
        return true;
    }

    if !left_path.exists() || !right_path.exists() {
        return false;
    }

    match (std::fs::read(left_path), std::fs::read(right_path)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn summarize_text(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut summary = normalized.chars().take(max_chars).collect::<String>();
    if normalized.chars().count() > max_chars {
        summary.push_str("...");
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::{ClipboardHistory, ClipboardHistoryItem};
    use std::collections::VecDeque;
    use tempfile::tempdir;

    #[test]
    fn push_returns_evicted_image_paths_when_history_exceeds_limit() {
        let dir = tempdir().unwrap();
        let image_path = dir.path().join("oldest.png");
        std::fs::write(&image_path, b"png").unwrap();
        let mut history = ClipboardHistory::default();
        history.push(ClipboardHistoryItem::Image {
            width: 1,
            height: 1,
            path: image_path.clone(),
            byte_len: 3,
        });
        for index in 0..19 {
            history.push(ClipboardHistoryItem::Text {
                text: format!("entry {index}"),
            });
        }

        let evicted = history.push(ClipboardHistoryItem::Text {
            text: "newest".to_string(),
        });

        assert_eq!(evicted, vec![image_path]);
        assert_eq!(history.items().len(), 20);
    }

    #[test]
    fn push_duplicate_text_front_keeps_existing_item_and_returns_no_cleanup_paths() {
        let mut history = ClipboardHistory::default();
        let item = ClipboardHistoryItem::Text {
            text: "same".to_string(),
        };
        history.push(item.clone());

        let evicted = history.push(item);

        assert!(evicted.is_empty());
        assert_eq!(history.items().len(), 1);
    }

    #[test]
    fn push_duplicate_image_returns_discarded_image_path_for_cleanup() {
        let dir = tempdir().unwrap();
        let first_path = dir.path().join("clipboard-image-1-0.png");
        let second_path = dir.path().join("clipboard-image-1-1.png");
        let bytes = b"same image bytes";
        std::fs::write(&first_path, bytes).unwrap();
        std::fs::write(&second_path, bytes).unwrap();
        let mut history = ClipboardHistory::default();
        history.push(ClipboardHistoryItem::Image {
            width: 2,
            height: 2,
            path: first_path.clone(),
            byte_len: bytes.len() as u64,
        });

        let evicted = history.push(ClipboardHistoryItem::Image {
            width: 2,
            height: 2,
            path: second_path.clone(),
            byte_len: bytes.len() as u64,
        });

        assert_eq!(evicted, vec![second_path]);
        assert_eq!(history.items().len(), 1);
        assert_eq!(
            history
                .get(0)
                .and_then(|item| item.image_path().map(ToOwned::to_owned)),
            Some(first_path)
        );
    }

    #[test]
    fn retain_valid_images_drops_missing_image_entries_and_returns_paths() {
        let dir = tempdir().unwrap();
        let existing = dir.path().join("existing.png");
        let missing = dir.path().join("missing.png");
        std::fs::write(&existing, b"png").unwrap();
        let mut history = ClipboardHistory::default();
        history.push(ClipboardHistoryItem::Image {
            width: 1,
            height: 1,
            path: existing.clone(),
            byte_len: 3,
        });
        history.push(ClipboardHistoryItem::Image {
            width: 1,
            height: 1,
            path: missing.clone(),
            byte_len: 3,
        });

        let removed = history.retain_valid_images();

        assert_eq!(removed, vec![missing]);
        assert_eq!(history.items().len(), 1);
        assert_eq!(
            history
                .get(0)
                .and_then(|item| item.image_path().map(ToOwned::to_owned)),
            Some(existing)
        );
    }

    #[test]
    fn trim_to_limit_returns_evicted_image_paths() {
        let dir = tempdir().unwrap();
        let image_path = dir.path().join("oldest.png");
        std::fs::write(&image_path, b"png").unwrap();
        let mut items = VecDeque::new();
        items.push_back(ClipboardHistoryItem::Image {
            width: 1,
            height: 1,
            path: image_path.clone(),
            byte_len: 3,
        });
        for index in 0..20 {
            items.push_front(ClipboardHistoryItem::Text {
                text: format!("entry {index}"),
            });
        }
        let mut history = ClipboardHistory { items };

        let evicted = history.trim_to_limit();

        assert_eq!(evicted, vec![image_path]);
        assert_eq!(history.items().len(), 20);
    }
}

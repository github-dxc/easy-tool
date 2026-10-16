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
#[derive(Debug, Default, Clone)]
pub struct ClipboardHistory {
    items: VecDeque<ClipboardHistoryItem>,
}

impl ClipboardHistory {
    /// 插入最新记录并去重，返回被淘汰或丢弃记录的待清理图片路径。
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

    /// 按列表位置复制一条历史记录，索引不存在时返回空值。
    pub fn get(&self, index: usize) -> Option<ClipboardHistoryItem> {
        self.items.get(index).cloned()
    }

    /// 移除指定位置的历史记录，并返回被移除的条目。
    pub fn remove(&mut self, index: usize) -> Option<ClipboardHistoryItem> {
        self.items.remove(index)
    }

    /// 按最新在前的顺序返回历史记录快照。
    pub fn items(&self) -> Vec<ClipboardHistoryItem> {
        self.items.iter().cloned().collect()
    }

    /// 剔除图片文件已不存在的历史记录，返回被剔除的路径。
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

    /// 移除超过历史数量上限的旧记录，收集待清理图片路径。
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
    /// 生成历史列表中展示的简短标题。
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

    /// 生成文本摘要、图片尺寸或文件路径等详细描述。
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

    /// 返回完整文本；图片和文件条目使用详细描述代替。
    pub fn full_text(&self) -> String {
        match self {
            Self::Text { text } => text.clone(),
            Self::Image { .. } | Self::Files { .. } => self.detail(),
        }
    }

    /// 返回界面使用的记录类型标识。
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text",
            Self::Image { .. } => "image",
            Self::Files { .. } => "file",
        }
    }

    /// 返回图片条目的缓存路径，其他内容类型返回空值。
    pub fn image_path(&self) -> Option<&Path> {
        match self {
            Self::Image { path, .. } => Some(path.as_path()),
            _ => None,
        }
    }

    /// 从磁盘解码图片，恢复写入剪贴板所需的 RGBA 像素。
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

    /// 比较两条记录的实际内容，供去重与粘贴抑制使用。
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

/// 先比较图片元数据，再按需比较缓存文件内容以识别重复图片。
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

/// 压缩连续空白并按字符数截断文本，超长时添加省略号。
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
    /// 验证超过数量上限时返回被淘汰图片的清理路径。
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
    /// 验证连续复制相同文本时保留原条目且无需清理文件。
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
    /// 验证重复图片被丢弃时返回新缓存路径供清理。
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
    /// 验证失效图片记录被移除，并返回对应路径。
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
    /// 验证缩减历史数量时收集淘汰条目的图片路径。
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

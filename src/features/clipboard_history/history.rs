use std::collections::VecDeque;
use std::path::PathBuf;

use arboard::ImageData;

const MAX_HISTORY_ITEMS: usize = 20;

#[derive(Debug, Clone)]
pub enum ClipboardHistoryItem {
    Text {
        text: String,
    },
    Image {
        width: usize,
        height: usize,
        bytes: Vec<u8>,
    },
    Files {
        paths: Vec<PathBuf>,
    },
}

#[derive(Debug, Default)]
pub struct ClipboardHistory {
    items: VecDeque<ClipboardHistoryItem>,
}

impl ClipboardHistory {
    pub fn push(&mut self, item: ClipboardHistoryItem) {
        if self
            .items
            .front()
            .is_some_and(|latest| latest.same_content(&item))
        {
            return;
        }

        self.items.push_front(item);
        while self.items.len() > MAX_HISTORY_ITEMS {
            self.items.pop_back();
        }
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
}

impl ClipboardHistoryItem {
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

    pub fn detail(&self) -> String {
        match self {
            Self::Text { text } => summarize_text(text, 180),
            Self::Image {
                width,
                height,
                bytes,
            } => {
                format!(
                    "RGBA 图片，{} KB，原始尺寸 {width} x {height}",
                    bytes.len() / 1024
                )
            }
            Self::Files { paths } => paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    }

    pub fn full_text(&self) -> String {
        match self {
            Self::Text { text } => text.clone(),
            Self::Image { .. } | Self::Files { .. } => self.detail(),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Text { .. } => "text",
            Self::Image { .. } => "image",
            Self::Files { .. } => "file",
        }
    }

    pub fn image_data(&self) -> Option<ImageData<'static>> {
        match self {
            Self::Image {
                width,
                height,
                bytes,
            } => Some(ImageData {
                width: *width,
                height: *height,
                bytes: bytes.clone().into(),
            }),
            _ => None,
        }
    }

    fn same_content(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Text { text: left }, Self::Text { text: right }) => left == right,
            (
                Self::Image {
                    width: left_width,
                    height: left_height,
                    bytes: left_bytes,
                },
                Self::Image {
                    width: right_width,
                    height: right_height,
                    bytes: right_bytes,
                },
            ) => {
                left_width == right_width
                    && left_height == right_height
                    && left_bytes == right_bytes
            }
            (Self::Files { paths: left }, Self::Files { paths: right }) => left == right,
            _ => false,
        }
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

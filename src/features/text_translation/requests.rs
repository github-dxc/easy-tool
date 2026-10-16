//! 翻译请求代次：阻止过期的防抖任务和 API 结果覆盖当前输入。

use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Default)]
pub struct TranslationRequests {
    generation: AtomicU64,
}

impl TranslationRequests {
    /// 为一次新输入或请求分配代次，旧任务随即失效。
    pub fn begin(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// 关闭窗口、清空输入或停用功能时，使已有任务全部失效。
    pub fn cancel(&self) {
        self.begin();
    }

    /// 在发起请求和更新 UI 前检查任务是否仍然有效。
    pub fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证新输入会使旧防抖任务和旧请求结果失效。
    #[test]
    fn new_input_supersedes_previous_request() {
        let requests = TranslationRequests::default();
        let old = requests.begin();
        let current = requests.begin();
        assert!(!requests.is_current(old));
        assert!(requests.is_current(current));
    }

    /// 验证窗口关闭后返回的 API 结果不会再次激活窗口。
    #[test]
    fn cancellation_invalidates_pending_result() {
        let requests = TranslationRequests::default();
        let pending = requests.begin();
        requests.cancel();
        assert!(!requests.is_current(pending));
    }
}

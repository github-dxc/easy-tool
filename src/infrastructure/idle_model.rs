//! Lazy model holder with an idle unload deadline.

use std::time::{Duration, Instant};

const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

pub struct IdleModel<T> {
    model: Option<T>,
    idle_timeout: Duration,
    idle_deadline: Option<Instant>,
}

impl<T> IdleModel<T> {
    pub fn empty() -> Self {
        Self::with_idle_timeout(DEFAULT_IDLE_TIMEOUT)
    }

    pub fn with_idle_timeout(idle_timeout: Duration) -> Self {
        Self {
            model: None,
            idle_timeout,
            idle_deadline: None,
        }
    }

    pub fn get_or_try_load(
        &mut self,
        load: impl FnOnce() -> Result<T, String>,
    ) -> Result<&mut T, String> {
        if self.model.is_none() {
            self.model = Some(load()?);
        }

        self.model
            .as_mut()
            .ok_or_else(|| "model is not loaded".to_string())
    }

    pub fn refresh_idle_deadline(&mut self, now: Instant) {
        if self.model.is_some() {
            self.idle_deadline = Some(now + self.idle_timeout);
        }
    }

    pub fn unload_if_idle(&mut self, now: Instant) -> bool {
        if self.model.is_some() && self.idle_deadline.is_some_and(|deadline| now >= deadline) {
            self.unload_now();
            return true;
        }

        false
    }

    pub fn unload_now(&mut self) {
        self.model = None;
        self.idle_deadline = None;
    }

    #[cfg(test)]
    pub fn is_loaded(&self) -> bool {
        self.model.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::IdleModel;
    use std::time::{Duration, Instant};

    #[test]
    fn starts_without_loading_model() {
        let model = IdleModel::<usize>::with_idle_timeout(Duration::from_secs(60));

        assert!(!model.is_loaded());
    }

    #[test]
    fn loads_once_and_reuses_loaded_model() {
        let mut model = IdleModel::with_idle_timeout(Duration::from_secs(60));
        let mut load_count = 0;

        *model
            .get_or_try_load(|| {
                load_count += 1;
                Ok::<_, String>(41usize)
            })
            .unwrap() += 1;
        let value = *model
            .get_or_try_load(|| {
                load_count += 1;
                Ok::<_, String>(0usize)
            })
            .unwrap();

        assert_eq!(load_count, 1);
        assert_eq!(value, 42);
    }

    #[test]
    fn unloads_only_after_refreshed_idle_deadline_expires() {
        let mut model = IdleModel::with_idle_timeout(Duration::from_secs(60));
        let now = Instant::now();
        model.get_or_try_load(|| Ok::<_, String>(7usize)).unwrap();
        model.refresh_idle_deadline(now);

        assert!(!model.unload_if_idle(now + Duration::from_secs(59)));
        assert!(model.is_loaded());
        assert!(model.unload_if_idle(now + Duration::from_secs(60)));
        assert!(!model.is_loaded());
    }

    #[test]
    fn refresh_extends_idle_lifetime() {
        let mut model = IdleModel::with_idle_timeout(Duration::from_secs(60));
        let now = Instant::now();
        model.get_or_try_load(|| Ok::<_, String>(7usize)).unwrap();
        model.refresh_idle_deadline(now);
        model.refresh_idle_deadline(now + Duration::from_secs(30));

        assert!(!model.unload_if_idle(now + Duration::from_secs(89)));
        assert!(model.unload_if_idle(now + Duration::from_secs(90)));
    }

    #[test]
    fn unload_now_drops_loaded_model() {
        let mut model = IdleModel::with_idle_timeout(Duration::from_secs(60));
        model.get_or_try_load(|| Ok::<_, String>(7usize)).unwrap();

        model.unload_now();

        assert!(!model.is_loaded());
    }

    #[test]
    fn active_model_borrow_prevents_unload_until_borrow_ends() {
        let mut model = IdleModel::with_idle_timeout(Duration::from_secs(60));
        let now = Instant::now();
        let borrowed = model.get_or_try_load(|| Ok::<_, String>(7usize)).unwrap();
        *borrowed += 1;

        assert_eq!(*borrowed, 8);

        model.refresh_idle_deadline(now);
        assert!(model.unload_if_idle(now + Duration::from_secs(60)));
    }
}

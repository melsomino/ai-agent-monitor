//! Actor-owned delayed work; a failed item must not block unrelated ready items.
use std::{collections::VecDeque, time::Instant};

pub struct Queue<T> {
    items: VecDeque<(Instant, T)>,
}
impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            items: VecDeque::new(),
        }
    }
}
impl<T> Queue<T> {
    pub fn push(&mut self, item: T, due: Instant) {
        self.items.push_back((due, item));
    }
    pub fn pop_ready(&mut self, now: Instant) -> Option<T> {
        let index = self.items.iter().position(|(due, _)| *due <= now)?;
        self.items.remove(index).map(|(_, item)| item)
    }
    pub fn next_due(&self) -> Option<Instant> {
        self.items.iter().map(|(due, _)| *due).min()
    }
    pub fn retain(&mut self, mut keep: impl FnMut(&T) -> bool) {
        self.items.retain(|(_, item)| keep(item));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[test]
    fn delayed_retry_does_not_block_new_work_and_can_be_cancelled() {
        let now = Instant::now();
        let mut queue = Queue::default();
        queue.push("retry", now + Duration::from_secs(15));
        queue.push("new", now);
        assert_eq!(queue.pop_ready(now), Some("new"));
        assert_eq!(queue.pop_ready(now), None);
        assert_eq!(queue.next_due(), Some(now + Duration::from_secs(15)));
        queue.retain(|item| *item != "retry");
        assert_eq!(queue.pop_ready(now + Duration::from_secs(20)), None);
        assert_eq!(queue.next_due(), None);
    }
}

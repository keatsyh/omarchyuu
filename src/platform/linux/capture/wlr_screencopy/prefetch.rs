//! One picture of lookahead, with an epoch barrier for forced snapshots.
use anyhow::{Result, bail};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

struct State<T> {
    picture: Option<T>,
    generation: u64,
    error: Option<String>,
    stopped: bool,
}
pub(super) struct Slot<T> {
    state: Mutex<State<T>>,
    changed: Condvar,
}
/// Unwinding a capture thread must surface an error, not leave its consumer
/// timing out forever with the last desktop image.
pub(super) struct Producer<T>(Arc<Slot<T>>);
impl<T> Drop for Producer<T> {
    fn drop(&mut self) {
        let mut state = self.0.lock();
        if !state.stopped && state.error.is_none() {
            state.error = Some("Wayland 采集线程提前退出".into());
        }
        self.0.changed.notify_all();
    }
}
impl<T> Slot<T> {
    pub fn producer(self: &Arc<Self>) -> Producer<T> {
        Producer(self.clone())
    }
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                picture: None,
                generation: 0,
                error: None,
                stopped: false,
            }),
            changed: Condvar::new(),
        }
    }
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub fn demand(&self) -> Option<u64> {
        let mut state = self.lock();
        while !state.stopped && state.error.is_none() && state.picture.is_some() {
            state = self.changed.wait(state).unwrap_or_else(|p| p.into_inner());
        }
        (!state.stopped && state.error.is_none()).then_some(state.generation)
    }
    pub fn publish(&self, generation: u64, result: Result<Option<T>>) -> bool {
        let mut state = self.lock();
        if state.stopped {
            return false;
        }
        // Resize may race a frame already being copied. Neither that picture
        // nor its error is allowed to cross the requested snapshot barrier.
        if generation != state.generation {
            return true;
        }
        match result {
            Ok(picture) => state.picture = picture,
            Err(error) => state.error = Some(format!("{error:#}")),
        }
        self.changed.notify_all();
        state.error.is_none()
    }
    pub fn invalidate(&self) {
        let mut state = self.lock();
        state.generation = state.generation.wrapping_add(1);
        state.picture = None;
        self.changed.notify_all();
    }
    pub fn stop(&self) {
        self.lock().stopped = true;
        self.changed.notify_all();
    }
    pub fn take(&self, timeout: Duration) -> Result<Option<T>> {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        loop {
            if let Some(error) = &state.error {
                bail!("{error}");
            }
            if let Some(picture) = state.picture.take() {
                self.changed.notify_all();
                return Ok(Some(picture));
            }
            if state.stopped {
                bail!("Wayland 采集已停止");
            }
            let now = Instant::now();
            if now >= deadline {
                return Ok(None);
            }
            state = self
                .changed
                .wait_timeout(state, deadline - now)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn snapshot_discards_ready_and_in_flight_old_pictures() {
        let slot = Slot::new();
        let old = slot.demand().unwrap();
        slot.publish(old, Ok(Some((2560, 1600))));
        slot.invalidate();
        // A late completion, including an old failure, must not replace the
        // first frame of the newly requested size.
        slot.publish(old, Ok(Some((2560, 1600))));
        slot.publish(old, Err(anyhow::anyhow!("old copy cancelled")));
        assert_eq!(slot.take(Duration::ZERO).unwrap(), None);
        let new = slot.demand().unwrap();
        slot.publish(new, Ok(Some((768, 480))));
        assert_eq!(slot.take(Duration::ZERO).unwrap(), Some((768, 480)));
    }

    #[test]
    fn producer_is_bounded_and_shutdown_wakes_it() {
        let slot = Arc::new(Slot::new());
        slot.publish(0, Ok(Some(1)));
        let producer = slot.clone();
        let (sent, received) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            sent.send(producer.demand()).unwrap();
        });
        assert!(received.try_recv().is_err());
        slot.stop();
        assert_eq!(received.recv_timeout(Duration::from_secs(1)).unwrap(), None);
        thread.join().unwrap();
    }

    #[test]
    fn copy_failure_remains_visible_on_every_read() {
        let slot = Slot::<()>::new();
        assert!(!slot.publish(0, Err(anyhow::anyhow!("copy refused"))));
        for _ in 0..2 {
            assert!(
                slot.take(Duration::ZERO)
                    .unwrap_err()
                    .to_string()
                    .contains("copy refused")
            );
        }
    }

    #[test]
    fn unexpected_producer_exit_is_reported_instead_of_timing_out() {
        let slot = Arc::new(Slot::<()>::new());
        drop(slot.producer());
        assert!(
            slot.take(Duration::ZERO)
                .unwrap_err()
                .to_string()
                .contains("提前退出")
        );
    }
}

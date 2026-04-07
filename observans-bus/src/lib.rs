use std::sync::{Arc, Condvar, Mutex};
use tokio::sync::broadcast;

pub type Frame = Vec<u8>;
pub type FrameSender = broadcast::Sender<Frame>;
pub type FrameReceiver = broadcast::Receiver<Frame>;

pub fn create_bus(capacity: usize) -> (FrameSender, FrameReceiver) {
    broadcast::channel(capacity)
}

/// Coordinates whether the capture pipeline should be active.
///
/// The capture thread calls [`wait_for_clients`](Self::wait_for_clients) and
/// parks until at least one viewer connects.  When the last viewer leaves the
/// ongoing ffmpeg process is killed and the thread returns to the parked state.
pub struct ClientGate {
    count: Mutex<usize>,
    has_clients: Condvar,
}

impl ClientGate {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            count: Mutex::new(0),
            has_clients: Condvar::new(),
        })
    }

    /// Increment the viewer count and wake any parked capture threads.
    ///
    /// Returns the new count.
    pub fn add_client(&self) -> usize {
        let mut count = self.count.lock().unwrap();
        *count += 1;
        self.has_clients.notify_all();
        *count
    }

    /// Decrement the viewer count (saturating at zero).
    ///
    /// Returns the new count.
    pub fn remove_client(&self) -> usize {
        let mut count = self.count.lock().unwrap();
        *count = count.saturating_sub(1);
        *count
    }

    /// Current number of connected stream viewers.
    pub fn client_count(&self) -> usize {
        *self.count.lock().unwrap()
    }

    /// Block the calling thread until at least one viewer is connected.
    pub fn wait_for_clients(&self) {
        let mut count = self.count.lock().unwrap();
        while *count == 0 {
            count = self.has_clients.wait(count).unwrap();
        }
    }
}

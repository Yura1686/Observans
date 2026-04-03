use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

#[derive(Clone, Debug)]
pub struct Shutdown {
    inner: Arc<ShutdownInner>,
}

#[derive(Debug)]
struct ShutdownInner {
    triggered: AtomicBool,
    notify: Notify,
}

impl Shutdown {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ShutdownInner {
                triggered: AtomicBool::new(false),
                notify: Notify::new(),
            }),
        }
    }

    pub fn trigger(&self) -> bool {
        let was_triggered = self.inner.triggered.swap(true, Ordering::SeqCst);
        if !was_triggered {
            self.inner.notify.notify_waiters();
        }
        !was_triggered
    }

    pub fn is_triggered(&self) -> bool {
        self.inner.triggered.load(Ordering::SeqCst)
    }

    pub async fn wait(&self) {
        if self.is_triggered() {
            return;
        }

        loop {
            let notified = self.inner.notify.notified();
            if self.is_triggered() {
                return;
            }
            notified.await;
            if self.is_triggered() {
                return;
            }
        }
    }
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

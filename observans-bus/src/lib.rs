use tokio::sync::broadcast;

pub type Frame = Vec<u8>;
pub type FrameSender = broadcast::Sender<Frame>;
pub type FrameReceiver = broadcast::Receiver<Frame>;

pub fn create_bus(capacity: usize) -> (FrameSender, FrameReceiver) {
    broadcast::channel(capacity)
}

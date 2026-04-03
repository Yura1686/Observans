use crate::AppState;
use async_stream::stream;
use axum::body::Body;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::response::Response;
use bytes::Bytes;
use std::convert::Infallible;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{timeout, Duration};

pub async fn mjpeg_handler(State(state): State<AppState>) -> Response {
    let mut rx = state.tx.subscribe();
    let stream_state = state.clone();

    let body_stream = stream! {
        let _guard = ClientGuard::new(stream_state.clone());
        loop {
            // Use a timeout so that if no frames arrive (e.g. camera stalled),
            // the stream does not block forever. More importantly: axum detects
            // a gone client only when it tries to write. By sending a keepalive
            // comment (which is valid in multipart streams) we force a write
            // attempt every second even during frame gaps, so ClientGuard::drop
            // fires promptly and gate.client_count() returns to zero.
            match timeout(Duration::from_secs(1), rx.recv()).await {
                Ok(Ok(jpeg)) => {
                    yield Ok::<Bytes, Infallible>(build_chunk(&jpeg));
                }
                Ok(Err(RecvError::Lagged(skipped))) => {
                    stream_state.metrics.note_queue_drop(skipped);
                }
                Ok(Err(RecvError::Closed)) => break,
                Err(_elapsed) => {
                    // No frame arrived within 1 s. Send an empty MJPEG comment
                    // to force a write — this makes axum detect the closed
                    // connection and drop the ClientGuard.
                    yield Ok::<Bytes, Infallible>(keepalive_chunk());
                }
            }
        }
    };

    Response::builder()
        .header(CONTENT_TYPE, "multipart/x-mixed-replace; boundary=frame")
        .body(Body::from_stream(body_stream))
        .expect("valid MJPEG response")
}

pub fn build_chunk(jpeg: &[u8]) -> Bytes {
    let mut chunk = format!(
        "--frame\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
        jpeg.len()
    )
    .into_bytes();
    chunk.extend_from_slice(jpeg);
    chunk.extend_from_slice(b"\r\n");
    Bytes::from(chunk)
}

/// A minimal MJPEG keepalive: an empty part with no Content-Type.
/// Browsers ignore unknown parts; the only purpose is to provoke a write()
/// syscall so the OS can report that the client TCP connection is gone.
fn keepalive_chunk() -> Bytes {
    Bytes::from_static(b"--frame\r\n\r\n")
}

struct ClientGuard {
    state: AppState,
}

impl ClientGuard {
    fn new(state: AppState) -> Self {
        state.client_connected();
        Self { state }
    }
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        self.state.client_disconnected();
    }
}

#[cfg(test)]
mod tests {
    use super::build_chunk;

    #[test]
    fn formats_valid_mjpeg_chunk() {
        let chunk = build_chunk(&[0xFF, 0xD8, 0xFF, 0xD9]);
        let text = String::from_utf8_lossy(chunk.as_ref());

        assert!(text.starts_with("--frame\r\nContent-Type: image/jpeg\r\n"));
        assert!(chunk.ends_with(b"\r\n"));
    }
}

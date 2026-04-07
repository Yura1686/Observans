use crate::AppState;
use async_stream::stream;
use axum::body::Body;
use axum::extract::{ConnectInfo, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Extension;
use bytes::Bytes;
use observans_bus::FrameReceiver;
use observans_core::ListenerKind;
use std::convert::Infallible;
use std::net::SocketAddr;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::time::{timeout, Duration};

pub async fn mjpeg_handler(
    State(state): State<AppState>,
    Extension(listener_kind): Extension<ListenerKind>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Result<Response, StatusCode> {
    crate::authorize_request(&state, listener_kind, peer)?;
    let mut rx = state.tx.subscribe();
    let stream_state = state.clone();
    let mut lan_policy = state.network.subscribe_lan();

    let body_stream = stream! {
        let _guard = ClientGuard::new(stream_state.clone());
        loop {
            match next_stream_step(&mut rx, &stream_state, listener_kind, &mut lan_policy).await {
                StreamStep::Yield(chunk) => yield Ok::<Bytes, Infallible>(chunk),
                StreamStep::Continue => continue,
                StreamStep::Break => break,
            }
        }
    };

    Ok(Response::builder()
        .header(CONTENT_TYPE, "multipart/x-mixed-replace; boundary=frame")
        .body(Body::from_stream(body_stream))
        .expect("valid MJPEG response"))
}

enum StreamStep {
    Yield(Bytes),
    Continue,
    Break,
}

async fn next_stream_step(
    rx: &mut FrameReceiver,
    state: &AppState,
    listener_kind: ListenerKind,
    lan_policy: &mut watch::Receiver<bool>,
) -> StreamStep {
    if listener_kind == ListenerKind::Lan && !*lan_policy.borrow() {
        return StreamStep::Break;
    }

    let recv = timeout(Duration::from_secs(1), rx.recv());
    let result = if listener_kind == ListenerKind::Lan {
        tokio::select! {
            changed = lan_policy.changed() => {
                if changed.is_err() || !*lan_policy.borrow() {
                    return StreamStep::Break;
                }
                return StreamStep::Continue;
            }
            result = recv => result,
        }
    } else {
        recv.await
    };

    match result {
        Ok(Ok(jpeg)) => StreamStep::Yield(build_chunk(&jpeg)),
        Ok(Err(RecvError::Lagged(skipped))) => {
            state.metrics.note_queue_drop(skipped);
            StreamStep::Continue
        }
        Ok(Err(RecvError::Closed)) => StreamStep::Break,
        Err(_elapsed) => {
            // No frame arrived within 1 s. Send an empty MJPEG comment
            // to force a write — this makes axum detect the closed
            // connection and drop the ClientGuard.
            StreamStep::Yield(keepalive_chunk())
        }
    }
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

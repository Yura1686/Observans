pub mod stream;
pub mod ui;

use anyhow::Result;
use axum::extract::State;
use axum::response::Html;
use axum::routing::get;
use axum::{Json, Router};
use observans_bus::{ClientGate, FrameSender};
use observans_core::{Config, SharedMetrics};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub tx: FrameSender,
    pub metrics: SharedMetrics,
    pub gate: Arc<ClientGate>,
    pub config: Config,
}

impl AppState {
    pub fn new(
        tx: FrameSender,
        metrics: SharedMetrics,
        config: Config,
        gate: Arc<ClientGate>,
    ) -> Self {
        Self { tx, metrics, gate, config }
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.config.bind_addr()
    }

    /// Called when a viewer opens the MJPEG stream.
    /// Increments the gate counter — this wakes the capture thread if it was parked.
    pub fn client_connected(&self) {
        let count = self.gate.add_client();
        self.metrics.set_clients(count);
    }

    /// Called when a viewer closes the MJPEG stream.
    /// Decrements the gate counter — when it reaches zero the capture thread
    /// kills ffmpeg and parks until the next viewer arrives.
    pub fn client_disconnected(&self) {
        let count = self.gate.remove_client();
        self.metrics.set_clients(count);
    }
}

pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/metrics", get(metrics))
        .route("/stream", get(stream::mjpeg_handler))
        .with_state(state)
}

pub async fn serve(addr: SocketAddr, state: AppState) -> Result<()> {
    let router = app(state);
    let listener = TcpListener::bind(addr).await?;
    info!(
        "observans web listening on http://{}",
        listener.local_addr()?
    );

    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn root() -> Html<String> {
    Html(ui::render_index_html())
}

async fn metrics(State(state): State<AppState>) -> Json<observans_core::MetricsSnapshot> {
    Json(state.metrics.snapshot())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
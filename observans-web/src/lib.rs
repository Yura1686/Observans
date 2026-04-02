pub mod stream;
pub mod ui;

use anyhow::Result;
use axum::extract::State;
use axum::response::Html;
use axum::routing::get;
use axum::{Json, Router};
use observans_bus::FrameSender;
use observans_core::{Config, SharedMetrics};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::info;

#[derive(Clone)]
pub struct AppState {
    pub tx: FrameSender,
    pub metrics: SharedMetrics,
    pub client_count: Arc<AtomicUsize>,
    pub config: Config,
}

impl AppState {
    pub fn new(tx: FrameSender, metrics: SharedMetrics, config: Config) -> Self {
        Self {
            tx,
            metrics,
            client_count: Arc::new(AtomicUsize::new(0)),
            config,
        }
    }

    pub fn bind_addr(&self) -> SocketAddr {
        self.config.bind_addr()
    }

    pub fn client_connected(&self) {
        let count = self.client_count.fetch_add(1, Ordering::Relaxed) + 1;
        self.metrics.set_clients(count);
    }

    pub fn client_disconnected(&self) {
        let previous = self.client_count.fetch_sub(1, Ordering::Relaxed);
        let next = previous.saturating_sub(1);
        self.metrics.set_clients(next);
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

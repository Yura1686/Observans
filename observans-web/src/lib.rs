pub mod stream;
pub mod ui;

use anyhow::{Context, Result};
use axum::extract::{ConnectInfo, State};
use axum::http::StatusCode;
use axum::response::Html;
use axum::routing::get;
use axum::{Extension, Json, Router};
use observans_bus::{ClientGate, FrameSender};
use observans_core::network::peer_allowed;
use observans_core::{
    discover_desired_bindings, Config, ListenerBinding, ListenerKind, SharedMetrics,
    SharedNetworkPolicy, Shutdown,
};
use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tracing::{info, warn};

#[derive(Clone)]
pub struct AppState {
    pub tx: FrameSender,
    pub metrics: SharedMetrics,
    pub gate: Arc<ClientGate>,
    pub config: Config,
    pub network: SharedNetworkPolicy,
}

impl AppState {
    pub fn new(
        tx: FrameSender,
        metrics: SharedMetrics,
        config: Config,
        gate: Arc<ClientGate>,
        network: SharedNetworkPolicy,
    ) -> Self {
        Self {
            tx,
            metrics,
            gate,
            config,
            network,
        }
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
    app_for_listener(state, ListenerKind::Loopback)
}

pub fn app_for_listener(state: AppState, listener_kind: ListenerKind) -> Router {
    Router::new()
        .route("/", get(root))
        .route("/metrics", get(metrics))
        .route("/stream", get(stream::mjpeg_handler))
        .layer(Extension(listener_kind))
        .with_state(state)
}

pub async fn serve(state: AppState, shutdown: Shutdown) -> Result<()> {
    let mut lan_rx = state.network.subscribe_lan();
    let mut listeners = HashMap::<ListenerBinding, ListenerHandle>::new();
    reconcile_listeners(&state, &mut listeners).await?;

    loop {
        tokio::select! {
            _ = shutdown.wait() => break,
            changed = lan_rx.changed() => {
                if changed.is_err() {
                    break;
                }
                reconcile_listeners(&state, &mut listeners).await?;
            }
        }
    }

    state.network.set_active_bindings(Vec::new());
    for handle in listeners.into_values() {
        handle.stop().await;
    }

    Ok(())
}

pub(crate) fn authorize_request(
    state: &AppState,
    listener_kind: ListenerKind,
    peer: SocketAddr,
) -> Result<(), StatusCode> {
    if peer_allowed(listener_kind, peer.ip(), state.network.lan_enabled()) {
        Ok(())
    } else {
        warn!(
            listener = listener_kind.label(),
            peer = %peer,
            "rejecting request outside allowed network scope"
        );
        Err(StatusCode::FORBIDDEN)
    }
}

async fn reconcile_listeners(
    state: &AppState,
    listeners: &mut HashMap<ListenerBinding, ListenerHandle>,
) -> Result<()> {
    let desired = discover_desired_bindings(&state.network);
    let desired_set = desired.iter().cloned().collect::<HashSet<_>>();

    let stale_bindings = listeners
        .keys()
        .filter(|binding| !desired_set.contains(*binding))
        .cloned()
        .collect::<Vec<_>>();
    for binding in stale_bindings {
        if let Some(handle) = listeners.remove(&binding) {
            info!(
                listener = binding.kind.label(),
                addr = %binding.addr,
                "stopping listener"
            );
            handle.stop().await;
        }
    }

    for binding in desired {
        if listeners.contains_key(&binding) {
            continue;
        }

        if let Some(handle) = start_listener(state.clone(), binding).await? {
            listeners.insert(handle.binding.clone(), handle);
        }
    }

    state
        .network
        .set_active_bindings(listeners.keys().cloned().collect());
    Ok(())
}

async fn start_listener(
    state: AppState,
    binding: ListenerBinding,
) -> Result<Option<ListenerHandle>> {
    let listener = match TcpListener::bind(binding.addr).await {
        Ok(listener) => listener,
        Err(error) if binding.kind == ListenerKind::Loopback => {
            return Err(error).with_context(|| {
                format!(
                    "failed to bind required {} listener on {}",
                    binding.kind.label(),
                    binding.addr
                )
            });
        }
        Err(error) => {
            warn!(
                listener = binding.kind.label(),
                addr = %binding.addr,
                error = %error,
                "failed to bind optional listener"
            );
            return Ok(None);
        }
    };

    let binding = ListenerBinding {
        kind: binding.kind,
        addr: listener
            .local_addr()
            .context("failed to resolve listener address")?,
    };
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let router =
        app_for_listener(state, binding.kind).into_make_service_with_connect_info::<SocketAddr>();
    info!(
        listener = binding.kind.label(),
        addr = %binding.addr,
        "observans web listening"
    );

    let task = tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
        {
            warn!(
                listener = binding.kind.label(),
                addr = %binding.addr,
                error = %error,
                "listener exited with error"
            );
        }
    });

    Ok(Some(ListenerHandle {
        binding,
        shutdown_tx: Some(shutdown_tx),
        task,
    }))
}

async fn root(
    State(state): State<AppState>,
    Extension(listener_kind): Extension<ListenerKind>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Result<Html<String>, StatusCode> {
    authorize_request(&state, listener_kind, peer)?;
    Ok(Html(ui::render_index_html()))
}

async fn metrics(
    State(state): State<AppState>,
    Extension(listener_kind): Extension<ListenerKind>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Result<Json<observans_core::MetricsSnapshot>, StatusCode> {
    authorize_request(&state, listener_kind, peer)?;
    Ok(Json(state.metrics.snapshot()))
}

struct ListenerHandle {
    binding: ListenerBinding,
    shutdown_tx: Option<oneshot::Sender<()>>,
    task: JoinHandle<()>,
}

impl ListenerHandle {
    async fn stop(mut self) {
        if let Some(shutdown_tx) = self.shutdown_tx.take() {
            let _ = shutdown_tx.send(());
        }
        let _ = self.task.await;
    }
}

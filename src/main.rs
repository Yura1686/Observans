mod log_capture;

use anyhow::Result;
use observans_bus::{create_bus, ClientGate};
use observans_core::{
    spawn_dashboard, spawn_system_sampler, start_capture, terminal_is_interactive, Config,
    DashboardContext, LogLevel, SharedLogBuffer, SharedMetrics, Shutdown,
};
use observans_web::{serve, AppState};
use tracing::info;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

fn init_tracing(logs: SharedLogBuffer, render_console: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("observans=info,observans_core=info,observans_web=info")
    });

    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(log_capture::UiLogLayer::new(logs));

    if render_console {
        registry
            .with(
                tracing_subscriber::fmt::layer()
                    .with_target(false)
                    .compact(),
            )
            .init();
    } else {
        registry.init();
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let interactive = terminal_is_interactive();
    let config = Config::from_args_with_bootstrap(std::env::args())?;
    let logs = SharedLogBuffer::default();
    init_tracing(logs.clone(), !interactive);

    let shutdown = Shutdown::new();
    let metrics = SharedMetrics::new(&config);
    let (frame_tx, _frame_rx) = create_bus(4);
    logs.push(
        LogLevel::Ok,
        "CFG",
        format!("camera selection resolved to {}", config.device),
    );

    // Shared gate: the web layer signals viewer connect/disconnect;
    // the capture thread parks when the count is zero and wakes on the first viewer.
    let gate = ClientGate::new();

    let shutdown_signal = shutdown.clone();
    let logs_signal = logs.clone();
    let _signal_listener = tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        logs_signal.push(LogLevel::Wait, "SYS", "Ctrl+C received - shutting down");
        shutdown_signal.trigger();
    });

    let dashboard = spawn_dashboard(DashboardContext {
        config: config.clone(),
        metrics: metrics.clone(),
        logs: logs.clone(),
        shutdown: shutdown.clone(),
    });

    let _sampler = spawn_system_sampler(metrics.clone());
    let _capture = start_capture(
        config.clone(),
        frame_tx.clone(),
        metrics.clone(),
        gate.clone(),
    );

    let state = AppState::new(frame_tx, metrics, config, gate);
    info!("observans runtime initialised");
    let result = serve(state.bind_addr(), state, shutdown.clone()).await;
    shutdown.trigger();

    if let Some(handle) = dashboard {
        let _ = handle.join();
    }

    result
}

use anyhow::Result;
use observans_bus::{create_bus, ClientGate};
use observans_core::{spawn_system_sampler, start_capture, Config, SharedMetrics};
use observans_web::{serve, AppState};
use tracing_subscriber::EnvFilter;

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("observans=info,observans_core=info,observans_web=info")
    });

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

#[tokio::main]
async fn main() -> Result<()> {
    init_tracing();

    let config = Config::from_args_with_bootstrap(std::env::args())?;
    let metrics = SharedMetrics::new(&config);
    let (frame_tx, _frame_rx) = create_bus(4);

    // Shared gate: the web layer signals viewer connect/disconnect;
    // the capture thread parks when the count is zero and wakes on the first viewer.
    let gate = ClientGate::new();

    let _sampler = spawn_system_sampler(metrics.clone());
    let _capture = start_capture(
        config.clone(),
        frame_tx.clone(),
        metrics.clone(),
        gate.clone(),
    );

    let state = AppState::new(frame_tx, metrics, config, gate);
    serve(state.bind_addr(), state).await
}і
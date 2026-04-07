use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode};
use clap::Parser;
use futures_util::StreamExt;
use http_body_util::BodyExt;
use observans_bus::{create_bus, ClientGate, FrameSender};
use observans_core::{Config, ListenerKind, SharedMetrics, SharedNetworkPolicy};
use observans_web::{app, app_for_listener, AppState};
use tower::ServiceExt;

fn test_state(allow_lan: bool) -> (AppState, FrameSender) {
    let mut config = Config::try_parse_from(["observans"]).unwrap();
    config.allow_lan = allow_lan;
    let metrics = SharedMetrics::new(&config);
    let (tx, _rx) = create_bus(8);
    let gate = ClientGate::new();
    let network = SharedNetworkPolicy::new(config.port, allow_lan);
    (
        AppState::new(tx.clone(), metrics, config, gate, network),
        tx,
    )
}

fn request_with_peer(uri: &str, peer: [u8; 4]) -> Request<Body> {
    let mut request = Request::builder().uri(uri).body(Body::empty()).unwrap();
    request
        .extensions_mut()
        .insert(ConnectInfo(std::net::SocketAddr::from((peer, 9000))));
    request
}

#[tokio::test]
async fn root_page_keeps_old_ui_contract() {
    let (state, _tx) = test_state(false);
    let response = app(state)
        .oneshot(request_with_peer("/", [127, 0, 0, 1]))
        .await
        .unwrap();

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8(body.to_vec()).unwrap();

    assert!(html.contains("LIVE STREAM"));
    assert!(html.contains("id=\"record-btn\""));
    assert!(html.contains("id=\"stream-stage\""));
    assert!(html.contains("id=\"fullscreen-btn\""));
    assert!(html.contains("id=\"battery-fill\""));
}

#[tokio::test]
async fn metrics_route_returns_expected_keys() {
    let (state, _tx) = test_state(false);
    let response = app(state)
        .oneshot(request_with_peer("/metrics", [127, 0, 0, 1]))
        .await
        .unwrap();

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let payload = String::from_utf8(body.to_vec()).unwrap();

    assert!(payload.contains("\"cpu\""));
    assert!(payload.contains("\"ram_pct\""));
    assert!(payload.contains("\"stream_pipeline\""));
    assert!(payload.contains("\"batt_status\""));
}

#[tokio::test]
async fn stream_route_emits_mjpeg_boundary_chunks() {
    let (state, tx) = test_state(false);
    let response = app(state)
        .oneshot(request_with_peer("/stream", [127, 0, 0, 1]))
        .await
        .unwrap();

    assert_eq!(
        response.headers()["content-type"],
        "multipart/x-mixed-replace; boundary=frame"
    );

    tx.send(vec![0xFF, 0xD8, 0x01, 0x02, 0xFF, 0xD9]).unwrap();
    let mut body_stream = response.into_body().into_data_stream();
    let chunk = tokio::time::timeout(std::time::Duration::from_secs(1), body_stream.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    assert!(chunk.starts_with(b"--frame\r\nContent-Type: image/jpeg\r\n"));
    assert!(chunk.ends_with(b"\r\n"));
}

#[tokio::test]
async fn loopback_listener_rejects_lan_peers() {
    let (state, _tx) = test_state(false);
    let response = app_for_listener(state, ListenerKind::Loopback)
        .oneshot(request_with_peer("/", [192, 168, 1, 10]))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn lan_listener_rejects_requests_when_policy_is_off() {
    let (state, _tx) = test_state(false);
    let response = app_for_listener(state, ListenerKind::Lan)
        .oneshot(request_with_peer("/metrics", [192, 168, 1, 10]))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn lan_listener_accepts_private_peers_when_enabled() {
    let (state, _tx) = test_state(true);
    let response = app_for_listener(state, ListenerKind::Lan)
        .oneshot(request_with_peer("/metrics", [192, 168, 1, 10]))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn tailscale_listener_accepts_tailscale_peers() {
    let (state, _tx) = test_state(false);
    let response = app_for_listener(state, ListenerKind::Tailscale)
        .oneshot(request_with_peer("/metrics", [100, 100, 1, 10]))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn lan_stream_stops_after_runtime_toggle_off() {
    let (state, tx) = test_state(true);
    let network = state.network.clone();
    let response = app_for_listener(state, ListenerKind::Lan)
        .oneshot(request_with_peer("/stream", [192, 168, 1, 10]))
        .await
        .unwrap();

    tx.send(vec![0xFF, 0xD8, 0x01, 0x02, 0xFF, 0xD9]).unwrap();
    let mut body_stream = response.into_body().into_data_stream();
    let first = tokio::time::timeout(std::time::Duration::from_secs(1), body_stream.next())
        .await
        .unwrap();
    assert!(first.is_some());

    assert!(network.set_lan_enabled(false));

    let next = tokio::time::timeout(std::time::Duration::from_secs(1), body_stream.next())
        .await
        .unwrap();
    assert!(next.is_none());
}

use axum::body::Body;
use clap::Parser;
use futures_util::StreamExt;
use http_body_util::BodyExt;
use observans_bus::{create_bus, FrameSender};
use observans_core::{Config, SharedMetrics};
use observans_web::{app, AppState};
use tower::ServiceExt;

fn test_state() -> (AppState, FrameSender) {
    let config = Config::try_parse_from(["observans"]).unwrap();
    let metrics = SharedMetrics::new(&config);
    let (tx, _rx) = create_bus(8);
    (AppState::new(tx.clone(), metrics, config), tx)
}

#[tokio::test]
async fn root_page_keeps_old_ui_contract() {
    let (state, _tx) = test_state();
    let response = app(state)
        .oneshot(
            axum::http::Request::builder()
                .uri("/")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = response.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8(body.to_vec()).unwrap();

    assert!(html.contains("Capture Deck"));
    assert!(html.contains("id=\"record-btn\""));
    assert!(html.contains("id=\"cpu-graph\""));
    assert!(html.contains("id=\"stream-stage\""));
}

#[tokio::test]
async fn metrics_route_returns_expected_keys() {
    let (state, _tx) = test_state();
    let response = app(state)
        .oneshot(
            axum::http::Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
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
    let (state, tx) = test_state();
    let response = app(state)
        .oneshot(
            axum::http::Request::builder()
                .uri("/stream")
                .body(Body::empty())
                .unwrap(),
        )
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

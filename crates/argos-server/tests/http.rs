use argos_server::{config::Config, server, store::Store};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use std::collections::BTreeMap;
use tower::ServiceExt;
fn config() -> Config {
    Config {
        bind: "127.0.0.1:8080".parse().unwrap(),
        origin: "http://localhost:5173".into(),
        production: false,
        dev_auth: true,
        demo: true,
        endpoint: "tcp/127.0.0.1:7448".into(),
        prefix: "terra/rover".into(),
        database: "sqlite::memory:".into(),
        web_dir: "/nonexistent".into(),
        github_client: String::new(),
        github_secret: String::new(),
        allowed: BTreeMap::new(),
        tile_style: None,
    }
}
#[test]
fn development_auth_cannot_bind_publicly_or_control_real_hardware() {
    let mut c = config();
    assert!(c.validate().is_ok());
    c.bind = "0.0.0.0:8080".parse().unwrap();
    assert!(c.validate().is_err());
    c.bind = "127.0.0.1:8080".parse().unwrap();
    c.demo = false;
    assert!(c.validate().is_err());
    c.demo = true;
    c.production = true;
    assert!(c.validate().is_err());
}
#[tokio::test]
async fn private_api_requires_session_and_mutations_require_origin_and_csrf() {
    let store = Store::open("sqlite::memory:").await.unwrap();
    let state = server::AppState::new(config(), store.clone())
        .await
        .unwrap();
    let app = server::router(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/fleet")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/auth/dev")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/me")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/control")
                .header("cookie", &cookie)
                .header("x-argos-client", "tab")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let csrf: String = sqlx::query_scalar("SELECT csrf FROM sessions")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/control")
                .header("cookie", &cookie)
                .header("origin", "http://localhost:5173")
                .header("x-csrf-token", &csrf)
                .header("x-argos-client", "tab")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // Unknown endpoints do not expose a generic Zenoh action proxy.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/rovers/1/teleop")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_ne!(response.status(), StatusCode::OK);
}
#[tokio::test]
async fn restart_marks_unfinished_operations_and_preserves_request_identity() {
    use argos_server::{engine::Engine, fleet::DemoFleet, types::*};
    let store = Store::open("sqlite::memory:").await.unwrap();
    let mut e = Engine::new(DemoFleet::default());
    let user = Identity {
        id: "1".into(),
        login: "test".into(),
        role: "operator".into(),
        csrf: String::new(),
    };
    e.claim(&user, "tab", 0.).unwrap();
    let op = e
        .submit(
            &user,
            "tab",
            "1",
            WaypointRequest {
                run_id: "demo-run".into(),
                frame_id: Some("demo-run".into()),
                request_id: uuid::Uuid::new_v4().to_string(),
                waypoint: Waypoint::Local {
                    x: 2.,
                    y: 3.,
                    yaw: None,
                },
                arrival_radius: None,
            },
            0.,
        )
        .unwrap();
    store.record(&op).await.unwrap();
    let recovered = store.recover().await.unwrap();
    assert_eq!(recovered[0].request_id, op.request_id);
    assert_eq!(recovered[0].phase, "interrupted");
    assert!(store.recover().await.unwrap()[0].terminal());
}
#[tokio::test]
async fn viewer_expired_session_and_wrong_origin_cannot_mutate() {
    use argos_server::auth;
    let mut c = config();
    c.allowed.insert("2".into(), "viewer".into());
    let store = Store::open("sqlite::memory:").await.unwrap();
    let response = auth::session(&store, &c, "2", "viewer").await.unwrap();
    let cookie = response.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let csrf: String = sqlx::query_scalar("SELECT csrf FROM sessions WHERE user_id='2'")
        .fetch_one(&store.pool)
        .await
        .unwrap();
    let state = server::AppState::new(c, store.clone()).await.unwrap();
    let worker = state.worker.clone();
    let app = server::router(state);
    for origin in ["http://localhost:5173", "https://evil.invalid"] {
        let r = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/control")
                    .header("cookie", &cookie)
                    .header("origin", origin)
                    .header("x-csrf-token", &csrf)
                    .header("x-argos-client", "viewer-tab")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
    }
    sqlx::query("UPDATE sessions SET expires=0")
        .execute(&store.pool)
        .await
        .unwrap();
    let r = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/fleet")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    worker.stop();
}

#[tokio::test]
async fn hardware_route_requires_authentication() {
    let store = Store::open("sqlite::memory:").await.unwrap();
    let state = server::AppState::new(config(), store).await.unwrap();
    let app = server::router(state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/rovers/1/hardware")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"action":"arm","request_id":"bad","run_id":"r","revision":"1"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    state.worker.stop();
}
#[tokio::test]
async fn occupancy_route_requires_authentication() {
    let store = Store::open("sqlite::memory:").await.unwrap();
    let state = server::AppState::new(config(), store).await.unwrap();
    let response = server::router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/api/v1/rovers/1/occupancy")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    state.worker.stop();
}

#[tokio::test]
async fn drive_and_emergency_routes_require_authentication() {
    let store = Store::open("sqlite::memory:").await.unwrap();
    let state = server::AppState::new(config(), store).await.unwrap();
    for route in ["drive", "stop"] {
        let response=server::router(state.clone()).oneshot(Request::builder().method("POST").uri(format!("/api/v1/rovers/1/{route}")).header("content-type","application/json").body(Body::from(r#"{"run_id":"r","revision":"1","sequence":1,"observed_at":0,"linear":0.3,"angular":0}"#)).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    state.worker.stop();
}

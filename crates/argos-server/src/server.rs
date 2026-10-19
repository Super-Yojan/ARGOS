use crate::{
    auth,
    config::Config,
    engine::Fleet,
    fleet::{DemoFleet, LiveFleet},
    store::Store,
    types::*,
    worker::{Action, Worker},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use serde::Deserialize;
use std::{convert::Infallible, sync::Arc, time::Duration};
use tower_http::{
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub store: Store,
    pub worker: Worker,
    pub http: reqwest::Client,
}
impl AppState {
    pub async fn new(config: Config, store: Store) -> Result<Self, String> {
        config.validate()?;
        let fleet: Box<dyn Fleet> = if config.demo {
            Box::new(DemoFleet::default())
        } else {
            Box::new(LiveFleet::new(
                config.endpoint.clone(),
                config.prefix.clone(),
            ))
        };
        let worker = Worker::start(fleet, store.clone()).await?;
        Ok(Self {
            config: Arc::new(config),
            store,
            worker,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| e.to_string())?,
        })
    }
}
type ApiError = (StatusCode, Json<serde_json::Value>);
fn error(status: StatusCode, message: &str) -> ApiError {
    (status, Json(serde_json::json!({"error":message})))
}
async fn user(s: &AppState, h: &HeaderMap) -> Result<(Identity, i64), ApiError> {
    auth::identity(h, &s.store, &s.config)
        .await
        .map_err(|c| error(c, "Sign in with an invited account"))
}
fn client(h: &HeaderMap) -> Result<String, ApiError> {
    let tab = h
        .get("x-argos-client")
        .and_then(|s| s.to_str().ok())
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 64
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        .ok_or_else(|| error(StatusCode::BAD_REQUEST, "Browser client ID required"))?;
    Ok(tab.into())
}
fn owner(h: &HeaderMap, u: &Identity, tab: &str) -> String {
    format!(
        "{}:{}:{tab}",
        u.id,
        auth::hash(&auth::cookie(h, "argos_session").unwrap_or_default())
    )
}
fn mutation(s: &AppState, h: &HeaderMap, u: &Identity) -> Result<(), ApiError> {
    if h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) != Some(s.config.origin.as_str())
        || h.get("x-csrf-token").and_then(|v| v.to_str().ok()) != Some(u.csrf.as_str())
    {
        return Err(error(
            StatusCode::FORBIDDEN,
            "Origin and CSRF validation required",
        ));
    }
    Ok(())
}
async fn dispatch(
    s: &AppState,
    h: &HeaderMap,
    action: Action,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (u, _) = user(s, h).await?;
    mutation(s, h, &u)?;
    if u.role != "operator" {
        return Err(error(StatusCode::FORBIDDEN, "Operator role required"));
    }
    let tab = client(h)?;
    let owner = owner(h, &u, &tab);
    s.worker
        .request(u, owner, action)
        .await
        .map(Json)
        .map_err(|e| error(StatusCode::CONFLICT, &e))
}
pub fn router(state: AppState) -> Router {
    let files = ServeDir::new(&state.config.web_dir).not_found_service(ServeFile::new(format!(
        "{}/index.html",
        state.config.web_dir
    )));
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route(
            "/readyz",
            get(|State(s): State<AppState>| async move {
                if s.worker.snapshots.borrow().link == "healthy" {
                    StatusCode::OK
                } else {
                    StatusCode::SERVICE_UNAVAILABLE
                }
            }),
        )
        .route("/auth/github", get(login))
        .route("/auth/callback", get(callback))
        .route("/auth/dev", get(dev))
        .route("/api/v1/me", get(me))
        .route("/api/v1/logout", post(logout))
        .route("/api/v1/config", get(config))
        .route("/api/v1/fleet", get(snapshot))
        .route("/api/v1/events", get(events))
        .route("/api/v1/control", post(claim).delete(release))
        .route("/api/v1/rovers/{id}/waypoints", post(waypoint))
        .route("/api/v1/rovers/{id}/cancel", post(cancel))
        .route("/api/v1/rovers/{id}/drive", post(drive))
        .route("/api/v1/rovers/{id}/stop", post(emergency_stop))
        .route("/api/v1/rovers/{id}/hardware", post(hardware))
        .route("/api/v1/rovers/{id}/autonomy", post(autonomy))
        .route("/api/v1/rovers/{id}/occupancy", get(occupancy))
        .route("/api/v1/rovers/{id}/navigation", post(navigation))
        .route("/api/v1/rovers/{id}/navigation/source", post(source))
        .route("/api/v1/operations/{id}", get(operation))
        .route(
            "/api/{*path}",
            get(|| async { StatusCode::NOT_FOUND }).post(|| async { StatusCode::NOT_FOUND }),
        )
        .fallback_service(files)
        .layer(DefaultBodyLimit::max(4096))
        .layer(TraceLayer::new_for_http().make_span_with(|request: &axum::http::Request<Body>| tracing::info_span!("http",method=%request.method(),path=%request.uri().path())))
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}
async fn security_headers(req: axum::http::Request<Body>, next: Next) -> Response {
    let mut r = next.run(req).await;
    for (name, value) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("x-frame-options", "DENY"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; worker-src 'self' blob:; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob: https:; font-src 'self'; connect-src 'self' https:; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
        ),
    ] {
        r.headers_mut().insert(
            axum::http::HeaderName::from_static(name),
            axum::http::HeaderValue::from_static(value),
        );
    }
    r
}
async fn me(State(s): State<AppState>, h: HeaderMap) -> Result<Json<Identity>, ApiError> {
    Ok(Json(user(&s, &h).await?.0))
}
async fn config(State(s): State<AppState>, h: HeaderMap) -> Result<Json<PublicConfig>, ApiError> {
    user(&s, &h).await?;
    Ok(Json(PublicConfig {
        demo: s.config.demo,
        tile_style: s.config.tile_style.clone(),
    }))
}
async fn snapshot(
    State(s): State<AppState>,
    h: HeaderMap,
) -> Result<Json<FleetSnapshot>, ApiError> {
    user(&s, &h).await?;
    Ok(Json(s.worker.snapshots.borrow().clone()))
}
async fn claim(
    State(s): State<AppState>,
    h: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    dispatch(&s, &h, Action::Claim).await
}
async fn release(
    State(s): State<AppState>,
    h: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    dispatch(&s, &h, Action::Release).await
}
async fn waypoint(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(r): Json<WaypointRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    dispatch(&s, &h, Action::Submit(id, r)).await
}
async fn navigation(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(r): Json<NavigationRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    dispatch(&s, &h, Action::Navigation(id, r)).await
}
async fn source(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(r): Json<SourceRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    dispatch(&s, &h, Action::Source(id, r)).await
}
async fn occupancy(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (u, _) = user(&s, &h).await?;
    s.worker
        .request(u, String::new(), Action::Occupancy(id))
        .await
        .map(Json)
        .map_err(|e| error(StatusCode::SERVICE_UNAVAILABLE, &e))
}
async fn drive(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(r): Json<DriveRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    dispatch(&s, &h, Action::Drive(id, r, std::time::Instant::now())).await
}
async fn emergency_stop(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    dispatch(&s, &h, Action::EmergencyStop(id)).await
}
async fn autonomy(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(r): Json<AutonomyRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    dispatch(&s, &h, Action::Autonomy(id, r)).await
}
async fn hardware(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(r): Json<HardwareRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    dispatch(&s, &h, Action::Hardware(id, r)).await
}
async fn cancel(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(r): Json<CancelRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    dispatch(&s, &h, Action::Cancel(id, r.request_id)).await
}
async fn operation(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (u, _) = user(&s, &h).await?;
    s.worker
        .request(u, String::new(), Action::Operation(id))
        .await
        .map(Json)
        .map_err(|e| error(StatusCode::NOT_FOUND, &e))
}
struct StreamGuard {
    worker: Worker,
    user: Identity,
    owner: String,
}
impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.worker.release(self.user.clone(), self.owner.clone());
    }
}
#[derive(Deserialize)]
struct EventQuery {
    client: String,
}
async fn events(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<EventQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let (u, expires) = user(&s, &h).await?;
    if q.client.is_empty()
        || q.client.len() > 64
        || !q
            .client
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(error(StatusCode::BAD_REQUEST, "Invalid client ID"));
    }
    let owner = owner(&h, &u, &q.client);
    let mut rx = s.worker.snapshots.clone();
    let guard = StreamGuard {
        worker: s.worker.clone(),
        user: u,
        owner,
    };
    let stream = async_stream::stream! {
        let _guard=guard;
        let deadline=tokio::time::sleep(Duration::from_secs((expires-auth::epoch()).max(0) as u64));tokio::pin!(deadline);
        loop {
            if auth::identity(&h,&s.store,&s.config).await.is_err() {break;}
            let snap=rx.borrow_and_update().clone();
            yield Ok::<_,Infallible>(Event::default().event("fleet").json_data(snap).unwrap_or_else(|_|Event::default().event("error").data("Snapshot unavailable")));
            tokio::select! {_= &mut deadline=>break, changed=rx.changed()=>if changed.is_err(){break}}
        }
    };
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))))
}
async fn login(State(s): State<AppState>) -> Result<Response, StatusCode> {
    if s.config.dev_auth {
        return Err(StatusCode::NOT_FOUND);
    }
    auth::start(&s.store, &s.config).await
}
async fn dev(State(s): State<AppState>) -> Result<Response, StatusCode> {
    if !s.config.dev_auth {
        return Err(StatusCode::NOT_FOUND);
    }
    auth::session(&s.store, &s.config, "dev-operator", "Local developer").await
}
#[derive(Deserialize)]
struct Callback {
    code: String,
    state: String,
}
async fn callback(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<Callback>,
) -> Result<Response, StatusCode> {
    auth::callback(&s.store, &s.config, &s.http, &h, &q.code, &q.state).await
}
async fn logout(State(s): State<AppState>, h: HeaderMap) -> Result<Response, ApiError> {
    let (u, _) = user(&s, &h).await?;
    mutation(&s, &h, &u)?;
    if let Ok(tab) = client(&h) {
        let owner = owner(&h, &u, &tab);
        s.worker.release(u, owner);
    }
    if let Some(token) = auth::cookie(&h, "argos_session") {
        sqlx::query("DELETE FROM sessions WHERE token_hash=?")
            .bind(auth::hash(&token))
            .execute(&s.store.pool)
            .await
            .map_err(|_| error(StatusCode::SERVICE_UNAVAILABLE, "Session store unavailable"))?;
    }
    let mut r = StatusCode::NO_CONTENT.into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        auth::set_cookie("argos_session", "", 0, &s.config)
            .parse()
            .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Cookie unavailable"))?,
    );
    Ok(r)
}

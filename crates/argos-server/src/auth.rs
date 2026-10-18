use crate::{config::Config, store::Store, types::Identity};
use axum::{
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
pub fn secret() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}
pub fn hash(s: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(s.as_bytes()))
}
pub fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|p| {
            let (k, v) = p.trim().split_once('=')?;
            (k == name).then(|| v.to_string())
        })
}
pub fn set_cookie(name: &str, value: &str, age: i64, config: &Config) -> String {
    format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={age}{}",
        if config.dev_auth { "" } else { "; Secure" }
    )
}
pub async fn identity(
    headers: &HeaderMap,
    store: &Store,
    config: &Config,
) -> Result<(Identity, i64), StatusCode> {
    let token = cookie(headers, "argos_session").ok_or(StatusCode::UNAUTHORIZED)?;
    let row = sqlx::query(
        "SELECT user_id,login,csrf,expires FROM sessions WHERE token_hash=? AND expires>?",
    )
    .bind(hash(&token))
    .bind(epoch())
    .fetch_optional(&store.pool)
    .await
    .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
    .ok_or(StatusCode::UNAUTHORIZED)?;
    let id: String = row.get("user_id");
    let role = config.role(&id).ok_or(StatusCode::FORBIDDEN)?;
    Ok((
        Identity {
            id,
            login: row.get("login"),
            csrf: row.get("csrf"),
            role,
        },
        row.get("expires"),
    ))
}
pub async fn session(
    store: &Store,
    config: &Config,
    id: &str,
    login: &str,
) -> Result<Response, StatusCode> {
    if config.role(id).is_none() {
        return Err(StatusCode::FORBIDDEN);
    }
    sqlx::query("DELETE FROM sessions WHERE expires<=?")
        .bind(epoch())
        .execute(&store.pool)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let token = secret();
    let csrf = secret();
    sqlx::query("INSERT INTO sessions(token_hash,user_id,login,csrf,expires) VALUES(?,?,?,?,?)")
        .bind(hash(&token))
        .bind(id)
        .bind(login)
        .bind(csrf)
        .bind(epoch() + 8 * 3600)
        .execute(&store.pool)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let mut response = Redirect::to(&config.origin).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        set_cookie("argos_session", &token, 8 * 3600, config)
            .parse()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    Ok(response)
}
pub async fn start(store: &Store, config: &Config) -> Result<Response, StatusCode> {
    let state = secret();
    let verifier = secret();
    sqlx::query("DELETE FROM oauth_states WHERE expires<=?")
        .bind(epoch())
        .execute(&store.pool)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    sqlx::query("INSERT INTO oauth_states(state_hash,verifier,expires) VALUES(?,?,?)")
        .bind(hash(&state))
        .bind(&verifier)
        .bind(epoch() + 600)
        .execute(&store.pool)
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let mut url = url::Url::parse("https://github.com/login/oauth/authorize")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    url.query_pairs_mut()
        .append_pair("client_id", &config.github_client)
        .append_pair("redirect_uri", &format!("{}/auth/callback", config.origin))
        .append_pair("state", &state)
        .append_pair("code_challenge", &hash(&verifier))
        .append_pair("code_challenge_method", "S256");
    let mut response = Redirect::to(url.as_str()).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        set_cookie("argos_oauth", &state, 600, config)
            .parse()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    Ok(response)
}
pub async fn callback(
    store: &Store,
    config: &Config,
    http: &reqwest::Client,
    headers: &HeaderMap,
    code: &str,
    state: &str,
) -> Result<Response, StatusCode> {
    if cookie(headers, "argos_oauth").as_deref() != Some(state) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let row =
        sqlx::query("DELETE FROM oauth_states WHERE state_hash=? AND expires>? RETURNING verifier")
            .bind(hash(state))
            .bind(epoch())
            .fetch_optional(&store.pool)
            .await
            .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?
            .ok_or(StatusCode::BAD_REQUEST)?;
    let verifier: String = row.get("verifier");
    let token: serde_json::Value = http
        .post("https://github.com/login/oauth/access_token")
        .header("Accept", "application/json")
        .form(&[
            ("client_id", config.github_client.as_str()),
            ("client_secret", config.github_secret.as_str()),
            ("code", code),
            ("code_verifier", &verifier),
            ("redirect_uri", &format!("{}/auth/callback", config.origin)),
        ])
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        .error_for_status()
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        .json()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let access = token["access_token"]
        .as_str()
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let user: serde_json::Value = http
        .get("https://api.github.com/user")
        .bearer_auth(access)
        .header("User-Agent", "ARGOS")
        .send()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        .error_for_status()
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        .json()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let id = user["id"]
        .as_u64()
        .ok_or(StatusCode::UNAUTHORIZED)?
        .to_string();
    let login = user["login"].as_str().ok_or(StatusCode::UNAUTHORIZED)?;
    let mut response = session(store, config, &id, login).await?;
    response.headers_mut().append(
        header::SET_COOKIE,
        set_cookie("argos_oauth", "", 0, config)
            .parse()
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    );
    Ok(response)
}

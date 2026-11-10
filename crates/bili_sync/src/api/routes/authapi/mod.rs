use anyhow::Result;
use axum::extract::Extension;
use axum::http::HeaderValue;
use axum::http::header::SET_COOKIE;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};

use crate::api::wrapper::{ApiError, ApiResponse};
use crate::auth;

#[derive(Deserialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct AuthState {
    /// 是否已创建过账号（false 表示首次使用，需要初始化）
    initialized: bool,
    /// 当前请求是否已登录
    logged_in: bool,
    username: Option<String>,
    /// 程序版本号
    version: String,
    /// 认证方式标识，前端可据此展示
    auth_mode: &'static str,
}

pub(super) fn router() -> Router {
    Router::new()
        .route("/auth/state", get(state))
        .route("/auth/setup", post(setup))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
}

/// 认证状态：给登录页/前端判断「要初始化还是要登录」
async fn state(
    Extension(db): Extension<DatabaseConnection>,
    headers: axum::http::HeaderMap,
) -> Result<ApiResponse<AuthState>, ApiError> {
    let initialized = auth::is_initialized(&db).await?;
    let session = auth::current_user(&headers);
    Ok(ApiResponse::ok(AuthState {
        initialized,
        logged_in: session.is_some(),
        username: session.map(|s| s.username),
        version: crate::config::version().to_string(),
        auth_mode: "password",
    }))
}

/// 首次使用：创建账号并自动登录
async fn setup(
    Extension(db): Extension<DatabaseConnection>,
    Json(form): Json<Credentials>,
) -> Result<Response, ApiError> {
    if auth::is_initialized(&db).await? {
        return Ok(ApiResponse::<()>::bad_request("账号已创建，请直接登录").into_response());
    }
    if let Err(e) = auth::validate_credentials(&form.username, &form.password) {
        return Ok(ApiResponse::<()>::bad_request(e.to_string()).into_response());
    }
    auth::create_user(&db, &form.username, &form.password).await?;
    let username = form.username.trim().to_owned();
    let (token, ttl) = auth::create_session(&username);
    info!("已创建管理账号：{username}，并自动登录");
    Ok(with_cookie(
        ApiResponse::ok(serde_json::json!({ "username": username })),
        &auth::set_cookie_header(&token, ttl),
    ))
}

/// 账号密码登录
async fn login(
    Extension(db): Extension<DatabaseConnection>,
    Json(form): Json<Credentials>,
) -> Result<Response, ApiError> {
    let username = form.username.trim().to_owned();
    if !auth::verify_user(&db, &username, &form.password).await? {
        warn!("登录失败：用户名或密码错误（{username}）");
        return Ok(ApiResponse::<()>::unauthorized("用户名或密码错误").into_response());
    }
    let (token, ttl) = auth::create_session(&username);
    info!("账号登录成功：{username}");
    Ok(with_cookie(
        ApiResponse::ok(serde_json::json!({ "username": username })),
        &auth::set_cookie_header(&token, ttl),
    ))
}

/// 退出登录
async fn logout(headers: axum::http::HeaderMap) -> Response {
    if let Some(token) = auth::session_token_from_headers(&headers) {
        auth::drop_session(&token);
    }
    with_cookie(ApiResponse::ok(serde_json::json!({ "ok": true })), &auth::clear_cookie_header())
}

fn with_cookie<T: Serialize>(body: ApiResponse<T>, cookie: &str) -> Response {
    let mut resp = body.into_response();
    if let Ok(value) = HeaderValue::from_str(cookie) {
        resp.headers_mut().insert(SET_COOKIE, value);
    }
    resp
}

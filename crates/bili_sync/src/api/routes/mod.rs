use axum::extract::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum::{Router, middleware};
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;

use crate::api::wrapper::ApiResponse;
use crate::auth;
use crate::config::VersionedConfig;

mod authapi;
mod config;
mod dashboard;
mod login;
mod me;
mod task;
mod video_sources;
mod videos;
mod ws;

pub use ws::{LogHelper, MAX_HISTORY_LOGS};

pub fn router() -> Router {
    Router::new().nest(
        "/api",
        config::router()
            .merge(me::router())
            .merge(login::router())
            .merge(video_sources::router())
            .merge(videos::router())
            .merge(dashboard::router())
            .merge(ws::router())
            .merge(task::router())
            // 账号体系接口（登录/初始化/登出/状态），自身不做鉴权
            .merge(authapi::router())
            .layer(middleware::from_fn(auth)),
    )
}

/// 中间件：账号会话优先，其次兼容旧的 auth_token（保持脚本/Webhook 调用可用）
pub async fn auth(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();

    // 登录相关接口：匿名可访问
    if path.starts_with("/api/auth/") {
        return next.run(request).await;
    }

    // 1) 浏览器登录态（Cookie 会话）
    if auth::current_user(request.headers()).is_some() {
        return next.run(request).await;
    }

    // 2) 机器调用：Authorization: <auth_token>（原逻辑，保留兼容）
    let token = VersionedConfig::get().read().auth_token.clone();
    if request
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s == token)
    {
        return next.run(request).await;
    }

    // 3) WebSocket 订阅：通过 Sec-WebSocket-Protocol 携带 base64(token)
    if let Some(protocol) = request
        .headers()
        .get("Sec-WebSocket-Protocol")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
        && BASE64_URL_SAFE_NO_PAD
            .decode(&protocol)
            .ok()
            .is_some_and(|s| s == token.as_bytes())
    {
        let mut resp = next.run(request).await;
        if let Ok(value) = protocol.parse() {
            resp.headers_mut().insert("Sec-WebSocket-Protocol", value);
        }
        return resp;
    }

    // 未登录：API 返回 401，其余（页面）引导到登录页
    if path.starts_with("/api/") {
        return ApiResponse::<()>::unauthorized("未登录或会话已过期").into_response();
    }
    Redirect::to("/login").into_response()
}

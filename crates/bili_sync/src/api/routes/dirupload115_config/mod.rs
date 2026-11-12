//! MoviePilot（下称 MP）DirUpload115 插件配置代理。
//!
//! 浏览器与 MP 不同源（且 API_TOKEN 不能下发到前端 / 仓库），所以统一由后端代转：
//! - `GET /api/dirupload115/config`：读取插件全量配置
//! - `PUT /api/dirupload115/config`：合并 5 个可编辑字段后整体覆盖写回 MP，并把新值回写 status.json
//!
//! 环境变量（部署时注入，仓库里不写死 token）：
//! - `BILI_SYNC_MP_API`：MP API 前缀，默认 `http://192.168.1.24:3000/api/v1`
//! - `BILI_SYNC_MP_TOKEN`：MP 的 API_TOKEN（放在 `X-API-KEY` 头里），未设置 → `available=false`
//! - `BILI_SYNC_MP_PLUGIN`：插件 ID，默认 `DirUpload115`
//! - `BILI_SYNC_UPLOAD_STATUS`：插件 status.json 路径，与 upload_status 模块保持一致
//!
//! 注意：MP 的写接口是**整体覆盖**（body 必须是被修改后的全量配置对象），
//! 所以 PUT 一律「先 GET 全量 → 合并请求里出现的字段 → 再整体 PUT 回去」。

use std::path::PathBuf;
use std::time::Duration;

use axum::routing::get;
use axum::{Json, Router};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::api::wrapper::{ApiError, ApiResponse};

/// MP 接口默认前缀
const DEFAULT_MP_API: &str = "http://192.168.1.24:3000/api/v1";
/// 插件默认 ID
const DEFAULT_MP_PLUGIN: &str = "DirUpload115";
/// status.json 默认路径（与 upload_status 模块一致）
const DEFAULT_STATUS_PATH: &str = "/downloads/.dirupload115/status.json";
/// 单次 MP 请求超时
const REQUEST_TIMEOUT_SECS: u64 = 15;
/// 连接超时
const CONNECT_TIMEOUT_SECS: u64 = 5;
/// 允许通过本接口修改的字段（顺序与前端展示顺序一致）
const EDITABLE_FIELDS: [&str; 5] = [
    "max_per_round",
    "max_per_hour",
    "interval_seconds",
    "yield_to_mp",
    "delete_mode",
];
/// delete_mode 允许取值
const DELETE_MODES: [&str; 3] = ["delete", "recycle", "keep"];

pub(super) fn router() -> Router {
    Router::new().route("/dirupload115/config", get(get_config).put(put_config))
}

fn mp_api() -> String {
    std::env::var("BILI_SYNC_MP_API")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_MP_API.to_owned())
}

fn mp_token() -> Option<String> {
    std::env::var("BILI_SYNC_MP_TOKEN")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn mp_plugin() -> String {
    std::env::var("BILI_SYNC_MP_PLUGIN")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_MP_PLUGIN.to_owned())
}

fn status_path() -> PathBuf {
    std::env::var("BILI_SYNC_UPLOAD_STATUS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_STATUS_PATH))
}

/// 构造访问 MP 的 HTTP 客户端
fn http_client() -> Result<reqwest::Client, String> {
    // 与 bilibili::Client 保持一致：仓库启用了 rustls 的 ring 特性但没有默认 provider，进程内装一次即可。
    // 重复安装会返回 Err，直接忽略（说明别处已经装过，这正是我们想要的）。
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT_SECS))
        .build()
        .map_err(|e| format!("初始化 MoviePilot HTTP 客户端失败：{e}"))
}

/// MP 统一返回信封：`{"success": bool, "message": str, "data": any}`
#[derive(Debug, Deserialize)]
struct MpEnvelope {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    message: String,
    #[serde(default)]
    data: Value,
}

/// 请求体：字段全部可选，只传要改的；未出现的字段保持 MP 上的原值
#[derive(Debug, Default, Deserialize)]
struct ConfigUpdate {
    #[serde(default)]
    max_per_round: Option<i64>,
    #[serde(default)]
    max_per_hour: Option<i64>,
    #[serde(default)]
    interval_seconds: Option<i64>,
    #[serde(default)]
    yield_to_mp: Option<bool>,
    #[serde(default)]
    delete_mode: Option<String>,
}

/// 把字符串错误包成 500（ApiError 默认走 internal_server_error 分支）
fn mp_error(message: String) -> ApiError {
    ApiError::from(anyhow::anyhow!("{}", message))
}

/// 响应体里截一小段当错误信息（避免把整页 HTML 塞进 message）
fn snippet(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        "（MoviePilot 返回空响应）".to_owned()
    } else {
        trimmed.chars().take(200).collect()
    }
}

/// 优先用信封里的 message，否则退回原始响应片段
fn error_detail(envelope: Option<&MpEnvelope>, body: &str) -> String {
    let message = envelope.map(|env| env.message.trim()).filter(|msg| !msg.is_empty());
    match message {
        Some(msg) => msg.to_owned(),
        None => snippet(body),
    }
}

/// 校验 MP 信封，成功时返回 `data`
fn unwrap_envelope(status: reqwest::StatusCode, body: &str) -> Result<Value, String> {
    let envelope = serde_json::from_str::<MpEnvelope>(body).ok();
    if !status.is_success() {
        return Err(format!(
            "MoviePilot 返回 HTTP {}：{}",
            status.as_u16(),
            error_detail(envelope.as_ref(), body)
        ));
    }
    let Some(envelope) = envelope else {
        return Err(format!("MoviePilot 响应不是合法 JSON：{}", snippet(body)));
    };
    if !envelope.success {
        return Err(format!("MoviePilot 返回失败：{}", error_detail(Some(&envelope), body)));
    }
    Ok(envelope.data)
}

/// GET `{api}/plugin/{plugin}`，返回插件全量配置对象
async fn fetch_plugin_config(api: &str, plugin: &str, token: &str) -> Result<Value, String> {
    let client = http_client()?;
    let url = format!("{api}/plugin/{plugin}");
    let response = client
        .request(Method::GET, url.as_str())
        .header("X-API-KEY", token)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .send()
        .await
        .map_err(|e| format!("请求 MoviePilot 失败：{e}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取 MoviePilot 响应失败：{e}"))?;
    unwrap_envelope(status, &body)
}

/// PUT `{api}/plugin/{plugin}`，body 是合并后的全量配置对象本身
async fn put_plugin_config(api: &str, plugin: &str, token: &str, config: &Value) -> Result<(), String> {
    let client = http_client()?;
    let url = format!("{api}/plugin/{plugin}");
    let response = client
        .request(Method::PUT, url.as_str())
        .header("X-API-KEY", token)
        .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECS))
        // .json() 会自动带上 Content-Type: application/json，且序列化 body = 全量配置对象
        .json(config)
        .send()
        .await
        .map_err(|e| format!("请求 MoviePilot 失败：{e}"))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|e| format!("读取 MoviePilot 响应失败：{e}"))?;
    unwrap_envelope(status, &body).map(|_| ())
}

/// GET /api/dirupload115/config
///
/// 前端只认 `available`：拿不到配置一律 HTTP 200 + `available=false`，避免页面报错。
async fn get_config() -> Result<ApiResponse<Value>, ApiError> {
    let api = mp_api();
    let plugin = mp_plugin();

    let Some(token) = mp_token() else {
        return Ok(ApiResponse::ok(json!({
            "available": false,
            "reason": "未配置 BILI_SYNC_MP_TOKEN",
            "mp_api": api,
        })));
    };

    // 先落到变量再 match：避免 match 临时 future 与后面移动 api/plugin 的借用冲突
    let result = fetch_plugin_config(&api, &plugin, &token).await;
    match result {
        Ok(config) if config.is_object() => Ok(ApiResponse::ok(json!({
            "available": true,
            "mp_api": api,
            "plugin_id": plugin,
            "config": config,
            "editable": EDITABLE_FIELDS,
        }))),
        Ok(_) => Ok(ApiResponse::ok(json!({
            "available": false,
            "reason": "MoviePilot 返回的插件配置不是 JSON 对象",
            "mp_api": api,
            "plugin_id": plugin,
        }))),
        Err(reason) => Ok(ApiResponse::ok(json!({
            "available": false,
            "reason": reason,
            "mp_api": api,
            "plugin_id": plugin,
        }))),
    }
}

/// PUT /api/dirupload115/config
///
/// 1. GET 全量配置；2. 合并请求里出现的字段；3. 范围校验（失败 400，不写 MP）；
/// 4. 整体 PUT 回 MP；5. 把 5 个字段回写 status.json，让 `/api/upload-status` 立刻生效。
async fn put_config(Json(payload): Json<ConfigUpdate>) -> Result<ApiResponse<Value>, ApiError> {
    let api = mp_api();
    let plugin = mp_plugin();

    let Some(token) = mp_token() else {
        return Err(mp_error(
            "未配置 BILI_SYNC_MP_TOKEN，无法写入 MoviePilot 插件配置".to_owned()
        ));
    };

    // 1) 先取全量配置：MP 的 PUT 是整体覆盖，不拿全量就会丢字段
    let current = fetch_plugin_config(&api, &plugin, &token).await.map_err(mp_error)?;
    let Value::Object(mut merged) = current else {
        // 异常配置中止写入，直接构造 400 响应（ApiResponse::bad_request 的 IntoResponse 会把 HTTP 状态设成 400）
        return Ok(ApiResponse::bad_request(
            "MoviePilot 返回的插件配置不是 JSON 对象，已中止写入",
        ));
    };

    // 2) 合并 + 校验：只处理请求体里出现的字段，其余字段一律保留 MP 上的原值/原类型
    if let Some(value) = payload.max_per_round {
        if !(1..=200).contains(&value) {
            return Ok(ApiResponse::bad_request("max_per_round 取值范围应为 1-200"));
        }
        merged.insert("max_per_round".to_owned(), json!(value));
    }

    if let Some(value) = payload.max_per_hour {
        if !(1..=2000).contains(&value) {
            return Ok(ApiResponse::bad_request("max_per_hour 取值范围应为 1-2000"));
        }
        merged.insert("max_per_hour".to_owned(), json!(value));
    }

    if let Some(value) = payload.interval_seconds {
        if !(600..=86400).contains(&value) {
            return Ok(ApiResponse::bad_request("interval_seconds 取值范围应为 600-86400"));
        }
        merged.insert("interval_seconds".to_owned(), json!(value));
    }

    if let Some(value) = payload.yield_to_mp {
        merged.insert("yield_to_mp".to_owned(), json!(value));
    }

    if let Some(mode) = payload.delete_mode.as_deref() {
        let mode = mode.trim();
        if !DELETE_MODES.contains(&mode) {
            return Ok(ApiResponse::bad_request("delete_mode 只能是 delete / recycle / keep"));
        }
        merged.insert("delete_mode".to_owned(), json!(mode));
    }

    let merged = Value::Object(merged);

    // 3) 整体覆盖写回 MP（失败 → 500，message 带出 MP 返回的错误文本）
    put_plugin_config(&api, &plugin, &token, &merged).await.map_err(mp_error)?;

    // 4) 回写 status.json；失败只影响 status_patched，不影响主流程
    let status_patched = patch_status_file(&merged);

    Ok(ApiResponse::ok(json!({
        "success": true,
        "message": "已保存到 MoviePilot 插件配置",
        "config": merged,
        "status_patched": status_patched,
    })))
}

/// 把合并后的 5 个字段写进 status.json（读-改-写 + tmp/rename 原子替换）
///
/// 读不到 / 解析失败就当空对象处理（插件未启用时目录可能都不存在），
/// 写失败一律返回 `false`，不向外抛错。
fn patch_status_file(config: &Value) -> bool {
    let Some(source) = config.as_object() else {
        return false;
    };

    let path = status_path();
    let mut current = match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str::<Value>(&content).unwrap_or_else(|_| json!({})),
        Err(_) => json!({}),
    };

    let Some(target) = current.as_object_mut() else {
        return false;
    };
    for field in EDITABLE_FIELDS {
        if let Some(value) = source.get(field) {
            target.insert(field.to_owned(), value.clone());
        }
    }

    let Ok(serialized) = serde_json::to_string_pretty(&current) else {
        return false;
    };

    // 临时文件放在同一目录，保证 rename 是原子替换
    let tmp_path = path.with_extension("tmp");
    if std::fs::write(&tmp_path, serialized.as_bytes()).is_err() {
        let _ = std::fs::remove_file(&tmp_path);
        return false;
    }
    std::fs::rename(&tmp_path, &path).is_ok()
}

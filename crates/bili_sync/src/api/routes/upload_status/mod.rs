//! 115 上传进度（数据来自 DirUpload115 插件写的 status.json）
use std::path::PathBuf;

use axum::Json;
use axum::Router;
use axum::routing::get;

use crate::api::wrapper::ApiResponse;

fn status_path() -> PathBuf {
    std::env::var("BILI_SYNC_UPLOAD_STATUS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/downloads/.dirupload115/status.json"))
}

/// GET /api/upload-status
///
/// 插件未安装 / 未启用 / 尚未跑过一轮时返回 `available=false`，前端据此显示「插件未启用」
pub async fn upload_status() -> Json<ApiResponse<serde_json::Value>> {
    let path = status_path();
    let body = match std::fs::read_to_string(&path) {
        Ok(content) => match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(mut value) => {
                value["available"] = serde_json::json!(true);
                value
            }
            Err(e) => serde_json::json!({
                "available": false,
                "reason": format!("status.json 解析失败：{e}"),
                "path": path.to_string_lossy(),
            }),
        },
        Err(_) => serde_json::json!({
            "available": false,
            "reason": "status.json 不存在（插件未启用或尚未运行一轮）",
            "path": path.to_string_lossy(),
        }),
    };
    Json(ApiResponse::ok(body))
}

pub fn router() -> Router {
    Router::new().route("/upload-status", get(upload_status))
}

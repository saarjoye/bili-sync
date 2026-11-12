//! 115 上传进度（数据来自 DirUpload115 插件写的 status.json）
use std::path::PathBuf;

use axum::Router;
use axum::routing::get;
use serde::{Deserialize, Serialize};

use crate::api::ApiResponse;
use crate::api::error::ApiError;

/// 仪表盘读到的上传状态
#[derive(Debug, Default, Deserialize, Serialize)]
pub struct UploadStatus {
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub round_started_at: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub interval_seconds: i64,
    #[serde(default)]
    pub max_per_round: i64,
    #[serde(default)]
    pub yield_to_mp: bool,
    #[serde(default)]
    pub delete_mode: String,
    #[serde(default)]
    pub ready_mode: String,
    #[serde(default)]
    pub queue: UploadQueue,
    #[serde(default)]
    pub round: UploadRound,
    #[serde(default)]
    pub recent: Vec<UploadEvent>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct UploadQueue {
    #[serde(default)]
    pub total: i64,
    #[serde(default)]
    pub done: i64,
    #[serde(default)]
    pub pending: i64,
    #[serde(default)]
    pub waiting: i64,
    #[serde(default)]
    pub failed: i64,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct UploadRound {
    #[serde(default)]
    pub scanned: i64,
    #[serde(default)]
    pub uploaded: i64,
    #[serde(default)]
    pub waiting: i64,
    #[serde(default)]
    pub skipped: i64,
    #[serde(default)]
    pub failed: i64,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct UploadEvent {
    #[serde(default)]
    pub time: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub label: String,
}

fn status_path() -> PathBuf {
    std::env::var("BILI_SYNC_UPLOAD_STATUS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/downloads/.dirupload115/status.json"))
}

/// GET /api/upload-status
///
/// 插件未安装/未启用/文件尚未生成时返回 `available=false`，前端据此显示「插件未启用」
pub async fn upload_status() -> Result<ApiResponse<serde_json::Value>, ApiError> {
    let path = status_path();
    if !path.exists() {
        return Ok(ApiResponse::ok(serde_json::json!({
            "available": false,
            "reason": "status.json 不存在（插件未启用或尚未运行一轮）",
            "path": path.to_string_lossy(),
        })));
    }
    let content = tokio::fs::read_to_string(&path).await?;
    let mut value: serde_json::Value = serde_json::from_str(&content)?;
    value["available"] = serde_json::json!(true);
    Ok(ApiResponse::ok(value))
}

pub fn router() -> Router {
    Router::new().route("/upload-status", get(upload_status))
}

//! 目录预设（仿 MoviePilot 的「目录」管理）：
//! 用户先定义若干「目录」（别名 + 本地路径 + 115 路径），订阅时直接选目录。
//!
//! 这里用原生 SQL 访问 `dir_preset` 表，避免为一个简单配置表引入 sea-orm entity。

use anyhow::{Context, Result};
use axum::extract::{Extension, Path};
use axum::routing::get;
use axum::{Json, Router};
use sea_orm::{ConnectionTrait, DatabaseConnection, FromQueryResult, Statement, Value};
use serde::{Deserialize, Serialize};

use crate::api::wrapper::{ApiError, ApiResponse};

#[derive(Debug, Clone, Serialize, Deserialize, FromQueryResult)]
pub struct DirPreset {
    pub id: i32,
    pub alias: String,
    pub local_path: String,
    pub target115_path: String,
    pub default_season: i32,
    /// 0=UP名  1=UP名-ID  2=仅ID
    pub upper_display: i32,
    pub sort_order: i32,
}

#[derive(Debug, Deserialize)]
pub struct DirPresetForm {
    pub alias: String,
    pub local_path: String,
    #[serde(default)]
    pub target115_path: String,
    #[serde(default)]
    pub default_season: i32,
    #[serde(default = "default_upper_display")]
    pub upper_display: i32,
    #[serde(default)]
    pub sort_order: i32,
}

fn default_upper_display() -> i32 {
    1
}

pub(super) fn router() -> Router {
    Router::new()
        .route("/dirs", get(list_dirs).post(create_dir))
        .route("/dirs/{id}", axum::routing::put(update_dir).delete(delete_dir))
}

const SELECT_COLS: &str = "id, alias, local_path, target115_path, default_season, upper_display, sort_order";

pub async fn list_dirs(Extension(db): Extension<DatabaseConnection>) -> Result<ApiResponse<Vec<DirPreset>>, ApiError> {
    Ok(ApiResponse::ok(load_dirs(&db).await?))
}

pub async fn load_dirs(db: &DatabaseConnection) -> Result<Vec<DirPreset>> {
    let stmt = Statement::from_string(
        db.get_database_backend(),
        format!("SELECT {SELECT_COLS} FROM dir_preset ORDER BY sort_order ASC, id ASC"),
    );
    let rows = DirPreset::find_by_statement(stmt)
        .all(db)
        .await
        .context("查询 dir_preset 失败")?;
    Ok(rows)
}

pub async fn get_dir(db: &DatabaseConnection, id: i32) -> Result<Option<DirPreset>> {
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        format!("SELECT {SELECT_COLS} FROM dir_preset WHERE id = ?"),
        [Value::from(id)],
    );
    Ok(DirPreset::find_by_statement(stmt).one(db).await?)
}

async fn create_dir(
    Extension(db): Extension<DatabaseConnection>,
    Json(form): Json<DirPresetForm>,
) -> Result<ApiResponse<DirPreset>, ApiError> {
    validate(&form)?;
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        "INSERT INTO dir_preset (alias, local_path, target115_path, default_season, upper_display, sort_order) \
         VALUES (?, ?, ?, ?, ?, ?)",
        [
            Value::from(form.alias.trim()),
            Value::from(normalize_path(&form.local_path)),
            Value::from(form.target115_path.trim()),
            Value::from(form.default_season),
            Value::from(form.upper_display),
            Value::from(form.sort_order),
        ],
    );
    let res = db.execute(stmt).await.context("插入目录失败")?;
    let id = res.last_insert_id() as i32;
    let dir = get_dir(&db, id).await?.context("插入后查询目录失败")?;
    Ok(ApiResponse::ok(dir))
}

async fn update_dir(
    Extension(db): Extension<DatabaseConnection>,
    Path(id): Path<i32>,
    Json(form): Json<DirPresetForm>,
) -> Result<ApiResponse<DirPreset>, ApiError> {
    validate(&form)?;
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        "UPDATE dir_preset SET alias = ?, local_path = ?, target115_path = ?, default_season = ?, \
         upper_display = ?, sort_order = ? WHERE id = ?",
        [
            Value::from(form.alias.trim()),
            Value::from(normalize_path(&form.local_path)),
            Value::from(form.target115_path.trim()),
            Value::from(form.default_season),
            Value::from(form.upper_display),
            Value::from(form.sort_order),
            Value::from(id),
        ],
    );
    db.execute(stmt).await.context("更新目录失败")?;
    let dir = get_dir(&db, id).await?.context("目录不存在")?;
    Ok(ApiResponse::ok(dir))
}

async fn delete_dir(
    Extension(db): Extension<DatabaseConnection>,
    Path(id): Path<i32>,
) -> Result<ApiResponse<bool>, ApiError> {
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        "DELETE FROM dir_preset WHERE id = ?",
        [Value::from(id)],
    );
    db.execute(stmt).await.context("删除目录失败")?;
    Ok(ApiResponse::ok(true))
}

fn normalize_path(p: &str) -> String {
    let mut s = p.trim().replace('\\', "/");
    while s.ends_with('/') && s.len() > 1 {
        s.pop();
    }
    s
}

fn validate(form: &DirPresetForm) -> Result<(), ApiError> {
    if form.alias.trim().is_empty() {
        return Err(ApiError::from(anyhow::anyhow!("目录别名不能为空")));
    }
    if form.local_path.trim().is_empty() {
        return Err(ApiError::from(anyhow::anyhow!("本地路径不能为空")));
    }
    if !(0..=2).contains(&form.upper_display) {
        return Err(ApiError::from(anyhow::anyhow!("作者显示方式取值应为 0/1/2")));
    }
    if form.default_season < 0 {
        return Err(ApiError::from(anyhow::anyhow!("默认季号不能为负数")));
    }
    Ok(())
}

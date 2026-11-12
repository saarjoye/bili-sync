//! 订阅向导：粘地址 → 识别 UP/合集/系列/单视频/收藏夹 → 选目录 → 一键订阅。
//!
//! 合集发现复用 `Submission::list_seasons()`（bilibili 模块内部实现两步发现，
//! 因为 wbi 签名密钥只在该模块内可见）。

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use axum::extract::Extension;
use axum::routing::post;
use axum::{Json, Router};
use sea_orm::ActiveValue::Set;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, IntoActiveModel, QueryFilter, TransactionTrait,
};
use serde::{Deserialize, Serialize};

use crate::adapter::VideoSourceEnum;
use crate::api::wrapper::{ApiError, ApiResponse};
use crate::bilibili::{BiliClient, Collection, CollectionItem, CollectionType, Submission, Video, VideoInfo};
use crate::config::{Config, TEMPLATE, VersionedConfig};
use crate::downloader::Downloader;
use crate::utils::download_context::DownloadContext;

use super::dirs;

pub(super) fn router() -> Router {
    Router::new()
        .route("/wizard/resolve", post(resolve))
        .route("/wizard/subscribe", post(subscribe))
        .route("/wizard/download-video", post(download_video))
}

#[derive(Debug, Deserialize)]
pub struct ResolveRequest {
    /// UP 空间地址 / 合集地址 / 系列地址 / 视频地址 / 收藏夹地址
    pub url: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Candidate {
    pub sid: i64,
    /// 2 = 合集，1 = 系列/列表
    pub collection_type: i32,
    pub name: String,
    pub count: i64,
}

#[derive(Debug, Deserialize)]
pub struct DownloadVideoRequest {
    pub bvid: String,
    pub dir_id: i32,
    /// 追新：同时订阅该 UP 的投稿，以后新视频自动下载
    #[serde(default)]
    pub follow_new: bool,
}

#[derive(Debug, Serialize)]
pub struct DownloadVideoResponse {
    pub video_id: i32,
    pub name: String,
    pub upper_name: String,
    pub mid: i64,
    pub path: String,
    pub follow_new: bool,
}

#[derive(Debug, Default, Serialize)]
pub struct ResolveResponse {
    /// upper / collection / series / video / favorite / unknown
    pub kind: String,
    pub mid: Option<i64>,
    pub upper_name: Option<String>,
    pub sid: Option<i64>,
    pub collection_type: Option<i32>,
    pub bvid: Option<String>,
    pub fid: Option<i64>,
    /// 合集名 / 视频标题 / 收藏夹名
    pub name: Option<String>,
    pub count: Option<i64>,
    /// 该 UP 名下可选的合集列表
    pub candidates: Vec<Candidate>,
    /// 给用户的提示（例如需要完整地址）
    pub hint: Option<String>,
}


/// 把 B站 标题/合集名里的非法文件名字符替换掉，直接作为目录名使用
fn safe_dir_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    if trimmed.is_empty() {
        "未命名".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[derive(Debug, Deserialize)]
pub struct SubscribeItem {
    pub sid: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default = "default_collection_type")]
    pub collection_type: i32,
}

fn default_collection_type() -> i32 {
    2
}

#[derive(Debug, Deserialize)]
pub struct SubscribeRequest {
    pub dir_id: i32,
    pub mid: i64,
    #[serde(default)]
    pub upper_name: String,
    #[serde(default)]
    pub items: Vec<SubscribeItem>,
    /// 追新：额外订阅该 UP 的投稿（以后新视频自动下载）
    #[serde(default)]
    pub follow_new: bool,
    /// 是否立即启用，默认 true
    #[serde(default)]
    pub enable: Option<bool>,
    /// 已存在时是否覆盖原下载路径（默认 false，只更新启用状态与 UP 名）
    #[serde(default)]
    pub force_path: bool,
}

#[derive(Debug, Serialize)]
pub struct SubscribeItemResult {
    pub sid: i64,
    pub name: String,
    /// created / updated
    pub action: String,
    /// 合集实际归属的 UP
    pub mid: i64,
    pub upper_name: String,
    pub path: String,
    /// 给用户看的说明
    pub note: String,
}

#[derive(Debug, Serialize)]
pub struct SubscribeResponse {
    pub items: Vec<SubscribeItemResult>,
    pub created: Vec<String>,
    pub updated: Vec<String>,
    pub submission_created: bool,
}

async fn resolve(
    Extension(bili_client): Extension<Arc<BiliClient>>,
    Json(request): Json<ResolveRequest>,
) -> Result<ApiResponse<ResolveResponse>, ApiError> {
    let credential = &VersionedConfig::get().read().credential;
    let parsed = parse_url(&request.url);
    let mut resp = ResolveResponse {
        kind: parsed.kind.clone(),
        mid: parsed.mid,
        sid: parsed.sid,
        bvid: parsed.bvid.clone(),
        fid: parsed.fid,
        collection_type: parsed.collection_type,
        ..Default::default()
    };

    match parsed.kind.as_str() {
        "upper" => {
            let mid = parsed.mid.context("未能从地址中解析出 UP ID（mid）")?;
            let submission = Submission::new(bili_client.as_ref(), mid.to_string(), credential);
            if let Ok(upper) = submission.get_info().await {
                resp.upper_name = Some(upper.name.clone());
            }
            resp.candidates = submission
                .list_seasons()
                .await?
                .into_iter()
                .map(|s| Candidate {
                    sid: s.sid,
                    collection_type: 2,
                    name: s.name,
                    count: s.count,
                })
                .collect();
            if resp.candidates.is_empty() {
                resp.hint = Some("这个 UP 名下没有发现合集（可能内容都是单视频或系列）".to_owned());
            }
        }
        "collection" | "series" => {
            let sid = parsed.sid.context("未能从地址中解析出合集 ID（sid）")?;
            let ctype = if parsed.kind == "series" { 1 } else { 2 };
            let Some(mid) = parsed.mid else {
                resp.hint = Some(
                    "请粘贴完整地址（形如 https://space.bilibili.com/<UP ID>/channel/collectiondetail?sid=xxx），否则拿不到 UP ID"
                        .to_owned(),
                );
                return Ok(ApiResponse::ok(resp));
            };
            let submission = Submission::new(bili_client.as_ref(), mid.to_string(), credential);
            resp.upper_name = submission.get_info().await.ok().map(|u| u.name);
            if let Ok(info) = Collection::new(
                bili_client.as_ref(),
                CollectionItem {
                    sid: sid.to_string(),
                    mid: mid.to_string(),
                    collection_type: CollectionType::from_expected(ctype),
                },
                credential,
            )
            .get_info()
            .await
            {
                resp.name = Some(info.name);
            }
            if let Ok(list) = submission.list_seasons().await {
                resp.candidates = list
                    .into_iter()
                    .map(|s| Candidate {
                        sid: s.sid,
                        collection_type: 2,
                        name: s.name,
                        count: s.count,
                    })
                    .collect();
                resp.count = resp
                    .candidates
                    .iter()
                    .find(|c| c.sid == sid)
                    .map(|c| c.count);
            }
        }
        "video" => {
            let bvid = parsed.bvid.clone().context("未能从地址中解析出 BV 号")?;
            let video = crate::bilibili::Video::new(bili_client.as_ref(), &bvid, credential);
            let info = video.get_view_info().await?;
            if let crate::bilibili::VideoInfo::Detail { title, upper, .. } = &info {
                resp.name = Some(title.clone());
                resp.mid = Some(upper.mid);
                resp.upper_name = Some(upper.name.clone());
            }
        }
        "favorite" => {
            resp.hint = Some("收藏夹订阅请在「收藏夹」页签里添加（需要 fid 与路径）".to_owned());
        }
        _ => {
            resp.hint = Some(
                "无法识别的地址。支持：UP 空间、合集（collectiondetail）、系列（seriesdetail）、视频（video/BVxxx）"
                    .to_owned(),
            );
        }
    }

    Ok(ApiResponse::ok(resp))
}

async fn subscribe(
    Extension(db): Extension<DatabaseConnection>,
    Extension(bili_client): Extension<Arc<BiliClient>>,
    Json(request): Json<SubscribeRequest>,
) -> Result<ApiResponse<SubscribeResponse>, ApiError> {
    let credential = &VersionedConfig::get().read().credential;
    let dir = dirs::get_dir(&db, request.dir_id)
        .await?
        .context("目录不存在，请先在「目录」页创建")?;

    let upper_name = if request.upper_name.trim().is_empty() {
        Submission::new(bili_client.as_ref(), request.mid.to_string(), credential)
            .get_info()
            .await
            .ok()
            .map(|u| u.name)
            .unwrap_or_else(|| request.mid.to_string())
    } else {
        request.upper_name.trim().to_owned()
    };
    let author_segment = match dir.upper_display {
        0 => upper_name.clone(),
        2 => request.mid.to_string(),
        _ => format!("{}-{}", upper_name, request.mid),
    };
    let base = format!("{}/{}", dir.local_path.trim_end_matches('/'), author_segment);
    let enable = request.enable.unwrap_or(true);

    let txn = db.begin().await?;
    let mut created = Vec::new();
    let mut updated = Vec::new();
    let mut items: Vec<SubscribeItemResult> = Vec::new();
    for item in &request.items {
        let collection = Collection::new(
            bili_client.as_ref(),
            CollectionItem {
                sid: item.sid.to_string(),
                mid: request.mid.to_string(),
                collection_type: CollectionType::from_expected(item.collection_type),
            },
            credential,
        );
        let info = collection.get_info().await?;
        let path = format!("{}/{}", base, safe_dir_name(&info.name));
        let ctype: i32 = info.collection_type.into();
        let existing = bili_sync_entity::collection::Entity::find()
            .filter(bili_sync_entity::collection::Column::SId.eq(info.sid))
            .filter(bili_sync_entity::collection::Column::MId.eq(info.mid))
            .filter(bili_sync_entity::collection::Column::Type.eq(ctype))
            .one(&txn)
            .await?;
        match existing {
            Some(model) => {
                // 同一个合集（sid 相同）已经订阅过：默认只更新启用状态与 UP 名，绝不悄悄改路径
                let old_path = model.path.clone();
                let old_upper = model.upper_name.clone();
                let mut active = model.clone().into_active_model();
                active.enabled = Set(enable);
                if old_upper.trim().is_empty() {
                    active.upper_name = Set(upper_name.clone());
                }
                let mut note = format!(
                    "该合集已订阅过（归属 {} / {}，sid={}）",
                    if old_upper.trim().is_empty() { "-" } else { old_upper.as_str() },
                    model.m_id,
                    info.sid
                );
                if request.force_path && old_path != path {
                    active.path = Set(path.clone());
                    note.push_str("；已按本次目录覆盖路径");
                } else {
                    note.push_str("；保留原路径");
                }
                active.update(&txn).await?;
                updated.push(info.name.clone());
                items.push(SubscribeItemResult {
                    sid: info.sid,
                    name: info.name.clone(),
                    action: "updated".to_owned(),
                    mid: model.m_id,
                    upper_name: if old_upper.trim().is_empty() { upper_name.clone() } else { old_upper },
                    path: if request.force_path { path.clone() } else { old_path },
                    note,
                });
            }
            None => {
                bili_sync_entity::collection::Entity::insert(bili_sync_entity::collection::ActiveModel {
                    s_id: Set(info.sid),
                    m_id: Set(info.mid),
                    r#type: Set(ctype),
                    name: Set(info.name.clone()),
                    path: Set(path.clone()),
                    enabled: Set(enable),
                    upper_name: Set(upper_name.clone()),
                    ..Default::default()
                })
                .exec(&txn)
                .await?;
                created.push(info.name.clone());
                items.push(SubscribeItemResult {
                    sid: info.sid,
                    name: info.name.clone(),
                    action: "created".to_owned(),
                    mid: info.mid,
                    upper_name: upper_name.clone(),
                    path: path.clone(),
                    note: format!("新建订阅（{enable_label}）", enable_label = if enable { "已启用" } else { "未启用" }),
                });
            }
        }
    }

    let mut submission_created = false;
    if request.follow_new {
        let path = format!("{}/{}", base, safe_dir_name(&upper_name));
        let existing = bili_sync_entity::submission::Entity::find()
            .filter(bili_sync_entity::submission::Column::UpperId.eq(request.mid))
            .one(&txn)
            .await?;
        match existing {
            Some(model) => {
                let mut active = model.clone().into_active_model();
                active.path = Set(path);
                active.enabled = Set(enable);
                active.update(&txn).await?;
            }
            None => {
                bili_sync_entity::submission::Entity::insert(bili_sync_entity::submission::ActiveModel {
                    upper_id: Set(request.mid),
                    upper_name: Set(upper_name.clone()),
                    path: Set(path),
                    enabled: Set(enable),
                    ..Default::default()
                })
                .exec(&txn)
                .await?;
            }
        }
        submission_created = true;
    }
    txn.commit().await?;

    Ok(ApiResponse::ok(SubscribeResponse {
        items,
        created,
        updated,
        submission_created,
    }))
}

#[derive(Default)]
struct ParsedUrl {
    kind: String,
    mid: Option<i64>,
    sid: Option<i64>,
    bvid: Option<String>,
    fid: Option<i64>,
    collection_type: Option<i32>,
}

fn parse_url(raw: &str) -> ParsedUrl {
    let url = raw.trim();
    let mut out = ParsedUrl {
        kind: "unknown".to_owned(),
        ..Default::default()
    };
    if let Some(idx) = url.find("space.bilibili.com/") {
        let rest = &url[idx + "space.bilibili.com/".len()..];
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            out.mid = digits.parse().ok();
        }
    }
    if let Some(v) = query_param(url, "sid") {
        out.sid = v.parse().ok();
    }
    if let Some(v) = query_param(url, "fid") {
        out.fid = v.parse().ok();
    }
    if let Some(pos) = url.find("BV") {
        let bvid: String = url[pos..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if bvid.len() == 12 {
            out.bvid = Some(bvid);
        }
    }

    if url.contains("collectiondetail") {
        out.kind = "collection".to_owned();
        out.collection_type = Some(2);
    } else if url.contains("seriesdetail") {
        out.kind = "series".to_owned();
        out.collection_type = Some(1);
    } else if out.bvid.is_some() {
        out.kind = "video".to_owned();
    } else if url.contains("favlist") {
        out.kind = "favorite".to_owned();
    } else if out.mid.is_some() {
        out.kind = "upper".to_owned();
    }
    out
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let needle = format!("{key}=");
    let start = url.find(&needle)? + needle.len();
    let value: String = url[start..].chars().take_while(|c| c.is_ascii_digit()).collect();
    if value.is_empty() { None } else { Some(value) }
}

#[allow(dead_code)]
fn _assert_bail_used() -> Result<()> {
    bail!("unused")
}

/// 单视频一次性下载：入库后立刻后台下载，不进入任何订阅列表
pub async fn download_video(
    Extension(db): Extension<DatabaseConnection>,
    Extension(bili_client): Extension<Arc<BiliClient>>,
    Json(request): Json<DownloadVideoRequest>,
) -> Result<ApiResponse<DownloadVideoResponse>, ApiError> {
    let credential = &VersionedConfig::get().read().credential;
    let dir = dirs::get_dir(&db, request.dir_id)
        .await?
        .context("目录不存在，请先在「目录」页创建")?;

    let video = Video::new(bili_client.as_ref(), &request.bvid, credential);
    let info = video.get_view_info().await?;
    let VideoInfo::Detail {
        title,
        bvid,
        intro,
        cover,
        upper,
        ctime,
        pubtime,
        pages,
        ..
    } = info
    else {
        return Err(ApiError::from(anyhow::anyhow!("该视频信息不完整，无法下载")));
    };

    let mid = upper.mid;
    let upper_name = upper.name.clone();
    let base = format!(
        "{}/{}-{}/单视频",
        dir.local_path.trim_end_matches('/'),
        upper_name,
        mid
    );
    let page_count = pages.len();

    let inserted = bili_sync_entity::video::ActiveModel {
        bvid: Set(bvid),
        name: Set(title.clone()),
        intro: Set(intro),
        cover: Set(cover),
        category: Set(2),
        upper_id: Set(mid),
        upper_name: Set(upper_name.clone()),
        upper_face: Set(upper.face),
        ctime: Set(ctime.naive_utc()),
        pubtime: Set(pubtime.naive_utc()),
        // 这些列 NOT NULL 且无数据库默认值，必须显式给
        favtime: Set(pubtime.naive_utc()),
        download_status: Set(0),
        created_at: Set(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()),
        staff: Set(None),
        tags: Set(None),
        valid: Set(true),
        should_download: Set(true),
        single_page: Set(Some(page_count == 1)),
        path: Set(base.clone()),
        watch_later_id: Set(Some(1)),
        ..Default::default()
    }
    .insert(&db)
    .await?;
    let video_id = inserted.id;

    let page_models = pages
        .into_iter()
        .map(|p| p.into_active_model(video_id))
        .collect::<Vec<_>>();
    if !page_models.is_empty() {
        bili_sync_entity::page::Entity::insert_many(page_models)
            .exec(&db)
            .await?;
    }

    // 后台下载，避免 HTTP 请求长时间阻塞
    let db2 = db.clone();
    let client2 = bili_client.clone();
    tokio::spawn(async move {
        if let Err(e) = run_single_download(db2, client2, video_id).await {
            tracing::error!("单视频下载失败（video_id={}）：{:#}", video_id, e);
        }
    });

    let mut follow_created = false;
    if request.follow_new {
        let enable = true;
        let path = format!(
            "{}/{}-{}/{}",
            dir.local_path.trim_end_matches('/'),
            upper_name,
            mid,
            safe_dir_name(&upper_name)
        );
        let existing = bili_sync_entity::submission::Entity::find()
            .filter(bili_sync_entity::submission::Column::UpperId.eq(mid))
            .one(&db)
            .await?;
        match existing {
            Some(model) => {
                let mut active = model.into_active_model();
                active.enabled = Set(enable);
                active.update(&db).await?;
            }
            None => {
                bili_sync_entity::submission::Entity::insert(bili_sync_entity::submission::ActiveModel {
                    upper_id: Set(mid),
                    upper_name: Set(upper_name.clone()),
                    path: Set(path),
                    enabled: Set(enable),
                    ..Default::default()
                })
                .exec(&db)
                .await?;
            }
        }
        follow_created = true;
    }

    Ok(ApiResponse::ok(DownloadVideoResponse {
        video_id,
        name: title,
        upper_name,
        mid,
        path: base,
        follow_new: follow_created,
    }))
}

/// 真正执行下载（在后台任务里跑，复用主流程的 download_video_pages）
async fn run_single_download(
    db: DatabaseConnection,
    bili_client: Arc<BiliClient>,
    video_id: i32,
) -> Result<()> {
    let config: Arc<Config> = VersionedConfig::get().snapshot();
    let video_model = bili_sync_entity::video::Entity::find_by_id(video_id)
        .one(&db)
        .await?
        .context("视频记录不存在")?;
    let page_models = bili_sync_entity::page::Entity::find()
        .filter(bili_sync_entity::page::Column::VideoId.eq(video_id))
        .all(&db)
        .await?;
    // 「稍后再看」源只是用来借一个 VideoSource 上下文（真正的落盘路径取自 video.path）
    // 若数据库里还没有这一行，则按默认值补一条
    let watch_later_model = match bili_sync_entity::watch_later::Entity::find_by_id(1).one(&db).await? {
        Some(model) => model,
        None => {
            bili_sync_entity::watch_later::ActiveModel {
                path: Set(String::new()),
                created_at: Set(chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string()),
                latest_row_at: Set(chrono::DateTime::from_timestamp(0, 0)
                    .map(|d| d.naive_utc())
                    .unwrap_or_default()),
                enabled: Set(false),
                ..Default::default()
            }
            .insert(&db)
            .await?
        }
    };
    let video_source: VideoSourceEnum = watch_later_model.into();
    let template = TEMPLATE.read();
    let downloader = Downloader::new(bili_client.client.clone());
    let filter_option = config.filter_option.clone();
    let cx = DownloadContext::new(
        bili_client.as_ref(),
        &video_source,
        &template,
        &db,
        &downloader,
        &config,
        &filter_option,
    );
    let updated = crate::workflow::download_video_pages(video_model, page_models, vec![], cx).await?;
    updated.update(&db).await?;
    tracing::info!("单视频下载完成（video_id={}）", video_id);
    Ok(())
}

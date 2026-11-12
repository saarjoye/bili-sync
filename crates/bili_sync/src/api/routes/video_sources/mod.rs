use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::{Extension, Path, Query};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use bili_sync_entity::rule::Rule;
use bili_sync_entity::*;
use bili_sync_migration::Expr;
use futures::stream::FuturesUnordered;
use futures::{StreamExt, TryStreamExt};
use itertools::Itertools;
use serde::Deserialize;
use sea_orm::ActiveValue::Set;
use sea_orm::entity::prelude::*;
use sea_orm::IntoActiveModel;
use sea_orm::{ColumnTrait, DatabaseConnection, EntityTrait, QuerySelect, QueryTrait, TransactionTrait};

use crate::adapter::{_ActiveModel, VideoSource as _, VideoSourceEnum};
use crate::api::error::InnerApiError;
use crate::api::request::{
    DefaultPathRequest, FullSyncVideoSourceRequest, InsertCollectionRequest, InsertFavoriteRequest,
    InsertSubmissionRequest, UpdateVideoSourceRequest,
};
use crate::api::response::{
    FullSyncVideoSourceResponse, UpdateVideoSourceResponse, VideoSource, VideoSourceDetail,
    VideoSourcesDetailsResponse, VideoSourcesResponse,
};
use crate::api::wrapper::{ApiError, ApiResponse, ValidatedJson};
use crate::bilibili::{BiliClient, Collection, CollectionItem, FavoriteList, Submission};
use crate::config::{PathSafeTemplate, TEMPLATE, VersionedConfig};
use crate::utils::rule::FieldEvaluatable;

pub(super) fn router() -> Router {
    Router::new()
        .route("/video-sources", get(get_video_sources))
        .route("/video-sources/details", get(get_video_sources_details))
        .route(
            "/video-sources/{type}/default-path",
            get(get_video_sources_default_path),
        ) // 仅用于前端获取默认路径
        .route(
            "/video-sources/{type}/{id}",
            put(update_video_source).delete(remove_video_source),
        )
        .route("/video-sources/backfill-upper", post(backfill_upper_names))
        .route("/video-sources/{type}/{id}/schedule", put(update_schedule))
        .route("/video-sources/{type}/{id}/run", post(run_video_source))
        .route("/video-sources/{type}/{id}/evaluate", post(evaluate_video_source))
        .route("/video-sources/{type}/{id}/full-sync", post(full_sync_video_source))
        .route("/video-sources/favorites", post(insert_favorite))
        .route("/video-sources/collections", post(insert_collection))
        .route("/video-sources/submissions", post(insert_submission))
}

/// 列出所有视频来源
pub async fn get_video_sources(
    Extension(db): Extension<DatabaseConnection>,
) -> Result<ApiResponse<VideoSourcesResponse>, ApiError> {
    let (collection, favorite, submission, mut watch_later) = tokio::try_join!(
        collection::Entity::find()
            .select_only()
            .columns([collection::Column::Id, collection::Column::Name])
            .into_model::<VideoSource>()
            .all(&db),
        favorite::Entity::find()
            .select_only()
            .columns([favorite::Column::Id, favorite::Column::Name])
            .into_model::<VideoSource>()
            .all(&db),
        submission::Entity::find()
            .select_only()
            .column(submission::Column::Id)
            .column_as(submission::Column::UpperName, "name")
            .into_model::<VideoSource>()
            .all(&db),
        watch_later::Entity::find()
            .select_only()
            .column(watch_later::Column::Id)
            .column_as(Expr::value("稍后再看"), "name")
            .into_model::<VideoSource>()
            .all(&db)
    )?;
    // watch_later 是一个特殊的视频来源，如果不存在则添加一个默认项
    if watch_later.is_empty() {
        watch_later.push(VideoSource {
            id: 1,
            name: "稍后再看".to_string(),
        });
    }
    Ok(ApiResponse::ok(VideoSourcesResponse {
        collection,
        favorite,
        submission,
        watch_later,
    }))
}

/// 获取视频来源详情
pub async fn get_video_sources_details(
    Extension(db): Extension<DatabaseConnection>,
) -> Result<ApiResponse<VideoSourcesDetailsResponse>, ApiError> {
    let (mut collections, mut favorites, mut submissions, mut watch_later) = tokio::try_join!(
        collection::Entity::find()
            .select_only()
            .columns([
                collection::Column::Id,
                collection::Column::Name,
                collection::Column::Path,
                collection::Column::Rule,
                collection::Column::FilterOption,
                collection::Column::Enabled,
                collection::Column::LatestRowAt,
                collection::Column::UpperName,
                collection::Column::ScheduleMode,
                collection::Column::IntervalSeconds,
                collection::Column::CronExpr,
                collection::Column::NextRunAt,
                collection::Column::LastCheckedAt,
                collection::Column::LastFoundCount,
                collection::Column::FailCount,
                collection::Column::MaxDays,
                collection::Column::MaxPages
            ])
            .column_as(collection::Column::MId, "mid")
            .into_model::<VideoSourceDetail>()
            .all(&db),
        favorite::Entity::find()
            .select_only()
            .columns([
                favorite::Column::Id,
                favorite::Column::Name,
                favorite::Column::Path,
                favorite::Column::Rule,
                favorite::Column::FilterOption,
                favorite::Column::Enabled,
                favorite::Column::LatestRowAt,
                favorite::Column::ScheduleMode,
                favorite::Column::IntervalSeconds,
                favorite::Column::CronExpr,
                favorite::Column::NextRunAt,
                favorite::Column::LastCheckedAt,
                favorite::Column::LastFoundCount,
                favorite::Column::FailCount,
                favorite::Column::MaxDays,
                favorite::Column::MaxPages
            ])
            .into_model::<VideoSourceDetail>()
            .all(&db),
        submission::Entity::find()
            .select_only()
            .column_as(submission::Column::UpperName, "name")
            .columns([
                submission::Column::Id,
                submission::Column::Path,
                submission::Column::Enabled,
                submission::Column::Rule,
                submission::Column::FilterOption,
                submission::Column::UseDynamicApi,
                submission::Column::LatestRowAt,
                submission::Column::UpperName,
                submission::Column::ScheduleMode,
                submission::Column::IntervalSeconds,
                submission::Column::CronExpr,
                submission::Column::NextRunAt,
                submission::Column::LastCheckedAt,
                submission::Column::LastFoundCount,
                submission::Column::FailCount,
                submission::Column::MaxDays,
                submission::Column::MaxPages
            ])
            .column_as(submission::Column::UpperId, "mid")
            .into_model::<VideoSourceDetail>()
            .all(&db),
        watch_later::Entity::find()
            .select_only()
            .column_as(Expr::value("稍后再看"), "name")
            .columns([
                watch_later::Column::Id,
                watch_later::Column::Path,
                watch_later::Column::Enabled,
                watch_later::Column::Rule,
                watch_later::Column::FilterOption,
                watch_later::Column::LatestRowAt,
                watch_later::Column::ScheduleMode,
                watch_later::Column::IntervalSeconds,
                watch_later::Column::CronExpr,
                watch_later::Column::NextRunAt,
                watch_later::Column::LastCheckedAt,
                watch_later::Column::LastFoundCount,
                watch_later::Column::FailCount,
                watch_later::Column::MaxDays,
                watch_later::Column::MaxPages
            ])
            .into_model::<VideoSourceDetail>()
            .all(&db)
    )?;
    if watch_later.is_empty() {
        watch_later.push(VideoSourceDetail {
            id: 1,
            name: "稍后再看".to_string(),
            path: String::new(),
            rule: None,
            filter_option: None,
            rule_display: None,
            use_dynamic_api: None,
            enabled: false,
            latest_row_at: None,
            mid: None,
            upper_name: None,
            schedule_mode: "inherit".to_owned(),
            interval_seconds: 0,
            cron_expr: String::new(),
            next_run_at: None,
            last_checked_at: None,
            last_found_count: 0,
            fail_count: 0,
            max_days: 0,
            max_pages: 0,
        })
    }
    for sources in [&mut collections, &mut favorites, &mut submissions, &mut watch_later] {
        sources.iter_mut().for_each(|item| {
            if let Some(rule) = &item.rule {
                item.rule_display = Some(rule.to_string());
            }
            item.latest_row_at = item.latest_row_at.filter(|dt| dt.and_utc().timestamp() != 0);
        });
    }
    Ok(ApiResponse::ok(VideoSourcesDetailsResponse {
        collections,
        favorites,
        submissions,
        watch_later,
    }))
}

pub async fn get_video_sources_default_path(
    Path(source_type): Path<String>,
    Query(params): Query<DefaultPathRequest>,
) -> Result<ApiResponse<String>, ApiError> {
    let template_name = match source_type.as_str() {
        "favorites" => "favorite_default_path",
        "collections" => "collection_default_path",
        "submissions" => "submission_default_path",
        _ => return Err(InnerApiError::BadRequest("Invalid video source type".to_string()).into()),
    };
    let template = TEMPLATE.read();
    Ok(ApiResponse::ok(
        template.path_safe_render(template_name, &serde_json::to_value(params)?)?,
    ))
}

/// 更新视频来源
pub async fn update_video_source(
    Path((source_type, id)): Path<(String, i32)>,
    Extension(db): Extension<DatabaseConnection>,
    ValidatedJson(request): ValidatedJson<UpdateVideoSourceRequest>,
) -> Result<ApiResponse<UpdateVideoSourceResponse>, ApiError> {
    let rule_display = request.rule.as_ref().map(|rule| rule.to_string());
    let filter_option = request.filter_option.map(serde_json::to_value).transpose()?;
    let active_model = match source_type.as_str() {
        "collections" => collection::Entity::find_by_id(id).one(&db).await?.map(|model| {
            let mut active_model: collection::ActiveModel = model.into();
            active_model.path = Set(request.path);
            active_model.enabled = Set(request.enabled);
            active_model.rule = Set(request.rule);
            active_model.filter_option = Set(filter_option);
            _ActiveModel::Collection(active_model)
        }),
        "favorites" => favorite::Entity::find_by_id(id).one(&db).await?.map(|model| {
            let mut active_model: favorite::ActiveModel = model.into();
            active_model.path = Set(request.path);
            active_model.enabled = Set(request.enabled);
            active_model.rule = Set(request.rule);
            active_model.filter_option = Set(filter_option);
            _ActiveModel::Favorite(active_model)
        }),
        "submissions" => submission::Entity::find_by_id(id).one(&db).await?.map(|model| {
            let mut active_model: submission::ActiveModel = model.into();
            active_model.path = Set(request.path);
            active_model.enabled = Set(request.enabled);
            active_model.rule = Set(request.rule);
            active_model.filter_option = Set(filter_option);
            if let Some(use_dynamic_api) = request.use_dynamic_api {
                active_model.use_dynamic_api = Set(use_dynamic_api);
            }
            _ActiveModel::Submission(active_model)
        }),
        "watch_later" => match watch_later::Entity::find_by_id(id).one(&db).await? {
            // 稍后再看需要做特殊处理，get 时如果稍后再看不存在返回的是 id 为 1 的假记录
            // 因此此处可能是更新也可能是插入，做个额外的处理
            Some(model) => {
                // 如果有记录，使用 id 对应的记录更新
                let mut active_model: watch_later::ActiveModel = model.into();
                active_model.path = Set(request.path);
                active_model.enabled = Set(request.enabled);
                active_model.rule = Set(request.rule);
                active_model.filter_option = Set(filter_option);
                Some(_ActiveModel::WatchLater(active_model))
            }
            None => {
                if id != 1 {
                    None
                } else {
                    // 如果没有记录且 id 为 1，插入一个新的稍后再看记录
                    Some(_ActiveModel::WatchLater(watch_later::ActiveModel {
                        path: Set(request.path),
                        enabled: Set(request.enabled),
                        rule: Set(request.rule),
                        filter_option: Set(filter_option),
                        ..Default::default()
                    }))
                }
            }
        },
        _ => return Err(InnerApiError::BadRequest("Invalid video source type".to_string()).into()),
    };
    let Some(active_model) = active_model else {
        return Err(InnerApiError::NotFound(id).into());
    };
    active_model.save(&db).await?;
    Ok(ApiResponse::ok(UpdateVideoSourceResponse { rule_display }))
}

pub async fn remove_video_source(
    Path((source_type, id)): Path<(String, i32)>,
    Extension(db): Extension<DatabaseConnection>,
) -> Result<ApiResponse<bool>, ApiError> {
    // 不允许删除稍后再看
    let video_source: Option<VideoSourceEnum> = match source_type.as_str() {
        "collections" => collection::Entity::find_by_id(id).one(&db).await?.map(Into::into),
        "favorites" => favorite::Entity::find_by_id(id).one(&db).await?.map(Into::into),
        "submissions" => submission::Entity::find_by_id(id).one(&db).await?.map(Into::into),
        _ => return Err(InnerApiError::BadRequest("Invalid video source type".to_string()).into()),
    };
    let Some(video_source) = video_source else {
        return Err(InnerApiError::NotFound(id).into());
    };
    let txn = db.begin().await?;
    page::Entity::delete_many()
        .filter(
            page::Column::VideoId.in_subquery(
                video::Entity::find()
                    .filter(video_source.filter_expr())
                    .select_only()
                    .column(video::Column::Id)
                    .as_query()
                    .to_owned(),
            ),
        )
        .exec(&txn)
        .await?;
    video::Entity::delete_many()
        .filter(video_source.filter_expr())
        .exec(&txn)
        .await?;
    video_source.delete_from_db(&txn).await?;
    txn.commit().await?;
    Ok(ApiResponse::ok(true))
}

pub async fn evaluate_video_source(
    Path((source_type, id)): Path<(String, i32)>,
    Extension(db): Extension<DatabaseConnection>,
) -> Result<ApiResponse<bool>, ApiError> {
    // 找出对应 source 的规则与 video 筛选条件
    let (rule, filter_condition) = match source_type.as_str() {
        "collections" => (
            collection::Entity::find_by_id(id)
                .select_only()
                .column(collection::Column::Rule)
                .into_tuple::<Option<Rule>>()
                .one(&db)
                .await?
                .and_then(|r| r),
            video::Column::CollectionId.eq(id),
        ),
        "favorites" => (
            favorite::Entity::find_by_id(id)
                .select_only()
                .column(favorite::Column::Rule)
                .into_tuple::<Option<Rule>>()
                .one(&db)
                .await?
                .and_then(|r| r),
            video::Column::FavoriteId.eq(id),
        ),
        "submissions" => (
            submission::Entity::find_by_id(id)
                .select_only()
                .column(submission::Column::Rule)
                .into_tuple::<Option<Rule>>()
                .one(&db)
                .await?
                .and_then(|r| r),
            video::Column::SubmissionId.eq(id),
        ),
        "watch_later" => (
            watch_later::Entity::find_by_id(id)
                .select_only()
                .column(watch_later::Column::Rule)
                .into_tuple::<Option<Rule>>()
                .one(&db)
                .await?
                .and_then(|r| r),
            video::Column::WatchLaterId.eq(id),
        ),
        _ => return Err(InnerApiError::BadRequest("Invalid video source type".to_string()).into()),
    };
    let videos: Vec<(video::Model, Vec<page::Model>)> = video::Entity::find()
        .filter(filter_condition)
        .find_with_related(page::Entity)
        .all(&db)
        .await?;
    let video_should_download_pairs = videos
        .into_iter()
        .map(|(video, pages)| (video.id, rule.evaluate_model(&video, &pages)))
        .collect::<Vec<(i32, bool)>>();
    let txn = db.begin().await?;
    for chunk in video_should_download_pairs.chunks(500) {
        let sql = format!(
            "WITH tempdata(id, should_download) AS (VALUES {}) \
            UPDATE video \
            SET should_download = tempdata.should_download \
            FROM tempdata \
            WHERE video.id = tempdata.id",
            chunk.iter().map(|item| format!("({}, {})", item.0, item.1)).join(", ")
        );
        txn.execute_unprepared(&sql).await?;
    }
    txn.commit().await?;
    Ok(ApiResponse::ok(true))
}

pub async fn full_sync_video_source(
    Path((source_type, id)): Path<(String, i32)>,
    Extension(db): Extension<DatabaseConnection>,
    Extension(bili_client): Extension<Arc<BiliClient>>,
    Json(request): Json<FullSyncVideoSourceRequest>,
) -> Result<ApiResponse<FullSyncVideoSourceResponse>, ApiError> {
    let video_source: Option<VideoSourceEnum> = match source_type.as_str() {
        "collections" => collection::Entity::find_by_id(id).one(&db).await?.map(Into::into),
        "favorites" => favorite::Entity::find_by_id(id).one(&db).await?.map(Into::into),
        "submissions" => submission::Entity::find_by_id(id).one(&db).await?.map(Into::into),
        "watch_later" => watch_later::Entity::find_by_id(id).one(&db).await?.map(Into::into),
        _ => return Err(InnerApiError::BadRequest("Invalid video source type".to_string()).into()),
    };
    let Some(video_source) = video_source else {
        return Err(InnerApiError::NotFound(id).into());
    };
    let credential = &VersionedConfig::get().read().credential;
    let filter_expr = video_source.filter_expr();
    let (_, video_streams) = video_source.refresh(&bili_client, credential, &db).await?;
    let all_videos = video_streams
        .try_collect::<Vec<_>>()
        .await
        .context("failed to read all videos from video stream")?;
    let all_bvids = all_videos.into_iter().map(|v| v.bvid_owned()).collect::<HashSet<_>>();
    let videos_to_remove = video::Entity::find()
        .filter(video::Column::Bvid.is_not_in(all_bvids).and(filter_expr))
        .select_only()
        .columns([video::Column::Id, video::Column::Path])
        .into_tuple::<(i32, String)>()
        .all(&db)
        .await?;
    if videos_to_remove.is_empty() {
        return Ok(ApiResponse::ok(FullSyncVideoSourceResponse {
            removed_count: 0,
            warnings: None,
        }));
    }
    let remove_count = videos_to_remove.len();
    let (video_ids, video_paths): (Vec<i32>, Vec<String>) = videos_to_remove.into_iter().unzip();
    let txn = db.begin().await?;
    page::Entity::delete_many()
        .filter(page::Column::VideoId.is_in(video_ids.iter().copied()))
        .exec(&txn)
        .await?;
    video::Entity::delete_many()
        .filter(video::Column::Id.is_in(video_ids))
        .exec(&txn)
        .await?;
    txn.commit().await?;
    let warnings = if request.delete_local {
        let tasks = video_paths
            .into_iter()
            .filter_map(|path| {
                if path.is_empty() {
                    None
                } else {
                    Some(async move {
                        tokio::fs::remove_dir_all(&path)
                            .await
                            .with_context(|| format!("failed to remove {path}"))?;
                        Result::<_, anyhow::Error>::Ok(())
                    })
                }
            })
            .collect::<FuturesUnordered<_>>();
        Some(
            tasks
                .filter_map(|res| futures::future::ready(res.err().map(|e| format!("{:#}", e))))
                .collect::<Vec<_>>()
                .await,
        )
    } else {
        None
    };
    Ok(ApiResponse::ok(FullSyncVideoSourceResponse {
        removed_count: remove_count,
        warnings,
    }))
}

/// 新增收藏夹订阅
pub async fn insert_favorite(
    Extension(db): Extension<DatabaseConnection>,
    Extension(bili_client): Extension<Arc<BiliClient>>,
    ValidatedJson(request): ValidatedJson<InsertFavoriteRequest>,
) -> Result<ApiResponse<bool>, ApiError> {
    let credential = &VersionedConfig::get().read().credential;
    let favorite = FavoriteList::new(bili_client.as_ref(), request.fid.to_string(), credential);
    let favorite_info = favorite.get_info().await?;
    favorite::Entity::insert(favorite::ActiveModel {
        f_id: Set(favorite_info.id),
        name: Set(favorite_info.title.clone()),
        path: Set(request.path),
        enabled: Set(false),
        ..Default::default()
    })
    .exec(&db)
    .await?;
    Ok(ApiResponse::ok(true))
}

/// 新增合集/列表订阅
pub async fn insert_collection(
    Extension(db): Extension<DatabaseConnection>,
    Extension(bili_client): Extension<Arc<BiliClient>>,
    ValidatedJson(request): ValidatedJson<InsertCollectionRequest>,
) -> Result<ApiResponse<bool>, ApiError> {
    let credential = &VersionedConfig::get().read().credential;
    let collection = Collection::new(
        bili_client.as_ref(),
        CollectionItem {
            sid: request.sid.to_string(),
            mid: request.mid.to_string(),
            collection_type: request.collection_type,
        },
        credential,
    );
    let collection_info = collection.get_info().await?;
    let upper_name = Submission::new(bili_client.as_ref(), collection_info.mid.to_string(), credential)
        .get_info()
        .await
        .map(|u| u.name)
        .unwrap_or_default();
    collection::Entity::insert(collection::ActiveModel {
        s_id: Set(collection_info.sid),
        m_id: Set(collection_info.mid),
        r#type: Set(collection_info.collection_type.into()),
        name: Set(collection_info.name.clone()),
        path: Set(request.path),
        enabled: Set(false),
        upper_name: Set(upper_name),
        ..Default::default()
    })
    .exec(&db)
    .await?;

    Ok(ApiResponse::ok(true))
}

/// 新增投稿订阅
pub async fn insert_submission(
    Extension(db): Extension<DatabaseConnection>,
    Extension(bili_client): Extension<Arc<BiliClient>>,
    ValidatedJson(request): ValidatedJson<InsertSubmissionRequest>,
) -> Result<ApiResponse<bool>, ApiError> {
    let credential = &VersionedConfig::get().read().credential;
    let submission = Submission::new(bili_client.as_ref(), request.upper_id.to_string(), credential);
    let upper = submission.get_info().await?;
    submission::Entity::insert(submission::ActiveModel {
        upper_id: Set(upper.mid.parse()?),
        upper_name: Set(upper.name),
        path: Set(request.path),
        enabled: Set(false),
        ..Default::default()
    })
    .exec(&db)
    .await?;
    Ok(ApiResponse::ok(true))
}

/// 立即执行某个订阅：拉取新视频 → 写库 → 下载（等价于对该订阅跑一轮完整流程）
pub async fn run_video_source(
    Path((source_type, id)): Path<(String, i32)>,
    Extension(db): Extension<DatabaseConnection>,
    Extension(bili_client): Extension<Arc<BiliClient>>,
) -> Result<ApiResponse<bool>, ApiError> {
    let video_source: Option<VideoSourceEnum> = match source_type.as_str() {
        "collections" => collection::Entity::find_by_id(id).one(&db).await?.map(Into::into),
        "favorites" => favorite::Entity::find_by_id(id).one(&db).await?.map(Into::into),
        "submissions" => submission::Entity::find_by_id(id).one(&db).await?.map(Into::into),
        "watch_later" => watch_later::Entity::find_by_id(id).one(&db).await?.map(Into::into),
        _ => return Err(InnerApiError::BadRequest("Invalid video source type".to_string()).into()),
    };
    let Some(video_source) = video_source else {
        return Err(InnerApiError::NotFound(id).into());
    };
    let config = VersionedConfig::get().snapshot();
    let template = TEMPLATE.read();
    crate::workflow::process_video_source(video_source, bili_client.as_ref(), &db, &template, &config).await?;
    Ok(ApiResponse::ok(true))
}

/// 补全历史订阅缺失的 UP 名（二开新增）
pub async fn backfill_upper_names(
    Extension(db): Extension<DatabaseConnection>,
    Extension(bili_client): Extension<Arc<BiliClient>>,
) -> Result<ApiResponse<usize>, ApiError> {
    let credential = &VersionedConfig::get().read().credential;
    let rows = collection::Entity::find()
        .filter(collection::Column::UpperName.eq(""))
        .all(&db)
        .await?;
    let mut fixed = 0usize;
    for model in rows {
        if let Ok(upper) = Submission::new(bili_client.as_ref(), model.m_id.to_string(), credential)
            .get_info()
            .await
        {
            let mut active = model.into_active_model();
            active.upper_name = Set(upper.name);
            active.update(&db).await?;
            fixed += 1;
        }
    }
    info!("已补全 {} 个订阅的 UP 名", fixed);
    Ok(ApiResponse::ok(fixed))
}

#[derive(Debug, Deserialize)]
pub struct ScheduleForm {
    /// inherit / interval / cron / disabled
    pub schedule_mode: String,
    #[serde(default)]
    pub interval_seconds: i32,
    #[serde(default)]
    pub cron_expr: String,
    #[serde(default)]
    pub max_days: i32,
    #[serde(default)]
    pub max_pages: i32,
}

/// 更新订阅的刷新调度与刷新规则（二开新增）
pub async fn update_schedule(
    Path((source_type, id)): Path<(String, i32)>,
    Extension(db): Extension<DatabaseConnection>,
    Json(form): Json<ScheduleForm>,
) -> Result<ApiResponse<bool>, ApiError> {
    let table = match source_type.as_str() {
        "collections" => "collection",
        "favorites" => "favorite",
        "submissions" => "submission",
        "watch_later" => "watch_later",
        _ => return Err(InnerApiError::BadRequest("Invalid video source type".to_string()).into()),
    };
    let mode = form.schedule_mode.trim();
    if !crate::utils::schedule::MODES.contains(&mode) {
        return Err(InnerApiError::BadRequest(format!("不支持的调度模式：{mode}")).into());
    }
    let next_run = crate::utils::schedule::next_run_at(mode, form.interval_seconds, &form.cron_expr)?;

    let sql = format!(
        "UPDATE {table} SET schedule_mode = ?, interval_seconds = ?, cron_expr = ?, next_run_at = ?, \
         max_days = ?, max_pages = ? WHERE id = ?"
    );
    let stmt = sea_orm::Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        [
            sea_orm::Value::from(mode),
            sea_orm::Value::from(form.interval_seconds),
            sea_orm::Value::from(form.cron_expr.trim()),
            sea_orm::Value::from(next_run),
            sea_orm::Value::from(form.max_days),
            sea_orm::Value::from(form.max_pages),
            sea_orm::Value::from(id),
        ],
    );
    db.execute(stmt).await?;
    info!("订阅 {table}#{id} 调度已更新：mode={mode} next_run={next_run:?}");
    Ok(ApiResponse::ok(true))
}

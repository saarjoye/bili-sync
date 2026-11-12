use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use sea_orm::{DatabaseConnection, EntityTrait};
use serde::Serialize;
use tokio::sync::{OnceCell, watch};
use tokio_cron_scheduler::{Job, JobScheduler};

use crate::adapter::VideoSource;
use crate::bilibili::{self, BiliClient, BiliError};
use crate::config::{ARGS, Config, TEMPLATE, Trigger, VersionedConfig};
use crate::utils::model::get_enabled_video_sources;
use crate::utils::schedule::next_run_at;
use crate::utils::notify::error_and_notify;
use crate::workflow::process_video_source;

static INSTANCE: OnceCell<DownloadTaskManager> = OnceCell::const_new();

/// 启动周期下载视频的任务
pub async fn video_downloader(connection: DatabaseConnection, bili_client: Arc<BiliClient>) -> Result<()> {
    let task_manager = DownloadTaskManager::init(connection, bili_client).await?;
    task_manager.start().await
}

pub struct DownloadTaskManager {
    sched: Arc<tokio::sync::Mutex<JobScheduler>>,
    cx: Arc<TaskContext>,
    shutdown_rx: watch::Receiver<Result<()>>,
}

#[derive(Serialize, Default, Clone, Copy, Debug)]
pub struct TaskStatus {
    is_running: bool,
    last_run: Option<chrono::DateTime<chrono::Local>>,
    last_finish: Option<chrono::DateTime<chrono::Local>>,
    next_run: Option<chrono::DateTime<chrono::Local>>,
}

struct TaskContext {
    connection: DatabaseConnection,
    bili_client: Arc<BiliClient>,
    running: tokio::sync::Mutex<()>,
    status_tx: watch::Sender<TaskStatus>,
    status_rx: watch::Receiver<TaskStatus>,
    video_task_id: tokio::sync::Mutex<Option<uuid::Uuid>>, // 存储当前视频下载任务的 UUID
}

impl DownloadTaskManager {
    /// 初始化 DownloadTaskManager 单例
    pub async fn init(
        connection: DatabaseConnection,
        bili_client: Arc<BiliClient>,
    ) -> Result<&'static DownloadTaskManager> {
        INSTANCE
            .get_or_try_init(|| DownloadTaskManager::new(connection, bili_client))
            .await
    }

    /// 获取 DownloadTaskManager 单例，未初始化时直接 panic
    pub fn get() -> &'static DownloadTaskManager {
        INSTANCE.get().expect("DownloadTaskManager is not initialized")
    }

    /// 订阅下载任务的状态更新
    pub fn subscribe(&self) -> watch::Receiver<TaskStatus> {
        self.cx.status_rx.clone()
    }

    /// 手动执行一次下载任务
    pub async fn download_once(&self) -> Result<()> {
        let _ = self
            .sched
            .lock()
            .await
            .add(Job::new_one_shot_async(
                Duration::from_secs(0),
                DownloadTaskManager::download_video_task(self.cx.clone()),
            )?)
            .await?;
        Ok(())
    }

    /// 启动任务调度器
    async fn start(&self) -> Result<()> {
        self.sched.lock().await.start().await?;
        let mut shutdown_rx = self.shutdown_rx.clone();
        shutdown_rx.changed().await?;
        self.sched.lock().await.shutdown().await.context("任务调度器关闭失败")?;
        if let Err(e) = &*shutdown_rx.borrow() {
            bail!("{:#}", e);
        }
        Ok(())
    }

    /// 私有的调度器构造函数
    async fn new(connection: DatabaseConnection, bili_client: Arc<BiliClient>) -> Result<Self> {
        let sched = Arc::new(tokio::sync::Mutex::new(JobScheduler::new().await?));
        let (status_tx, status_rx) = watch::channel(TaskStatus::default());
        let (running, video_task_id) = (tokio::sync::Mutex::new(()), tokio::sync::Mutex::new(None));
        let cx = Arc::new(TaskContext {
            connection,
            bili_client,
            running,
            status_tx,
            status_rx,
            video_task_id,
        });
        // 读取初始配置
        let mut rx = VersionedConfig::get().subscribe();
        let initial_config = rx.borrow_and_update().clone();
        if ARGS.disable_credential_refresh {
            warn!("已禁用凭据检查与刷新任务，bili-sync 将不会自动检查刷新 Credential，需要用户自行维护");
        } else {
            // 初始化凭据检查与刷新任务，该任务必须成功，否则直接退出
            sched
                .lock()
                .await
                .add(Job::new_async_tz(
                    "0 0 1 * * *",
                    chrono::Local,
                    DownloadTaskManager::check_and_refresh_credential_task(cx.clone()),
                )?)
                .await?;
        }
        // 初始化并添加视频下载任务，将任务 ID 保存到 TaskManager 中
        let video_task_id = async {
            let job_run = DownloadTaskManager::download_video_task(cx.clone());
            let job = match &initial_config.interval {
                Trigger::Interval(interval) => Job::new_repeated_async(Duration::from_secs(*interval), job_run)?,
                Trigger::Cron(cron) => Job::new_async_tz(cron, chrono::Local, job_run)?,
            };
            Result::<_, anyhow::Error>::Ok(sched.lock().await.add(job).await?)
        }
        .await;
        let video_task_id = match video_task_id {
            Ok(id) => Some(id),
            Err(err) => {
                error_and_notify(
                    &initial_config,
                    &cx.bili_client,
                    format!("初始化视频下载任务失败：{:#}", err),
                    &err,
                );
                None
            }
        };
        *cx.video_task_id.lock().await = video_task_id;
        // 二开：订阅级调度（每分钟检查一次是否有到期的订阅需要单独刷新）
        sched
            .lock()
            .await
            .add(Job::new_repeated_async(
                Duration::from_secs(60),
                subscription_schedule_task(cx.clone()),
            )?)
            .await?;
        // 发起一个一次性的任务，更新一下下次运行的时间
        if let Some(video_task_id) = video_task_id {
            sched
                .lock()
                .await
                .add(Job::new_one_shot_async(
                    Duration::from_secs(0),
                    DownloadTaskManager::refresh_next_run(video_task_id, cx.clone()),
                )?)
                .await?;
        }
        // 发起一个新任务，用来监听配置变更，动态更新视频下载任务
        let cx_clone = cx.clone();
        let sched_clone = sched.clone();
        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(Ok(()));
        tokio::spawn(async move {
            let update_task_result = async {
                while rx.changed().await.is_ok() {
                    let new_config = rx.borrow().clone();
                    let cx = cx_clone.clone();
                    let mut video_task_id = cx.video_task_id.lock().await;
                    if let Some(old_video_task_id) = *video_task_id {
                        // 这里必须成功，不然后面会重复添加任务
                        sched_clone
                            .lock()
                            .await
                            .remove(&old_video_task_id)
                            .await
                            .context("移除旧的视频下载任务失败")?;
                    }
                    let new_video_task_id = async {
                        let job_run = DownloadTaskManager::download_video_task(cx.clone());
                        let job = match &new_config.interval {
                            Trigger::Interval(interval) => {
                                Job::new_repeated_async(Duration::from_secs(*interval), job_run)?
                            }
                            Trigger::Cron(cron) => Job::new_async_tz(cron, chrono::Local, job_run)?,
                        };
                        Result::<_, anyhow::Error>::Ok(sched_clone.lock().await.add(job).await?)
                    }
                    .await;
                    let new_video_task_id = match new_video_task_id {
                        Ok(id) => Some(id),
                        Err(err) => {
                            error_and_notify(
                                &initial_config,
                                &cx.bili_client,
                                format!("重载视频下载任务失败：{:#}", err),
                                &err,
                            );
                            None
                        }
                    };
                    *video_task_id = new_video_task_id;
                    if let Some(video_task_id) = new_video_task_id {
                        sched_clone
                            .lock()
                            .await
                            .add(Job::new_one_shot_async(
                                Duration::from_secs(0),
                                DownloadTaskManager::refresh_next_run(video_task_id, cx.clone()),
                            )?)
                            .await?;
                    }
                }
                Result::<(), anyhow::Error>::Ok(())
            }
            .await;
            // 如果执行正常，上面应该是永远不会退出的
            let _ = shutdown_tx.send(update_task_result);
        });
        Ok(Self { sched, cx, shutdown_rx })
    }

    fn check_and_refresh_credential_task(
        cx: Arc<TaskContext>,
    ) -> impl FnMut(uuid::Uuid, JobScheduler) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        move |_uuid, _l| {
            let cx = cx.clone();
            Box::pin(async move {
                let _lock = cx.running.lock().await;
                let config = VersionedConfig::get().read();
                info!("开始执行本轮凭据检查与刷新任务..");
                match check_and_refresh_credential(&cx.connection, &cx.bili_client, &config).await {
                    Ok(_) => info!("本轮凭据检查与刷新任务执行完毕"),
                    Err(e) => {
                        error_and_notify(
                            &config,
                            &cx.bili_client,
                            format!("本轮凭据检查与刷新任务执行遇到错误：{:#}", e),
                            &e,
                        );
                    }
                }
            })
        }
    }

    fn refresh_next_run(
        video_task_id: uuid::Uuid,
        cx: Arc<TaskContext>,
    ) -> impl FnMut(uuid::Uuid, JobScheduler) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        move |_uuid, mut l| {
            let cx = cx.clone();
            Box::pin(async move {
                let old_status = *cx.status_rx.borrow();
                let next_run = l
                    .next_tick_for_job(video_task_id)
                    .await
                    .ok()
                    .flatten()
                    .map(|dt| dt.with_timezone(&chrono::Local));
                let _ = cx.status_tx.send(TaskStatus { next_run, ..old_status });
            })
        }
    }

    fn download_video_task(
        cx: Arc<TaskContext>,
    ) -> impl FnMut(uuid::Uuid, JobScheduler) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        move |uuid, mut l| {
            let cx = cx.clone();
            Box::pin(async move {
                let Ok(_lock) = cx.running.try_lock() else {
                    warn!("上一次视频下载任务尚未结束，跳过本次执行..");
                    return;
                };
                let _ = cx.status_tx.send(TaskStatus {
                    is_running: true,
                    last_run: Some(chrono::Local::now()),
                    last_finish: None,
                    next_run: None,
                });
                info!("开始执行本轮视频下载任务..");
                let mut config = VersionedConfig::get().snapshot();
                match download_video(&cx.connection, &cx.bili_client, &mut config).await {
                    Ok(_) => info!("本轮视频下载任务执行完毕"),
                    Err(e) => {
                        error_and_notify(
                            &config,
                            &cx.bili_client,
                            format!("本轮视频下载任务执行遇到错误：{:#}", e),
                            &e,
                        );
                    }
                }
                // 注意此处尽量从 updating 中读取 uuid，因为当前任务可能是不存在 next_tick 的 oneshot 任务
                let task_uuid = (*cx.video_task_id.lock().await).unwrap_or(uuid);
                let next_run = l
                    .next_tick_for_job(task_uuid)
                    .await
                    .ok()
                    .flatten()
                    .map(|dt| dt.with_timezone(&chrono::Local));
                let last_status = *cx.status_rx.borrow();
                let _ = cx.status_tx.send(TaskStatus {
                    is_running: false,
                    last_run: last_status.last_run,
                    last_finish: Some(chrono::Local::now()),
                    next_run,
                });
            })
        }
    }
}

async fn check_and_refresh_credential(
    connection: &DatabaseConnection,
    bili_client: &BiliClient,
    config: &Config,
) -> Result<()> {
    match bili_client
        .check_refresh(&config.credential)
        .await
        .context("检查刷新 Credential 失败")?
    {
        None => {
            info!("Credential 无需刷新");
        }
        Some(new_credential) => {
            VersionedConfig::get()
                .update_credential(new_credential, connection)
                .await
                .context("新 Credential 持久化失败")?;
            info!("Credential 已刷新并保存");
        }
    }
    Ok(())
}

async fn download_video(
    connection: &DatabaseConnection,
    bili_client: &BiliClient,
    config: &mut Arc<Config>,
) -> Result<()> {
    config.check().context("配置检查失败")?;
    let mixin_key = bili_client
        .wbi_img(&config.credential)
        .await
        .context("获取 wbi_img 失败")?
        .into_mixin_key()
        .context("解析 mixin key 失败")?;
    bilibili::set_global_mixin_key(mixin_key);
    let template = TEMPLATE.snapshot();
    let bili_client = bili_client.snapshot()?;
    let video_sources = get_enabled_video_sources(connection)
        .await
        .context("获取视频源列表失败")?;
    if video_sources.is_empty() {
        bail!("没有可用的视频源");
    }
    let mut processed = 0usize;
    for video_source in video_sources {
        processed += 1;
        let display_name = video_source.display_name();
        crate::utils::notify::notify(
            config,
            &bili_client,
            format!("🔍 开始处理订阅：{}", video_source.display_name()),
        );
        if let Err(e) = process_video_source(video_source, &bili_client, connection, &template, config).await {
            error_and_notify(
                config,
                &bili_client,
                format!("处理 {} 时遇到错误：{:#}，跳过该视频源", display_name, e),
                &e,
            );
            if let Ok(e) = e.downcast::<BiliError>()
                && e.is_risk_control_related()
            {
                warn!("检测到风控，终止此轮视频下载任务..");
                break;
            }
        }
    }
    crate::utils::notify::notify(
        config,
        &bili_client,
        format!("✅ 本轮订阅检查完成，共处理 {} 个视频源", processed),
    );
    Ok(())
}

/// 每分钟检查一次：有没有「按间隔 / 按 cron」到期的订阅需要单独刷新
fn subscription_schedule_task(
    cx: Arc<TaskContext>,
) -> impl FnMut(uuid::Uuid, JobScheduler) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    move |_uuid, _l| {
        let cx = cx.clone();
        Box::pin(async move {
            if let Err(e) = tick_due_subscriptions(&cx).await {
                error!("订阅调度任务出错：{:#}", e);
            }
        })
    }
}

/// 单订阅的表名与实体类型
const SCHEDULE_TABLES: [&str; 4] = ["collection", "favorite", "submission", "watch_later"];

async fn tick_due_subscriptions(cx: &TaskContext) -> Result<()> {
    use sea_orm::{ConnectionTrait, Statement, Value};

    let config = VersionedConfig::get().snapshot();
    let template = TEMPLATE.read();
    let client = cx.bili_client.snapshot()?;
    let now = chrono::Local::now().naive_local();

    for table in SCHEDULE_TABLES {
        let sql = format!(
            "SELECT id FROM {table} WHERE enabled = 1 AND schedule_mode IN ('interval', 'cron') \
             AND (next_run_at IS NULL OR next_run_at <= ?)"
        );
        let stmt = Statement::from_sql_and_values(
            cx.connection.get_database_backend(),
            sql,
            [Value::from(now)],
        );
        let rows = cx.connection.query_all(stmt).await?;
        let mut ids = Vec::new();
        for row in rows {
            if let Ok(id) = row.try_get::<i32>("", "id") {
                ids.push(id);
            }
        }
        for id in ids {
            let Some(source) = load_video_source(&cx.connection, table, id).await? else {
                continue;
            };
            let source_name = source.display_name().to_string();
            info!("按订阅调度刷新：{}", source_name);
            match process_video_source(source, &client, &cx.connection, &template, &config).await {
                Ok(_) => {
                    info!("订阅刷新完成：{}", source_name);
                    finish_tick(&cx.connection, table, id, now, true).await?;
                    apply_max_days(&cx.connection, table, id).await?;
                }
                Err(e) => {
                    error!("订阅刷新失败：{}：{:#}", source_name, e);
                    finish_tick(&cx.connection, table, id, now, false).await?;
                    // 连续失败 3 次发通知
                    let fails = current_fail_count(&cx.connection, table, id).await?;
                    if fails > 0 && fails % 3 == 0 {
                        let msg = format!("订阅「{}」已连续失败 {} 次，最近错误：{:#}", source_name, fails, e);
                        crate::utils::notify::notify(&config, &client, msg);
                    }
                }
            }
        }
    }
    Ok(())
}

async fn load_video_source(
    db: &DatabaseConnection,
    table: &str,
    id: i32,
) -> Result<Option<crate::adapter::VideoSourceEnum>> {
    use bili_sync_entity::*;
    let source = match table {
        "collection" => collection::Entity::find_by_id(id).one(db).await?.map(Into::into),
        "favorite" => favorite::Entity::find_by_id(id).one(db).await?.map(Into::into),
        "submission" => submission::Entity::find_by_id(id).one(db).await?.map(Into::into),
        "watch_later" => watch_later::Entity::find_by_id(id).one(db).await?.map(Into::into),
        _ => None,
    };
    Ok(source)
}

/// 更新上次检查时间、失败计数与下次运行时间
async fn finish_tick(
    db: &DatabaseConnection,
    table: &str,
    id: i32,
    now: chrono::NaiveDateTime,
    success: bool,
) -> Result<()> {
    use sea_orm::{ConnectionTrait, Statement, Value};

    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        format!("SELECT schedule_mode, interval_seconds, cron_expr FROM {table} WHERE id = ?"),
        [Value::from(id)],
    );
    let Some(row) = db.query_one(stmt).await? else {
        return Ok(());
    };
    let mode: String = row.try_get("", "schedule_mode").unwrap_or_else(|_| "inherit".to_owned());
    let interval: i32 = row.try_get("", "interval_seconds").unwrap_or(0);
    let cron: String = row.try_get("", "cron_expr").unwrap_or_default();
    let next = next_run_at(&mode, interval.max(300), &cron).unwrap_or(None);

    let fail_sql = if success {
        String::from("fail_count = 0,")
    } else {
        String::from("fail_count = fail_count + 1,")
    };
    let sql = format!(
        "UPDATE {table} SET last_checked_at = ?, {fail_sql} next_run_at = ? WHERE id = ?"
    );
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        [Value::from(now), Value::from(next), Value::from(id)],
    );
    db.execute(stmt).await?;
    Ok(())
}

async fn current_fail_count(db: &DatabaseConnection, table: &str, id: i32) -> Result<i64> {
    use sea_orm::{ConnectionTrait, Statement, Value};
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        format!("SELECT fail_count FROM {table} WHERE id = ?"),
        [Value::from(id)],
    );
    let value = db
        .query_one(stmt)
        .await?
        .and_then(|row| row.try_get::<i32>("", "fail_count").ok())
        .unwrap_or(0);
    Ok(value as i64)
}

/// 刷新规则：只看最近 N 天（0 表示不限）——超期视频标记为不下载
async fn apply_max_days(db: &DatabaseConnection, table: &str, id: i32) -> Result<()> {
    use sea_orm::{ConnectionTrait, Statement, Value};

    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        format!("SELECT max_days FROM {table} WHERE id = ?"),
        [Value::from(id)],
    );
    let max_days = db
        .query_one(stmt)
        .await?
        .and_then(|row| row.try_get::<i32>("", "max_days").ok())
        .unwrap_or(0);
    if max_days <= 0 {
        return Ok(());
    }
    let cutoff = chrono::Local::now().naive_local() - chrono::Duration::days(max_days as i64);
    let fk = match table {
        "collection" => "collection_id",
        "favorite" => "favorite_id",
        "submission" => "submission_id",
        _ => "watch_later_id",
    };
    let sql = format!(
        "UPDATE video SET should_download = 0 WHERE {fk} = ? AND download_status = 0 AND pubtime < ?"
    );
    let stmt = Statement::from_sql_and_values(
        db.get_database_backend(),
        sql,
        [Value::from(id), Value::from(cutoff)],
    );
    db.execute(stmt).await?;
    Ok(())
}

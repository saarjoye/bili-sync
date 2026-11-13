use anyhow::Error;

use crate::bilibili::{BiliClient, BiliError};
use crate::config::Config;
use crate::notifier::{Message, NotifierAllExt};

/// 统一发送入口（不做开关判断）
fn send(config: &Config, bili_client: &BiliClient, msg: impl Into<Message<'static>>) {
    if let Some(notifiers) = &config.notifiers
        && !notifiers.is_empty()
    {
        let (notifiers, inner_client) = (notifiers.clone(), bili_client.inner_client().clone());
        let msg = msg.into();
        tokio::spawn(async move { notifiers.notify_all(&inner_client, msg).await });
    }
}

/// 下载成功 / 新视频入库类通知（受「下载成功」开关控制）
pub fn notify(config: &Config, bili_client: &BiliClient, msg: impl Into<Message<'static>>) {
    if !config.notify_download_success {
        return;
    }
    send(config, bili_client, msg);
}

/// 订阅检查的进度类通知（开始处理 / 本轮完成），默认关闭，避免每轮刷屏
pub fn notify_progress(config: &Config, bili_client: &BiliClient, msg: impl Into<Message<'static>>) {
    if !config.notify_progress {
        return;
    }
    send(config, bili_client, msg);
}

/// 失败 / 异常类通知（受「下载失败」开关控制）
pub fn notify_error(config: &Config, bili_client: &BiliClient, msg: impl Into<Message<'static>>) {
    if !config.notify_download_error {
        return;
    }
    send(config, bili_client, msg);
}

pub fn error_and_notify(config: &Config, bili_client: &BiliClient, msg: String, error: &Error) {
    error!("{msg}");
    if config.ignore_common_errors
        && error.chain().any(|cause| {
            cause
                .downcast_ref::<BiliError>()
                .is_some_and(BiliError::is_common_error)
        })
    {
        return;
    }
    notify_error(config, bili_client, msg);
}

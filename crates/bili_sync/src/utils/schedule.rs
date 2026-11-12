//! 订阅刷新调度：计算下次运行时间（二开新增）

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Duration, Local, NaiveDateTime};

/// 允许的调度模式
pub const MODES: [&str; 4] = ["inherit", "interval", "cron", "disabled"];

/// 计算下次运行时间（本地时间）
pub fn next_run_at(mode: &str, interval_seconds: i32, cron_expr: &str) -> Result<Option<NaiveDateTime>> {
    match mode {
        "interval" => {
            if interval_seconds < 60 {
                bail!("间隔不能小于 60 秒");
            }
            Ok(Some((Local::now() + Duration::seconds(interval_seconds as i64)).naive_local()))
        }
        "cron" => {
            if cron_expr.trim().is_empty() {
                bail!("Cron 表达式不能为空");
            }
            let cron: croner::Cron = cron_expr
                .trim()
                .parse()
                .map_err(|e| anyhow!("Cron 表达式解析失败：{e}"))?;
            let next: DateTime<Local> = cron
                .find_next_occurrence(&Local::now(), false)
                .map_err(|e| anyhow!("计算下次运行时间失败：{e}"))?;
            Ok(Some(next.naive_local()))
        }
        _ => Ok(None),
    }
}

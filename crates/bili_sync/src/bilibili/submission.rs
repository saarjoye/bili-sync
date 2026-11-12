use anyhow::{Context, Result, anyhow};
use async_stream::try_stream;
use bili_sync_entity::upper_vec::Upper;
use futures::Stream;
use reqwest::Method;
use serde_json::Value;

use crate::bilibili::{BiliClient, Credential, Dynamic, ErrorForStatusExt, MIXIN_KEY, Validate, VideoInfo, WbiSign};

const SEASON_ARCHIVES: &str = "https://api.bilibili.com/x/polymer/web-space/seasons_archives_list";

/// UP 名下的一个合集（season）
#[derive(Debug, Clone)]
pub struct SeasonSummary {
    pub sid: i64,
    pub name: String,
    pub count: i64,
}

pub struct Submission<'a> {
    client: &'a BiliClient,
    pub upper_id: String,
    credential: &'a Credential,
}

impl<'a> From<Submission<'a>> for Dynamic<'a> {
    fn from(submission: Submission<'a>) -> Self {
        Dynamic::new(submission.client, submission.upper_id, submission.credential)
    }
}

impl<'a> Submission<'a> {
    pub fn new(client: &'a BiliClient, upper_id: String, credential: &'a Credential) -> Self {
        Self {
            client,
            upper_id,
            credential,
        }
    }

    pub async fn get_info(&self) -> Result<Upper<String, String>> {
        let mut res = self
            .client
            .request(
                Method::GET,
                "https://api.bilibili.com/x/web-interface/card",
                self.credential,
            )
            .await
            .query(&[("mid", self.upper_id.as_str())])
            .send()
            .await?
            .error_for_status_ext()?
            .json::<serde_json::Value>()
            .await?
            .validate()?;
        Ok(serde_json::from_value(res["data"]["card"].take())?)
    }

    /// 列出该 UP 名下的全部「合集」（两步发现：先翻投稿收集 season_id，再逐个查元信息）
    ///
    /// 上游的 `seasons_series_list` 对无浏览器指纹的请求常返回 -400，因此这里走两步。
    /// 注意：只能发现「合集」，B站 的「系列/列表」目前没有稳定接口，需要手动提供 sid。
    pub async fn list_seasons(&self) -> Result<Vec<SeasonSummary>> {
        // 空间接口需要 wbi 签名；全局 MIXIN_KEY 只在下载任务启动时初始化，
        // 这里兜底一次，保证「订阅向导」在首次使用（还没跑过下载任务）时也能解析。
        if MIXIN_KEY.load().is_none() {
            let key = self
                .client
                .wbi_img(self.credential)
                .await?
                .into_mixin_key()
                .context("获取 wbi mixin key 失败")?;
            crate::bilibili::set_global_mixin_key(key);
        }

        let mut season_ids: Vec<i64> = Vec::new();
        for page in 1..=3 {
            let mut videos = self
                .get_videos(page)
                .await
                .with_context(|| format!("拉取 UP {} 第 {} 页投稿失败", self.upper_id, page))?;
            let vlist = &mut videos["data"]["list"]["vlist"];
            let Some(arr) = vlist.as_array() else { break };
            if arr.is_empty() {
                break;
            }
            for v in arr {
                if let Some(sid) = v["season_id"].as_i64()
                    && sid > 0
                    && !season_ids.contains(&sid)
                {
                    season_ids.push(sid);
                }
            }
            let count = videos["data"]["page"]["count"].as_i64().unwrap_or(0);
            if count <= (page * 30) as i64 {
                break;
            }
        }

        let mut result = Vec::new();
        for sid in season_ids {
            let params = vec![
                ("mid".to_owned(), self.upper_id.clone()),
                ("season_id".to_owned(), sid.to_string()),
                ("sort_reverse".to_owned(), "false".to_owned()),
                ("page_num".to_owned(), "1".to_owned()),
                ("page_size".to_owned(), "1".to_owned()),
            ];
            let res = self
                .client
                .request(Method::GET, SEASON_ARCHIVES, self.credential)
                .await
                .query(&params)
                .send()
                .await?
                .error_for_status_ext()?
                .json::<serde_json::Value>()
                .await?;
            let meta = &res["data"]["meta"];
            let Some(name) = meta["name"].as_str() else { continue };
            if name.is_empty() {
                continue;
            }
            result.push(SeasonSummary {
                sid,
                name: name.to_owned(),
                count: meta["total"].as_i64().unwrap_or(0),
            });
        }
        Ok(result)
    }

    async fn get_videos(&self, page: i32) -> Result<Value> {
        self.client
            .request(
                Method::GET,
                "https://api.bilibili.com/x/space/wbi/arc/search",
                self.credential,
            )
            .await
            .query(&[
                ("mid", self.upper_id.as_str()),
                ("order", "pubdate"),
                ("order_avoided", "true"),
                ("platform", "web"),
                ("web_location", "1550101"),
                ("ps", "30"),
            ])
            .query(&[("pn", page)])
            .wbi_sign(MIXIN_KEY.load().as_deref())?
            .send()
            .await?
            .error_for_status_ext()?
            .json::<serde_json::Value>()
            .await?
            .validate()
    }

    pub fn into_video_stream(self) -> impl Stream<Item = Result<VideoInfo>> + 'a {
        try_stream! {
            let mut page = 1;
            loop {
                let mut videos = self
                    .get_videos(page)
                    .await
                    .with_context(|| format!("failed to get videos of upper {} page {}", self.upper_id, page))?;
                let vlist = &mut videos["data"]["list"]["vlist"];
                if vlist.as_array().is_none_or(|v| v.is_empty()) {
                    if page == 1 {
                        break;
                    }
                    Err(anyhow!("no medias found in upper {} page {}", self.upper_id, page))?;
                }
                let videos_info: Vec<VideoInfo> = serde_json::from_value(vlist.take())
                    .with_context(|| format!("failed to parse videos of upper {} page {}", self.upper_id, page))?;
                for video_info in videos_info {
                    yield video_info;
                }
                let count = &videos["data"]["page"]["count"];
                if let Some(v) = count.as_i64() {
                    if v > (page * 30) as i64 {
                        page += 1;
                        continue;
                    }
                } else {
                    Err(anyhow!("count is not an i64"))?;
                }
                break;
            }
        }
    }
}

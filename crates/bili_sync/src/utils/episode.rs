//! 集号与季号的推导（二开新增）
//!
//! - 集号 `ep`：合集内按发布时间升序排名，1 开始，渲染成两位（01、02…）
//! - 季号 `season`：从订阅名/合集名里识别「初一/初二/初三」「上册/下册」「第N季」

use bili_sync_entity::video;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};

/// 计算某视频在其所属合集内的集号（按 pubtime 升序，1 开始）
pub async fn episode_no<C>(conn: &C, model: &video::Model) -> Option<i64>
where
    C: ConnectionTrait,
{
    let collection_id = model.collection_id?;
    let earlier = video::Entity::find()
        .filter(video::Column::CollectionId.eq(collection_id))
        .filter(video::Column::Pubtime.lt(model.pubtime))
        .all(conn)
        .await
        .ok()?
        .len();
    Some(earlier as i64 + 1)
}

/// 两位数的集号字符串（01、02…）；没有集号时返回空串
pub fn format_ep(ep: Option<i64>) -> String {
    match ep {
        Some(n) if n > 0 => format!("{n:02}"),
        _ => String::new(),
    }
}

/// 从名称里识别季号，返回两位数字符串
pub fn season_of(name: &str) -> Option<String> {
    for (idx, key) in ["初一", "初二", "初三"].iter().enumerate() {
        if name.contains(key) {
            return Some(format!("{:02}", idx + 1));
        }
    }
    if name.contains("上册") {
        return Some("01".to_owned());
    }
    if name.contains("下册") {
        return Some("02".to_owned());
    }
    // 第N季 / 第N期
    for marker in ["第", "季", "期"] {
        let _ = marker;
    }
    let chars: Vec<char> = name.chars().collect();
    for i in 0..chars.len() {
        if chars[i] == '第' {
            let mut num = String::new();
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_ascii_digit() || ('一'..='九').contains(&chars[j])) {
                num.push(chars[j]);
                j += 1;
            }
            if !num.is_empty() && j < chars.len() && (chars[j] == '季' || chars[j] == '期') {
                let value = if num.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                    num.parse::<i64>().ok()
                } else {
                    Some(chinese_to_num(&num))
                };
                if let Some(v) = value {
                    return Some(format!("{v:02}"));
                }
            }
        }
    }
    None
}

fn chinese_to_num(s: &str) -> i64 {
    let digits = ["一", "二", "三", "四", "五", "六", "七", "八", "九"];
    let mut total = 0i64;
    let mut current = 0i64;
    for ch in s.chars() {
        if ch == '十' {
            total += if current == 0 { 1 } else { current } * 10;
            current = 0;
        } else if let Some(pos) = digits.iter().position(|d| d.chars().next() == Some(ch)) {
            current = pos as i64 + 1;
        }
    }
    total + current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_season_of() {
        assert_eq!(season_of("合集·初一数学"), Some("01".to_owned()));
        assert_eq!(season_of("初二 物理"), Some("02".to_owned()));
        assert_eq!(season_of("初三冲刺"), Some("03".to_owned()));
        assert_eq!(season_of("数学上册"), Some("01".to_owned()));
        assert_eq!(season_of("数学下册"), Some("02".to_owned()));
        assert_eq!(season_of("第3季 语法"), Some("03".to_owned()));
        assert_eq!(season_of("第十二季"), Some("12".to_owned()));
        assert_eq!(season_of("合集·初中数学大合集"), None);
    }

    #[test]
    fn test_format_ep() {
        assert_eq!(format_ep(Some(1)), "01");
        assert_eq!(format_ep(Some(12)), "12");
        assert_eq!(format_ep(None), "");
    }
}

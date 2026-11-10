//! 账号密码认证：用户表读写、密码哈希、内存会话。
//!
//! 设计取舍：
//! - 只做「单用户」登录（首次进入创建账号），够用且实现最小；
//! - 密码用 SHA-256 + 随机盐 + 10 万次迭代（PBKDF 风格），不引入额外加密依赖；
//! - 会话存在内存里，进程重启即失效（重新登录即可），不落盘避免额外泄露面；
//! - 仍然保留旧的 `auth_token`（Authorization 头）作为**机器调用**通道，
//!   这样订阅助手/脚本/Webhook 不受影响。

use std::sync::LazyLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use dashmap::DashMap;
use rand::seq::IndexedRandom;
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement, Value};
use sha2::{Digest, Sha256};

/// 会话有效期：7 天
const SESSION_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
/// 密码哈希迭代次数
const HASH_ROUNDS: u32 = 100_000;
/// 会话 Cookie 名
pub const SESSION_COOKIE: &str = "bs_session";

const SALT_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const TOKEN_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

#[derive(Clone)]
pub struct Session {
    pub username: String,
    expires_at: SystemTime,
}

/// 进程内会话表（token -> 会话）
static SESSIONS: LazyLock<DashMap<String, Session>> = LazyLock::new(DashMap::new);

#[derive(Clone)]
pub struct UserRecord {
    pub username: String,
    pub password_hash: String,
    pub salt: String,
}

/// 密码哈希：SHA-256 迭代 + 盐，输出十六进制
fn hash_password(password: &str, salt: &str) -> String {
    let mut digest = Sha256::digest(format!("{salt}:{password}").as_bytes()).to_vec();
    for _ in 0..HASH_ROUNDS {
        let mut hasher = Sha256::new();
        hasher.update(&digest);
        hasher.update(salt.as_bytes());
        hasher.update(password.as_bytes());
        digest = hasher.finalize().to_vec();
    }
    hex::encode(digest)
}

fn random_string(len: usize, chars: &[u8]) -> String {
    let mut rng = rand::rng();
    (0..len)
        .map(|_| *chars.choose(&mut rng).expect("choose byte failed") as char)
        .collect()
}

pub fn generate_session_token() -> String {
    random_string(48, TOKEN_CHARS)
}

pub fn generate_salt() -> String {
    random_string(32, SALT_CHARS)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 是否已经创建过账号
pub async fn is_initialized(conn: &DatabaseConnection) -> Result<bool> {
    let stmt = Statement::from_string(
        conn.get_database_backend(),
        "SELECT COUNT(*) AS cnt FROM auth_user".to_owned(),
    );
    let row = conn
        .query_one(stmt)
        .await
        .context("查询 auth_user 失败")?;
    let Some(row) = row else { return Ok(false) };
    let cnt: i64 = row.try_get("", "cnt").unwrap_or(0);
    Ok(cnt > 0)
}

pub async fn get_user(conn: &DatabaseConnection, username: &str) -> Result<Option<UserRecord>> {
    let stmt = Statement::from_sql_and_values(
        conn.get_database_backend(),
        "SELECT username, password_hash, salt FROM auth_user WHERE username = ?",
        [Value::from(username)],
    );
    let row = conn.query_one(stmt).await.context("查询用户失败")?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some(UserRecord {
        username: row.try_get("", "username").unwrap_or_default(),
        password_hash: row.try_get("", "password_hash").unwrap_or_default(),
        salt: row.try_get("", "salt").unwrap_or_default(),
    }))
}

/// 创建账号；已存在则报错
pub async fn create_user(conn: &DatabaseConnection, username: &str, password: &str) -> Result<()> {
    validate_credentials(username, password)?;
    if is_initialized(conn).await? {
        bail!("账号已创建，请直接登录");
    }
    let salt = generate_salt();
    let password_hash = hash_password(password, &salt);
    let stmt = Statement::from_sql_and_values(
        conn.get_database_backend(),
        "INSERT INTO auth_user (username, password_hash, salt, created_at) VALUES (?, ?, ?, ?)",
        [
            Value::from(username),
            Value::from(password_hash),
            Value::from(salt),
            Value::from(now_secs().to_string()),
        ],
    );
    conn.execute(stmt).await.context("创建账号失败")?;
    Ok(())
}

/// 校验账号密码
pub async fn verify_user(conn: &DatabaseConnection, username: &str, password: &str) -> Result<bool> {
    let Some(user) = get_user(conn, username).await? else {
        return Ok(false);
    };
    Ok(hash_password(password, &user.salt) == user.password_hash)
}

pub fn validate_credentials(username: &str, password: &str) -> Result<()> {
    let username = username.trim();
    if username.chars().count() < 3 {
        bail!("用户名至少 3 个字符");
    }
    if username.chars().count() > 32 {
        bail!("用户名最多 32 个字符");
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        bail!("用户名只能包含字母、数字、下划线、短横线或点");
    }
    if password.chars().count() < 8 {
        bail!("密码至少 8 个字符");
    }
    Ok(())
}

/// 新建会话，返回 token 与有效期秒数
pub fn create_session(username: &str) -> (String, u64) {
    let token = generate_session_token();
    SESSIONS.insert(
        token.clone(),
        Session {
            username: username.to_owned(),
            expires_at: SystemTime::now() + SESSION_TTL,
        },
    );
    (token, SESSION_TTL.as_secs())
}

pub fn drop_session(token: &str) {
    SESSIONS.remove(token);
}

/// 校验会话 token，顺带清理过期项
pub fn check_session(token: &str) -> Option<Session> {
    let now = SystemTime::now();
    if let Some(entry) = SESSIONS.get(token) {
        if entry.expires_at > now {
            return Some(entry.clone());
        }
    }
    SESSIONS.remove(token);
    None
}

/// 从 Cookie 头里取出会话 token（不依赖额外框架）
pub fn session_token_from_headers(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| {
            raw.split(';')
                .filter_map(|kv| kv.split_once('='))
                .find(|(k, _)| k.trim() == SESSION_COOKIE)
                .map(|(_, v)| v.trim().to_owned())
        })
}

/// 当前请求是否已登录（Cookie 会话）
pub fn current_user(headers: &axum::http::HeaderMap) -> Option<Session> {
    session_token_from_headers(headers).and_then(|t| check_session(&t))
}

pub fn set_cookie_header(token: &str, max_age: u64) -> String {
    format!("{SESSION_COOKIE}={token}; Path=/; Max-Age={max_age}; HttpOnly; SameSite=Lax")
}

pub fn clear_cookie_header() -> String {
    format!("{SESSION_COOKIE}=; Path=/; Max-Age=0; HttpOnly; SameSite=Lax")
}

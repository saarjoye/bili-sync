# 二开说明（Fork Notes）

本仓库是 [amtoaer/bili-sync](https://github.com/amtoaer/bili-sync) 的**个人二开分支**，
用于「B站课程/合集订阅 → 下载 → 上传 115 → CloudMediaSync 生成 strm → Emby 播放」这条链路。

- 上游：`https://github.com/amtoaer/bili-sync`（MIT）
- 当前基线：`c900777`（2026-09-10，`feat: 支持 SMTP 邮件通知 (#747)`）
- 同步上游：`git fetch upstream main && git merge upstream/main`
- 本分支**不含**上游未合并的私有改动；上游 `License` 与作者信息保持原样

> 与上游的差异都会记在提交信息和本文件里，方便以后 rebase / 合并上游。

---

## 一、为什么要二开

上游能力已覆盖大部分需求（订阅合集/系列/UP主投稿、定时追更、NFO + 封面 + 弹幕 + UP主头像），
实测确认可直接用的部分见第三节。需要二开补的是三件事：

| 需求 | 上游现状 | 本分支计划 |
|---|---|---|
| 批量 BV 号清单下载 | `video_sources` 只有 favorites / collections / submissions 三种维度，没有「贴一堆 BV 号」入口（上游 issue #755） | 新增「BV 清单」视频源类型（DB 表 + 扫描分支 + API） |
| 课程合集在 Emby 里连播（一季 N 集） | 合集里的单 P 视频按「电影级」落盘（每集一个目录 + `movie.nfo`），Emby 里是 N 个独立条目 | 可选「课程→剧集」输出：`<剧集名>/Season N/<剧集名> - SxxEExx - 标题.mp4` + `tvshow.nfo` |
| 下载完成即刻通知下游 | 内置 Webhook 通知器（`notifier`） | 无需改代码，直接用 `notifiers` 打 MoviePilot 插件 `/hook` |

> 课程→剧集这件事目前由配套的 MoviePilot 插件 `DirUpload115` 在**上传侧**完成（见第四节），
> 这样不必改 bili-sync 的落盘逻辑，也就不会破坏它的数据库记录。

## 二、二开约定

1. PR 向上游提交时基于上游 `develop`（上游要求）；
2. 新增功能默认关闭，不改变上游默认行为；
3. 涉及 DB 结构必须走 `bili_sync_migration` 新迁移，不改历史迁移；
4. 改动集中在 `crates/bili_sync/src/`，前端 `web/` 同步补 UI（可选）。

## 三、已实测的上游事实（2026-11-10，v2.11.1）

这些是本地实测/读源码确认的，二开时不用再踩一遍：

- **必须登录**：`Config::check()` 要求 `sessdata / bili_jct / buvid3 / dedeuserid / ac_time_value` 五项全非空；
  扫码登录接口 `POST /api/login/qrcode/generate` + `GET /api/login/qrcode/poll` 可用。
- **API 鉴权**：请求头 `Authorization: <auth_token>`（裸 token，**不加 Bearer**）。
- **订阅默认不启用**：新建订阅 `enabled=false`，必须 `PUT /api/video-sources/{type}/{id}` 打开，否则扫描报「没有可用的视频源」。
- **追更是水位增量**：`workflow.rs` 用 `latest_row_at` 水位，每轮只取 `release_datetime > 水位` 的新视频；
  因此「订了 20 集、第 21 集后发」会自动继续拉，不会断更；反过来**补传的旧视频（发布时间早于水位）不会拉**，需手动全量同步。
- **合集/系列类型**：`CollectionType::Season = 2`（合集）、`Series = 1`（系列/列表）；
  新增订阅 `POST /api/video-sources/collections`，body `{sid, mid, collection_type, path}`。
- **产物结构**（单页视频）：`<path>/<video_name>/BVxxxx.mp4 + .nfo(movie) + -poster.jpg + -fanart.jpg + .zh-CN.default.ass`；
  多页视频才有 `tvshow.nfo + Season 1/S01E01.*`。
- **命名模板变量**：`video_name` 可用 `bvid / title / upper_name / upper_mid / pubtime / fav_time`，
  `page_name` 额外有 `ptitle / pid`；支持 `{{ truncate title 30 }}`。**没有「合集名」变量**。
- **落盘路径不可事后改名**：bili-sync 按路径记录文件，改本地目录名会被当成文件丢失并重新下载。
- **依赖 ffmpeg**：官方镜像自带；二进制裸跑需 PATH 里能访问 `ffmpeg`。

## 四、与 MoviePilot 的衔接

配套插件：`saarjoye/MoviePilot-Plugins` → `plugins.v3/dirupload115`（插件 id `DirUpload115`，显示名「目录监控上传115」）。

职责切分：

```
bili-sync（本仓库）            MoviePilot 插件 DirUpload115
├─ 订阅合集/系列 + 定时增量      ├─ 监控下载目录，目录稳定判定
├─ 下载 mp4 + NFO + 封面 + 弹幕   ├─ 上传前重命名（115 文件名 → CMS strm 名）
└─ 落盘 <path>/<video_name>/    ├─ 只上传视频到 115（复用 u115 存储插件，秒传）
                                ├─ NFO/封面/弹幕复制到本地 strm 目录（不上 115）
                                ├─ 课程→剧集：SxxExx 命名 + tvshow.nfo + 剧集海报
                                └─ 等 strm 就绪 / Emby 可查 → 清理本地（保留/回收/删除）
```

建议配置：

```toml
# bili-sync 侧
collection_default_path = "合集/{{name}}"

# MoviePilot 侧（插件设置）
# 监控目录     /downloads/bili/合集        ← 只填根目录，插件自动识别「系列容器 / 单集」
# 115 目标根   /B站
# 课程→剧集模式 开
# 删本地判定   strm（strm 已生成即认为链路通）
```

即时触发（可选）：bili-sync 的 `notifiers` 配 webhook 打插件 `/hook`（`auth: apikey`，用 MoviePilot 的 `API_TOKEN`）：

```json
{"notifiers":[{"type":"webhook",
  "url":"http://moviepilot:3000/api/v1/plugin/DirUpload115/hook",
  "headers":{"X-API-KEY":"<MoviePilot API_TOKEN>"},
  "template":"{\"event\":\"bili-sync-done\"}"}]}
```

## 五、配套工具

`tools/subscribe_helper.py`：列出某个 UP 的全部合集（B站 `seasons_series_list` 已对无指纹请求返回 -400，
所以走「翻投稿收集 season_id → 逐个查合集信息」两步），勾选后批量写入 bili-sync 订阅。

```bash
python tools/subscribe_helper.py list  --mid 3546913471203629
python tools/subscribe_helper.py add   --mid 3546913471203629 --select 1,4,5 --path "合集/{{name}}"
```

容器里跑时把配置文件目录指过去：

```bash
BILI_SYNC_CONFIG_DIR=/path/to/config/bili-sync python tools/subscribe_helper.py list --mid xxx
python tools/subscribe_helper.py add --mid xxx --select 1 --bs-url http://bili-sync:12345 --bs-token <auth_token>
```

## 六、已知限制

- 「系列（Series）」的自动发现不可用（B站 `series/series` 接口同样 -400），只能手工给 `sid` + `collection_type: 1`；
- 课程合集在上游是「一集一个电影级目录」，要剧集化必须靠上游改造或本文第四节的插件方案；
- 上游 `main` 落后 `develop`，二开若想用新特性请以 `develop` 为基线。

## 七、v2.12.0：账号密码登录（本次二开）

**动机**：上游 WebUI 只有单一 `auth_token`（16 位随机串，含 `!@#$%^&*` 等字符），
没有账号密码、没有 TLS、默认监听 `0.0.0.0:12345`；token 还会被 shell 误解析（`$xxx` 被展开）。

**改动**：

| 文件 | 内容 |
|---|---|
| `crates/bili_sync_migration/src/m20261110_000001_add_auth_user.rs` | 新增 `auth_user` 表（id / username / password_hash / salt / created_at） |
| `crates/bili_sync/src/auth.rs` | 密码哈希（SHA-256 + 32 位随机盐 + 100000 次迭代）、内存会话（DashMap，7 天 TTL）、Cookie 解析与 Set-Cookie 构造 |
| `crates/bili_sync/src/api/routes/auth/mod.rs` | `GET /api/auth/state`、`POST /api/auth/setup`、`POST /api/auth/login`、`POST /api/auth/logout` |
| `crates/bili_sync/src/api/routes/mod.rs` | 中间件改为「Cookie 会话优先，其次兼容旧 auth_token / WebSocket base64 token」；`/api/auth/*` 匿名可访问 |
| `crates/bili_sync/src/task/http_server.rs` | 新增 `GET /login`（内嵌单页，含版本号）；未登录访问首页跳转登录页；静态资源放行 |
| `crates/bili_sync/login.html` | 登录 / 首次创建账号的单页（深浅色自适应、显示版本号） |
| `Cargo.toml`、`crates/bili_sync/Cargo.toml`、`web/package.json` | 版本 2.12.0；新增 `sha2` 依赖 |

**接口行为**：

```http
GET  /api/auth/state     → {initialized, logged_in, username, version, auth_mode}
POST /api/auth/setup     {username, password}   # 仅在未初始化时可用，成功后自动登录
POST /api/auth/login     {username, password}   # 成功返回 Set-Cookie: bs_session=...; HttpOnly; Path=/; Max-Age=604800
POST /api/auth/logout                           # 清会话与 Cookie
```

**兼容性**：`Authorization: <auth_token>`、WebSocket 的 `Sec-WebSocket-Protocol: base64(token)` 两条老通道保留，
`tools/subscribe_helper.py`、MoviePilot webhook 等机器调用不受影响。

**安全边界（仍未做的）**：

- 程序本身不做 TLS，公网暴露仍需前置反向代理（Caddy/nginx）加 HTTPS；
- 会话存内存，进程重启需重新登录；
- 单用户模型（一个管理员账号），没有多用户/权限体系；
- 建议部署时把端口只绑内网或配合 WireGuard/Tailscale。

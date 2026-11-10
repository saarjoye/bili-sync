#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
bili-sync 订阅助手：列出某个 UP 的全部「合集 / 系列」，勾选后批量加入 bili-sync 订阅。

为什么需要它：bili-sync 只有「加入单个合集」的 API（POST /api/video-sources/collections，
参数 {sid, mid, collection_type}），没有「列出某 UP 的所有合集让我挑」的入口。
本脚本用 bili-sync 里已保存的登录凭据（%APPDATA%/bili-sync/data.sqlite）做 wbi 签名，
调用 B站 space 接口拿到合集/系列清单，再调 bili-sync API 批量订阅。

用法：
  python tools/subscribe_helper.py list  --mid 3546913471203629
  python tools/subscribe_helper.py add   --mid 3546913471203629 --select 1,3,5 --path "/合集/{{name}}"
  python tools/subscribe_helper.py add   --mid 3546913471203629 --all
  可选：--bs-url http://127.0.0.1:12345  --bs-token <auth_token>（默认从数据库读）
"""
import argparse
import hashlib
import json
import os
import re
import sqlite3
import sys
import time
import urllib.parse
import urllib.request

MIXIN_TAB = [46, 47, 18, 2, 53, 8, 23, 32, 15, 50, 10, 31, 58, 3, 45, 35, 27, 43, 5, 49,
             33, 9, 42, 19, 29, 28, 14, 39, 12, 38, 41, 13, 37, 48, 7, 16, 24, 55, 40,
             61, 26, 17, 0, 1, 60, 51, 30, 4, 22, 25, 54, 21, 56, 59, 6, 63, 57, 62, 11,
             36, 20, 34, 44, 52]
UA = ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 "
      "(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
TYPE_NAME = {1: "系列", 2: "合集"}


def bili_get(url: str, cookie: str = "", params: dict = None):
    if params:
        url = url + ("&" if "?" in url else "?") + urllib.parse.urlencode(params)
    req = urllib.request.Request(url, headers={
        "User-Agent": UA,
        "Referer": "https://space.bilibili.com/",
        "Cookie": cookie,
    })
    with urllib.request.urlopen(req, timeout=25) as resp:
        return json.loads(resp.read().decode("utf-8"))


def mixin_key(img: str, sub: str) -> str:
    orig = img + sub
    return "".join(orig[i] for i in MIXIN_TAB)[:32]


def wbi_sign(params: dict, mkey: str) -> dict:
    params = dict(params)
    params["wts"] = int(time.time())
    items = sorted((k, re.sub(r"[!'()*]", "", str(v))) for k, v in params.items())
    query = urllib.parse.urlencode(items, quote_via=urllib.parse.quote)
    params["w_rid"] = hashlib.md5((query + mkey).encode()).hexdigest()
    return params


def _config_dir() -> str:
    """定位 bili-sync 配置目录：优先环境变量 BILI_SYNC_CONFIG_DIR（容器里常用 /app/.config/bili-sync）"""
    env = os.environ.get("BILI_SYNC_CONFIG_DIR")
    if env:
        return env
    if os.name == "nt":
        return os.path.join(os.environ.get("APPDATA", ""), "bili-sync")
    return os.path.expanduser("~/.config/bili-sync")


def load_bs_db():
    """读 bili-sync 的配置文件：返回 (auth_token, credential)"""
    db = os.path.join(_config_dir(), "data.sqlite")
    if not os.path.exists(db):
        sys.exit(f"找不到 bili-sync 数据库：{db}")
    con = sqlite3.connect(db)
    row = con.execute("select data from config limit 1").fetchone()
    con.close()
    if not row:
        sys.exit("bili-sync 配置表为空，先在 WebUI 里登录一次")
    cfg = json.loads(row[0])
    return cfg.get("auth_token", ""), cfg.get("credential") or {}


def cookie_from(cred: dict) -> str:
    return "; ".join(f"{k}={v}" for k, v in (
        ("SESSDATA", cred.get("sessdata")),
        ("bili_jct", cred.get("bili_jct")),
        ("buvid3", cred.get("buvid3")),
        ("DedeUserID", cred.get("dedeuserid")),
    ) if v)


def _wbi_keys(cookie: str):
    nav = bili_get("https://api.bilibili.com/x/web-interface/nav", cookie)
    img = nav["data"]["wbi_img"]["img_url"].rsplit("/", 1)[-1].split(".")[0]
    sub = nav["data"]["wbi_img"]["sub_url"].rsplit("/", 1)[-1].split(".")[0]
    if not nav["data"].get("isLogin"):
        sys.exit("bili-sync 里的凭据已失效（isLogin=false），先去 WebUI 重新扫码登录")
    return mixin_key(img, sub)


def list_sources(mid: int, cookie: str, max_pages: int = 6):
    """
    发现该 UP 的合集(season)。

    注意：B站的 `seasons_series_list`（一次性列出全部合集/系列）现在对无浏览器
    指纹的请求返回 -400，所以改成两步走：
      1) /x/space/wbi/arc/search 翻 UP 的投稿，收集每条的 season_id（0 表示不属于合集）
      2) /x/polymer/web-space/seasons_archives_list 用 season_id 取合集名/集数
    局限：只能发现「投稿里出现过」的合集；系列(type=1) 无法这样发现，需要手工给 sid。
    """
    mkey = _wbi_keys(cookie)
    season_ids = []
    for pn in range(1, max_pages + 1):
        params = wbi_sign(
            {"mid": mid, "pn": pn, "ps": 30, "order": "pubdate", "platform": "web",
             "web_location": "1550101"},
            mkey,
        )
        data = bili_get("https://api.bilibili.com/x/space/wbi/arc/search", cookie, params)
        if data.get("code") != 0:
            sys.exit(f"arc/search 失败：code={data.get('code')} msg={data.get('message')}")
        vlist = ((data.get("data") or {}).get("list") or {}).get("vlist") or []
        if not vlist:
            break
        for v in vlist:
            sid = v.get("season_id")
            if sid and sid not in season_ids:
                season_ids.append(sid)
        if not data.get("data", {}).get("page", {}).get("count"):
            break

    out = []
    for sid in season_ids:
        params = wbi_sign(
            {"mid": mid, "season_id": sid, "sort_reverse": "false", "page_num": 1,
             "page_size": 5, "web_location": "333.999"},
            mkey,
        )
        data = bili_get(
            "https://api.bilibili.com/x/polymer/web-space/seasons_archives_list", cookie, params
        )
        if data.get("code") != 0:
            continue
        meta = (data.get("data") or {}).get("meta") or {}
        out.append(
            {
                "type": 2,
                "sid": sid,
                "name": meta.get("name"),
                "count": meta.get("total"),
                "cover": meta.get("cover"),
            }
        )
    return out


def bs_api(base: str, token: str, method: str, path: str, payload=None):
    data = json.dumps(payload, ensure_ascii=False).encode("utf-8") if payload is not None else None
    req = urllib.request.Request(base + path, data=data, method=method,
                                 headers={"Authorization": token, "Content-Type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        return {"http_error": e.code, "body": e.read().decode("utf-8", "replace")}


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("action", choices=["list", "add"])
    ap.add_argument("--mid", type=int, required=True, help="UP 主 UID")
    ap.add_argument("--select", help="要订阅的序号，逗号分隔，如 1,3,5")
    ap.add_argument("--all", action="store_true", help="订阅全部")
    ap.add_argument("--path", default="合集/{{name}}", help="落盘路径模板")
    ap.add_argument("--bs-url", default="http://127.0.0.1:12345")
    ap.add_argument("--bs-token", default="")
    args = ap.parse_args()

    token, cred = load_bs_db()
    token = args.bs_token or token
    cookie = cookie_from(cred)
    if not cookie:
        sys.exit("bili-sync 里没有可用凭据，请先在 WebUI 完成扫码登录")

    items = list_sources(args.mid, cookie)
    if not items:
        print("该 UP 没有公开的合集/系列")
        return
    print(f"UP {args.mid} 共有 {len(items)} 个合集/系列：\n")
    print(f"{'序号':<4}{'类型':<6}{'sid':<14}{'集数':<6}名称")
    for i, it in enumerate(items, 1):
        print(f"{i:<4}{TYPE_NAME[it['type']]:<6}{str(it['sid']):<14}{str(it['count']):<6}{it['name']}")

    if args.action == "list":
        return

    if args.all:
        picked = items
    elif args.select:
        indexes = [int(x) for x in args.select.split(",") if x.strip()]
        picked = [items[i - 1] for i in indexes if 1 <= i <= len(items)]
    else:
        sys.exit("请用 --select 1,3 或 --all 指定要订阅的项")

    print()
    for it in picked:
        payload = {
            "sid": int(it["sid"]),
            "mid": args.mid,
            "collection_type": it["type"],
            "path": args.path,
        }
        r = bs_api(args.bs_url, token, "POST", "/api/video-sources/collections", payload)
        ok = r.get("status_code") == 200
        print(f"{'✅' if ok else '❌'} {TYPE_NAME[it['type']]}「{it['name']}」 sid={it['sid']} -> {json.dumps(r, ensure_ascii=False)[:160]}")

    print("\n提示：bili-sync 新订阅默认 enabled=false，需要到 WebUI 里勾选启用，"
          "或在 /api/video-sources/{type}/{id} 上用 PUT 打开。")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
# ============================================================
# ⚠️ 已停用（用户改向 2026-10-27：不做真 3D/GLB，改 3D 国漫 CG
#    风格立绘主展示）——保留备查，勿再使用。
# ============================================================
"""gen_3d_seed3d.py — 火山方舟 doubao-seed3d 图生 3D 批量生成（真机执行）

⚠️ 沙盒无 GPU/无 key，本脚本在真机跑。
前提：已开通 doubao-seed3d-2-0-260328，环境变量 ARK_API_KEY。

用法：
  export ARK_API_KEY="xxx"
  python3 scripts/gen_3d_seed3d.py                  # 先跑活泼（默认登场形象验收）
  python3 scripts/gen_3d_seed3d.py --persona all    # 批量 5 形象
  python3 scripts/gen_3d_seed3d.py --persona huopo --dry-run   # 只打印请求不调 API

产物：src/assets/3d/{gudian,dushi,wenrou,huopo,reqing}.glb（A 组渲染层按此路径加载）
官方文档：https://docs.volcengine.com/docs/ark/create-3d-generation-task-api?lang=zh
"""
import argparse
import base64
import json
import os
import sys
import time
from pathlib import Path

import urllib.request
import urllib.error

API_BASE = "https://ark.cn-beijing.volces.com/api/v3"
MODEL = "doubao-seed3d-2-0-260328"
ALL_PERSONAS = ["gudian", "dushi", "wenrou", "huopo", "reqing"]

# 面数：low=100k / medium=500k / high=1M。桌宠实时渲染用 low 起步，风格验收可 medium。
SUBDIVISION = os.getenv("SEED3D_SUBDIVISION", "low")
FILE_FORMAT = "glb"


def log(msg: str) -> None:
    print(f"[seed3d] {msg}", flush=True)


def api(method: str, path: str, key: str, body: dict | None = None) -> dict:
    url = f"{API_BASE}{path}"
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method)
    req.add_header("Authorization", f"Bearer {key}")
    req.add_header("Content-Type", "application/json")
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.loads(r.read().decode())
    except urllib.error.HTTPError as e:
        err = e.read().decode()
        raise SystemExit(f"API {method} {path} -> HTTP {e.code}: {err}")


def img_to_data_uri(path: Path) -> str:
    b64 = base64.b64encode(path.read_bytes()).decode()
    return f"data:image/png;base64,{b64}"


def gen_one(persona: str, root: Path, key: str, dry_run: bool,
            subdivision: str, poll_timeout: int = 600) -> Path | None:
    img = root / "src/assets/ai_char" / f"{persona}_raw.png"
    out = root / "src/assets/3d" / f"{persona}.glb"
    if out.exists() and out.stat().st_size > 0:
        log(f"跳过 {persona}（已存在 {out}）")
        return out
    if not img.exists():
        log(f"跳过 {persona}（缺输入图 {img}）")
        return None

    body = {
        "model": MODEL,
        "content": [
            {"type": "text",
             "text": f"--subdivisionlevel {subdivision} --fileformat {FILE_FORMAT}"},
            {"type": "image_url", "image_url": {"url": img_to_data_uri(img)}},
        ],
    }

    if dry_run:
        log(f"[dry-run] {persona}: 将 POST 创建任务，image={img.name} "
            f"({img.stat().st_size} bytes) -> subdivision={subdivision}, format={FILE_FORMAT}")
        log(f"[dry-run] 请求体预览: {json.dumps({k: (v if k != 'content' else '[omitted]') for k,v in body.items()}, ensure_ascii=False)}")
        return None

    # 1) 创建任务
    log(f"{persona}: 提交图生3D任务（{img.name}）...")
    resp = api("POST", "/contents/generations/tasks", key, body)
    task_id = resp["id"]
    log(f"{persona}: task_id={task_id}")

    # 2) 轮询
    deadline = time.time() + poll_timeout
    file_url = None
    while time.time() < deadline:
        time.sleep(5)
        st = api("GET", f"/contents/generations/tasks/{task_id}", key)
        status = st.get("status")
        log(f"{persona}: 轮询 status={status}")
        if status == "succeeded":
            file_url = st.get("content", {}).get("video_url") or st.get("content", {}).get("file_url") \
                or st.get("output", {}).get("file_url")
            if not file_url:
                # 兜底：在整个响应里找 http 开头的 glb/obj 链接
                file_url = _find_glb_url(st)
            break
        if status in ("failed", "cancelled"):
            raise SystemExit(f"{persona}: 任务失败: {json.dumps(st, ensure_ascii=False)}")
    if not file_url:
        raise SystemExit(f"{persona}: 轮询超时（{poll_timeout}s）")

    # 3) 下载 GLB
    log(f"{persona}: 下载 {file_url}")
    out.parent.mkdir(parents=True, exist_ok=True)
    urllib.request.urlretrieve(file_url, out)
    log(f"{persona}: 完成 -> {out} ({out.stat().st_size//1024} KB)")
    return out


def _find_glb_url(obj):
    if isinstance(obj, dict):
        for v in obj.values():
            r = _find_glb_url(v)
            if r:
                return r
    elif isinstance(obj, list):
        for v in obj:
            r = _find_glb_url(v)
            if r:
                return r
    elif isinstance(obj, str) and obj.startswith("http") and (".glb" in obj or "file" in obj):
        return obj
    return None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--persona", default="huopo",
                    help="gudian/dushi/wenrou/huopo/reqing/all（默认 huopo 先验收）")
    ap.add_argument("--subdivision", default=SUBDIVISION,
                    choices=["low", "medium", "high"], help="面数等级，默认 low（桌宠实时）")
    ap.add_argument("--dry-run", action="store_true", help="只打印请求不调 API")
    args = ap.parse_args()

    root = Path(__file__).resolve().parent.parent
    key = os.getenv("ARK_API_KEY")
    if not key and not args.dry_run:
        raise SystemExit("需要环境变量 ARK_API_KEY")

    personas = ALL_PERSONAS if args.persona == "all" else [args.persona]
    for p in personas:
        gen_one(p, root, key or "dry", args.dry_run, args.subdivision)


if __name__ == "__main__":
    main()

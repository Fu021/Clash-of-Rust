#!/usr/bin/env python3
"""Render the standalone Slint UI with an existing viewer; never invokes Cargo."""
from __future__ import annotations

import argparse
from html import escape
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
ENTRY = ROOT / "crates/client-ui/ui/preview.slint"
PAGES = {
    "home": "首页",
    "proxies": "代理",
    "subscriptions": "订阅",
    "connections": "连接",
    "rules": "规则",
    "logs": "日志",
    "network": "网络诊断",
    "ip-check": "IP 检测",
    "ai-check": "A÷检测",
    "settings": "设置",
}
DIALOGS = {
    "delete": ("删除订阅确认", "subscriptions", 1),
    "draft": ("放弃草稿确认", "rules", 2),
    "protection": ("Claude 保护确认", "ai-check", 3),
    "rule-editor": ("新建规则", "rules", 4),
    "subscription-editor": ("编辑订阅", "subscriptions", 5),
    "ip-details": ("分类检测详情", "ip-check", 6),
    "ai-risks": ("风险项目详情", "ai-check", 7),
    "ai-normal": ("正常项目详情", "ai-check", 8),
    "ai-clues": ("其他线索详情", "ai-check", 9),
    "accent-picker": ("强调色选择", "settings", 10),
    "connection-details": ("连接详情", "connections", 11),
}


def run_viewer(viewer: str, arguments: list[str], env: dict[str, str]) -> None:
    result = subprocess.run([viewer, *arguments], cwd=ROOT, env=env, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=90)
    if result.stdout:
        print(result.stdout, end="")
    if result.stderr:
        print(result.stderr, end="", file=sys.stderr)
    if result.returncode:
        raise RuntimeError(f"Slint Viewer exited with {result.returncode}")


def image_size(path: Path) -> tuple[int, int]:
    with path.open("rb") as image:
        header = image.read(24)
    if header[:8] != b"\x89PNG\r\n\x1a\n" or header[12:16] != b"IHDR":
        raise RuntimeError(f"Viewer did not produce a PNG: {path}")
    return struct.unpack(">II", header[16:24])


def overview(viewer: str, entries: list[dict], theme: str, output: Path,
             env: dict[str, str]) -> str:
    selected = [entry for entry in entries if entry["theme"] == theme and not entry["dialog"]]
    if not selected:
        return ""
    image_width = 556
    image_height = round(image_width * selected[0]["height"] / selected[0]["width"])
    row_height = image_height + 54
    height = 64 + ((len(selected) + 1) // 2) * row_height
    background, ink = ("#0b1220", "#f1f5f9") if theme == "dark" else ("#e8edf4", "#0f172a")
    font = json.dumps(str(ROOT / "resources/fonts/SourceHanSansCN-Regular.otf"))
    source = [
        f"import {font};",
        "export component Overview inherits Window {",
        f'width: 1160px; height: {height}px; background: {background};',
        'default-font-family: "Source Han Sans CN";',
        f'Text {{ x: 16px; y: 14px; height: 32px; font-size: 22px; color: {ink}; '
        f'text: "Clash of Rust · Slint {"深色" if theme == "dark" else "浅色"}页面总览"; }}',
    ]
    for index, entry in enumerate(selected):
        x, y = 16 + (index % 2) * 572, 60 + (index // 2) * row_height
        title, filename = json.dumps(entry["label"], ensure_ascii=False), json.dumps(entry["file"])
        source.append(f'Text {{ x: {x}px; y: {y}px; height: 26px; font-size: 16px; color: {ink}; text: {title}; }}')
        source.append(f'Image {{ x: {x}px; y: {y + 30}px; width: {image_width}px; height: {image_height}px; '
                      f'source: @image-url({filename}); image-fit: contain; }}')
    source.append("}")
    name = f"overview-{theme}"
    source_file = output / f"{name}.slint"
    source_file.write_text("\n".join(source) + "\n", encoding="utf-8")
    run_viewer(viewer, ["--screenshot", str(output / f"{name}.png"), str(source_file)], env)
    return f"{name}.png"


def gallery(entries: list[dict], output: Path) -> None:
    cards = []
    for entry in entries:
        label, file, theme = escape(entry["label"]), escape(entry["file"]), entry["theme"]
        cards.append(
            f'<article data-theme="{theme}"><h2>{label} · {"深色" if theme == "dark" else "浅色"}</h2>'
            f'<a href="{file}" target="_blank"><img src="{file}" width="{entry["width"]}" '
            f'height="{entry["height"]}" loading="lazy" alt="{label}"></a></article>'
        )
    html = """<!doctype html>
<html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>Clash of Rust · Slint 界面预览</title>
<style>
body{margin:0;background:#edf2f7;color:#0f172a;font:15px system-ui,sans-serif}
header{position:sticky;top:0;background:#ffffffed;border-bottom:1px solid #cbd5e1;padding:16px 24px;z-index:1}
h1{font-size:22px;margin:0 0 8px}p{color:#475569;margin:6px 0 12px}
button{background:white;border:1px solid #94a3b8;color:#0f172a;border-radius:6px;padding:8px 16px;cursor:pointer}
button.active{background:#0f766e;color:white;border-color:#0f766e}main{max-width:1160px;margin:auto;padding:24px}
article{margin-bottom:32px}h2{font-size:17px}img{display:block;width:100%;height:auto;border:1px solid #cbd5e1;border-radius:8px}
[hidden]{display:none!important}
</style>
<header><h1>Clash of Rust · Slint 界面预览</h1>
<p>以下均为 .slint 文件实际渲染截图，使用演示数据；点击图片查看原尺寸。</p>
<button class="active" data-filter="all">全部</button> <button data-filter="light">浅色</button>
<button data-filter="dark">深色</button></header><main>"""
    html += "\n".join(cards)
    html += """</main><script>
document.querySelectorAll('button[data-filter]').forEach(button=>button.onclick=()=>{
 document.querySelectorAll('button[data-filter]').forEach(item=>item.classList.toggle('active',item===button));
 document.querySelectorAll('article').forEach(item=>item.hidden=button.dataset.filter!=='all'&&item.dataset.theme!==button.dataset.filter);
});
</script></html>"""
    (output / "index.html").write_text(html, encoding="utf-8")


def pairs(viewer: str, entries: list[dict], output: Path, env: dict[str, str]) -> list[str]:
    """Render native-size light/dark pairs for displaying directly in chat."""
    folder = output / "pairs"
    folder.mkdir(exist_ok=True)
    grouped = {}
    for item in entries:
        grouped.setdefault(item["file"].removesuffix(f'-{item["theme"]}.png'), {})[item["theme"]] = item
    rendered = []
    font = json.dumps(str(ROOT / "resources/fonts/SourceHanSansCN-Regular.otf"))
    for name, themes in grouped.items():
        if "light" not in themes or "dark" not in themes:
            continue
        left, right = themes["light"], themes["dark"]
        width = left["width"] + right["width"] + 48
        height = max(left["height"], right["height"]) + 68
        source = [f'import {font};', 'export component Pair inherits Window {',
                  f'width: {width}px; height: {height}px; background: #e8edf4;',
                  'default-font-family: "Source Han Sans CN";']
        for item, x, theme_name in ((left, 16, "浅色"), (right, left["width"] + 32, "深色")):
            title = json.dumps(f'{item["label"]} · {theme_name}', ensure_ascii=False)
            image = json.dumps("../" + item["file"])
            source.append(f'Text {{ x: {x}px; y: 12px; height: 28px; font-size: 18px; color: #0f172a; text: {title}; }}')
            source.append(f'Image {{ x: {x}px; y: 52px; width: {item["width"]}px; height: {item["height"]}px; source: @image-url({image}); }}')
        source.append("}")
        source_file = folder / f"{name}.slint"
        source_file.write_text("\n".join(source) + "\n", encoding="utf-8")
        run_viewer(viewer, ["--screenshot", str(folder / f"{name}.png"), str(source_file)], env)
        rendered.append(f"pairs/{name}.png")
    return rendered


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--viewer", default=shutil.which("slint-viewer"))
    parser.add_argument("--page", choices=["all", *PAGES], default="all")
    parser.add_argument("--theme", choices=["light", "dark", "both"], default="both")
    parser.add_argument("--dialogs", action="store_true", help="also render confirmation, editing and result dialogs")
    parser.add_argument("--live", action="store_true", help="open the viewer with automatic reload")
    parser.add_argument("--check", action="store_true", help="only check Slint syntax and bindings")
    parser.add_argument("--pairs", action="store_true", help="also render light/dark pairs suitable for inline sharing")
    parser.add_argument("--full-settings", action="store_true", help="show all settings without scrolling in its page screenshot")
    parser.add_argument("--size", default="950x700")
    parser.add_argument("--output", type=Path, default=ROOT / "dist/slint-preview")
    args = parser.parse_args()
    if not args.viewer:
        parser.error("Install the official prebuilt Slint Viewer in a persistent directory, or pass --viewer PATH.")
    try:
        width, height = map(int, args.size.lower().split("x"))
        if width < 800 or height < 450 or width > 4096 or height > 4096:
            raise ValueError
    except ValueError:
        parser.error("--size must be WIDTHxHEIGHT, between 800x450 and 4096x4096")

    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    fixtures = output / "fixtures"
    fixtures.mkdir(exist_ok=True)
    # Keep fontconfig caches writable without requiring installation as root.
    cache = output / "font-cache"
    cache.mkdir(exist_ok=True)
    fontconfig = fixtures / "fonts.conf"
    fontconfig.write_text('<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">'
                          '<fontconfig><include ignore_missing="yes">/etc/fonts/fonts.conf</include>'
                          f'<cachedir>{escape(str(cache))}</cachedir></fontconfig>', encoding="utf-8")
    env = {**os.environ, "FONTCONFIG_FILE": str(fontconfig), "SLINT_SCALE_FACTOR": "1"}
    entry = ENTRY
    run_viewer(args.viewer, ["--check", str(entry)], env)
    if args.check:
        print("Slint UI syntax and bindings: OK")
        return

    pages = list(PAGES) if args.page == "all" else [args.page]
    themes = ["light", "dark"] if args.theme == "both" else [args.theme]
    if args.live:
        fixture = fixtures / "live.json"
        values = {"preview-page": list(PAGES).index(pages[0])}
        fixture.write_text(json.dumps({**values, "preview-dark": themes[0] == "dark"}))
        subprocess.run([args.viewer, "--auto-reload", "--load-data", str(fixture), str(entry)],
                       cwd=ROOT, env=env, check=True)
        return

    scenes = [(name, PAGES[name], name, 0) for name in pages]
    if args.dialogs:
        scenes.extend((name, label, page, kind) for name, (label, page, kind) in DIALOGS.items()
                      if args.page == "all" or page == args.page)
    entries = []
    for theme in themes:
        for name, label, page, dialog in scenes:
            fixture = fixtures / f"{name}-{theme}.json"
            fixture.write_text(json.dumps({"preview-page": list(PAGES).index(page), "preview-dark": theme == "dark", "dialog": dialog}))
            image = output / f"{name}-{theme}.png"
            scene_size = f"{width}x{max(height, 1120)}" if args.full_settings and page == "settings" and not dialog else args.size
            run_viewer(args.viewer, ["--load-data", str(fixture), "--size", scene_size,
                                   "--screenshot", str(image), str(ENTRY)], env)
            actual_width, actual_height = image_size(image)
            entries.append({"page": page, "label": label, "dialog": dialog, "theme": theme,
                            "file": image.name, "width": actual_width, "height": actual_height})
            print(f"Rendered {name}-{theme}: {actual_width}x{actual_height}", flush=True)
    overviews = [overview(args.viewer, entries, theme, output, env) for theme in themes]
    paired = pairs(args.viewer, entries, output, env) if args.pairs else []
    (output / "manifest.json").write_text(json.dumps({"entry": str(ENTRY.relative_to(ROOT)),
                                                     "screenshots": entries, "overviews": overviews, "pairs": paired},
                                                    ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    gallery(entries, output)
    print(f"Gallery: {output / 'index.html'}")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, subprocess.SubprocessError, OSError) as error:
        print(f"Slint preview failed: {error}", file=sys.stderr)
        sys.exit(1)

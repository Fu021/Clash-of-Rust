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


def mobile_preview(args) -> None:
    """Preview the independent Android/iOS entries without building a native host."""
    platforms = ['android', 'ios'] if args.platform == 'both' else [args.platform]
    for platform in platforms:
        entry = ROOT / f'crates/client-ui/ui/mobile/{platform}/preview.slint'
        run_viewer(args.viewer, ['--check', str(entry)], dict(os.environ))
    if args.check:
        print('Android/iOS Slint syntax and bindings: OK')
        return
    try:
        width, height = map(int, (args.size or '393x852').lower().split('x'))
        if not (320 <= width <= 4096 and 320 <= height <= 4096):
            raise ValueError
    except ValueError:
        raise RuntimeError('--size must be WIDTHxHEIGHT, between 320x320 and 4096x4096')
    base = (args.output or ROOT / 'dist/mobile-preview').resolve()
    pages = {**PAGES, 'tools': '工具'}
    names = list(pages) if args.page == 'all' else [args.page]
    themes = [False, True] if args.theme == 'both' else [args.theme == 'dark']
    for platform in platforms:
        entry = ROOT / f'crates/client-ui/ui/mobile/{platform}/preview.slint'
        output = base / platform
        fixtures = output / 'fixtures'
        fixtures.mkdir(parents=True, exist_ok=True)
        env = {**os.environ, 'SLINT_SCALE_FACTOR': '1', 'XDG_CACHE_HOME': str(output / 'cache')}
        entries = []

        def render(name, label, size='393x852', page=0, dark=False, detail=0,
                   scale=1, connected=True, keyboard=0, **extra):
            width, height = map(int, size.split('x'))
            short = height < 480
            values = {'preview-page': page, 'preview-dark': dark, 'preview-detail': detail,
                      'inset-top': 0 if short else 24 if width >= 640 else 54 if platform == 'ios' else 28,
                      'inset-bottom': 21 if short else 30 if platform == 'ios' else 24,
                      'inset-left': 44 if short and platform == 'ios' else 0,
                      'inset-right': 44 if short and platform == 'ios' else 0,
                      'text-scale': scale, 'preview-connected': connected, 'preview-keyboard': keyboard,
                      'preview-notice': args.notice, 'preview-notice-elapsed': args.notice_elapsed,
                      **extra}
            fixture = fixtures / (name + '.json')
            fixture.write_text(json.dumps(values), encoding='utf-8')
            path = output / (name + '.png')
            run_viewer(args.viewer, ['--load-data', str(fixture), '--size', size,
                                    '--screenshot', str(path), str(entry)], env)
            if image_size(path) != (width, height):
                raise RuntimeError(f'{name}: unexpected screenshot dimensions: {image_size(path)}')
            entries.append({'file': path.name, 'label': label, 'width': width, 'height': height,
                            'theme': 'dark' if dark else 'light', 'dialog': detail, 'page': page})
            print(f'Rendered {platform}/{name}', flush=True)

        for name in names:
            for dark in themes:
                render(f'{name}-{"dark" if dark else "light"}', pages[name],
                       args.size or '393x852', list(pages).index(name), dark, scale=args.text_scale)
        if args.page == 'all':
            for name, label, size, page, dark, detail, scale in [
                ('phone-landscape', '手机横屏 · 首页', '852x393', 0, False, 0, 1),
                ('landscape-rules', '手机横屏 · 规则', '852x393', 4, False, 0, 1),
                ('landscape-connections', '手机横屏 · 连接', '852x393', 3, False, 0, 1),
                ('pad-portrait', 'Pad 竖屏 · 代理', '834x1194', 1, False, 0, 1),
                ('pad-landscape', 'Pad 横屏 · 代理', '1194x834', 1, True, 0, 1),
                ('pad-connections', 'Pad 横屏 · 连接详情', '1194x834', 3, False, 5, 1),
                ('pad-risk', 'Pad 横屏 · 风险详情', '1194x834', 8, True, 1, 1),
                ('pad-settings', 'Pad 横屏 · 设置', '1194x834', 9, False, 0, 1),
                ('settings-full', '设置 · 完整内容', '1194x1200', 9, False, 0, 1),
                ('pad-split', 'Pad 分屏 · 检测报告', '507x834', 8, False, 0, 1),
                ('small-phone', '小屏手机', '360x780', 9, False, 0, 1),
                ('large-text', '大字号 · 125%', '393x852', 8, False, 0, 1.25),
                ('risk-detail', '风险项目详情', '393x852', 8, False, 1, 1),
                ('rule-editor', '规则编辑', '393x852', 4, False, 2, 1),
                ('connection-details', '连接详情', '393x852', 3, False, 5, 1),
                ('log-detail', '日志全文', '393x852', 5, False, 10, 1),
                ('ip-category', '分类检测详情', '393x852', 7, False, 6, 1),
                ('delete-confirm', '删除确认', '393x852', 2, False, 4, 1),
                ('draft-confirm', '草稿保护', '393x852', 4, False, 12, 1),
                ('protection', '保护确认', '393x852', 8, True, 8, 1),
                ('accent-picker', '强调色', '393x852', 9, False, 11, 1),
            ]:
                render(name, label, size, page, dark, detail, scale)
            render('disconnected', '连接前状态', connected=False)
            render('rule-keyboard', '键盘避让', page=4, detail=2, keyboard=300)
            render('rule-selected', '规则操作置顶', page=4, **{'preview-selected-rule': True})
            render('rules-shortcuts', '快捷规则', page=4, **{'preview-shortcuts': True})
            render('protection-pending', '切换期间进度', page=8, **{'preview-protection-pending': True})
            render('notice', '通知与消退进度', **{'preview-notice': '配置已更新，当前连接保持运行。', 'preview-notice-elapsed': 3})
            render('notice-over-modal', '通知位于弹窗上方', page=9, detail=11,
                   **{'preview-notice': '强调色已保存。', 'preview-notice-elapsed': 3})
            render('sparse-subscriptions', '少量内容置顶', page=2, **{'preview-sparse': True})
        (output / 'manifest.json').write_text(json.dumps(entries, ensure_ascii=False, indent=2), encoding='utf-8')
        gallery(entries, output)
        if args.pairs:
            pairs(args.viewer, entries, output, env)
        for name, selected in [
            ('phone-main', ['home-light', 'proxies-light', 'subscriptions-light', 'connections-light']),
            ('phone-tools', ['rules-light', 'logs-light', 'network-light', 'tools-light']),
            ('phone-reports', ['ip-check-light', 'ai-check-light', 'settings-light', 'risk-detail']),
            ('phone-dark', ['home-dark', 'ai-check-dark', 'settings-dark', 'logs-dark']),
            ('phone-details', ['connection-details', 'log-detail', 'rule-editor', 'ip-category']),
            ('phone-confirmations', ['delete-confirm', 'draft-confirm', 'accent-picker', 'rule-keyboard']),
        ]:
            items = [e for filename in selected for e in entries if e['file'] == filename + '.png']
            if not items:
                continue
            mobile_sheet(args.viewer, output, name, items, env)
        print(f'{platform}: {output / "index.html"}', flush=True)


def mobile_sheet(viewer, output, name, items, env):
    """Compose unmodified screenshots with Slint Image items, at native scale."""
    cell_width = max(item['width'] for item in items)
    cell_height = max(item['height'] for item in items)
    rows = (len(items) + 1) // 2
    sheet_width = 48 + cell_width * 2
    sheet_height = 24 + rows * (cell_height + 58)
    source = ['import ' + json.dumps(str(ROOT / 'resources/fonts/SourceHanSansCN-Regular.otf')) + ';',
              'export component Sheet inherits Window {',
              f'width: {sheet_width}px; height: {sheet_height}px; background: #e8edf4;',
              'default-font-family: "Source Han Sans CN";']
    for i, item in enumerate(items):
        x, y = 16 + i % 2 * (cell_width + 16), 12 + i // 2 * (cell_height + 58)
        source.append(f'Text {{ x: {x}px; y: {y}px; width: {cell_width}px; height: 36px; '
                      f'text: {json.dumps(item["label"], ensure_ascii=False)}; font-size: 18px; color: #141c29; }}')
        source.append(f'Image {{ x: {x}px; y: {y+42}px; width: {item["width"]}px; height: {item["height"]}px; '
                      f'source: @image-url({json.dumps(item["file"])}); }}')
    source.append('}')
    sheet = output / (name + '.slint')
    sheet.write_text('\n'.join(source), encoding='utf-8')
    run_viewer(viewer, ['--screenshot', str(output / (name + '.png')), str(sheet)], env)


def run_viewer(viewer: str, arguments: list[str], env: dict[str, str]) -> None:
    result = subprocess.run([viewer, *arguments], cwd=ROOT, env=env, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=90)
    if result.stdout:
        print(result.stdout, end="")
    if result.stderr:
        print(result.stderr, end="", file=sys.stderr)
    if result.returncode:
        raise RuntimeError(f"Slint Viewer exited with {result.returncode}")
    if "ignoring unknown property:" in result.stderr:
        raise RuntimeError("Preview data references a property missing from the selected Slint entry")


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
    parser.add_argument("--page", choices=["all", *PAGES, "tools"], default="all")
    parser.add_argument("--mobile", action="store_true", help="render the separate adaptive Android/iOS UI study")
    parser.add_argument("--platform", choices=["android", "ios", "both"], default="both", help="mobile platform entry")
    parser.add_argument("--theme", choices=["light", "dark", "both"], default="both")
    parser.add_argument("--dialogs", action="store_true", help="also render confirmation, editing and result dialogs")
    parser.add_argument("--live", action="store_true", help="open the viewer with automatic reload")
    parser.add_argument("--check", action="store_true", help="only check Slint syntax and bindings")
    parser.add_argument("--pairs", action="store_true", help="also render light/dark pairs suitable for inline sharing")
    parser.add_argument("--full-settings", action="store_true", help="show all settings without scrolling in its page screenshot")
    parser.add_argument("--notice", default="", help="show a floating notification using the production component")
    parser.add_argument("--notice-elapsed", type=float, default=3, help="elapsed notification lifetime in seconds")
    parser.add_argument("--size")
    parser.add_argument("--text-scale", type=float, default=1, help="mobile font preview factor (native Android uses system SP conversions)")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if not args.viewer:
        parser.error("Install the official prebuilt Slint Viewer in a persistent directory, or pass --viewer PATH.")
    if args.mobile:
        mobile_preview(args)
        return
    if args.page == 'tools':
        parser.error('--page tools is only available with --mobile')
    args.size = args.size or '950x700'
    args.output = args.output or ROOT / 'dist/slint-preview'
    try:
        width, height = map(int, args.size.lower().split("x"))
        if width < 800 or height < 450 or width > 4096 or height > 4096:
            raise ValueError
    except ValueError:
        parser.error("--size must be WIDTHxHEIGHT, between 800x450 and 4096x4096")

    if args.check:
        run_viewer(args.viewer, ["--check", str(ENTRY)], dict(os.environ))
        print("Slint UI syntax and bindings: OK")
        return

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
    pages = list(PAGES) if args.page == "all" else [args.page]
    themes = ["light", "dark"] if args.theme == "both" else [args.theme]
    if args.live:
        fixture = fixtures / "live.json"
        values = {"preview-page": list(PAGES).index(pages[0]), "preview-motion": True}
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
            fixture.write_text(json.dumps({"preview-page": list(PAGES).index(page), "preview-dark": theme == "dark", "dialog": dialog,
                                           "preview-notice": args.notice, "preview-notice-elapsed": args.notice_elapsed}))
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

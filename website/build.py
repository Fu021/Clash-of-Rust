"""Build the Clash of Rust site into website/dist. Standard library only.

    python website/build.py

Chinese pages land in dist/, English pages in dist/en/. Illustrations and fonts
come ready-made from src/assets; prepare_assets.py regenerates those.
"""
from html import escape
import json
from pathlib import Path
import re
import shutil
from release_content import validate, render as render_releases, SUFFIXES

ROOT = Path(__file__).resolve().parent
SRC = ROOT / "src"
DIST = ROOT / "dist"
REPO_URL = "https://github.com/Fu021/Clash-of-Rust"
DOCS_URL = f"{REPO_URL}/blob/main/docs"
SITE_URL = "https://clash-of-rust.fu021.shop"
PAGES = ("index", "tools", "guide", "download", "releases")
SPACING = ("0.9", "1", "1.1", "1.2", "1.3", "1.4", "1.6")

TEXT = {
    "en": {
        "html_lang": "en", "skip": "Skip to content", "home": "Clash of Rust home",
        "nav_label": "Site", "nav": {"tools": "Network tools", "guide": "Guide", "releases": "Versions", "download": "Download"},
        "other_lang": "中文", "other_label": "切换到中文", "star": "Star",
        "foot_note": "Free software under GPL-3.0. Runs on the mihomo core.",
        "foot_links": {"releases": "All releases", "issues": "Report a problem", "docs": "Developer docs"},
        "labels": {"checks": "What it checks", "when": "Use it when", "note": "Keep in mind", "options": "Options",
                   "none": "No extra options", "symptom": "What you see", "try": "Start with",
                   "flag": "Option", "range": "Range", "default": "Default", "meaning": "Meaning", "used": "Used by"},
    },
    "zh": {
        "html_lang": "zh-CN", "skip": "跳到正文", "home": "Clash of Rust 首页",
        "nav_label": "站内导航", "nav": {"tools": "网络工具", "guide": "使用指南", "releases": "版本记录", "download": "下载"},
        "other_lang": "English", "other_label": "Switch to English", "star": "Star",
        "foot_note": "开源 mihomo 客户端 · GPL-3.0",
        "foot_links": {"releases": "全部版本", "issues": "反馈问题", "docs": "开发文档"},
        "labels": {"checks": "查什么", "when": "什么时候用", "note": "留意", "options": "可调参数",
                   "none": "没有额外参数", "symptom": "现象", "try": "先用",
                   "flag": "参数", "range": "范围", "default": "默认", "meaning": "含义", "used": "适用工具"},
    },
}

WORDMARK = (
    '<svg class="wordmark" viewBox="0 0 116 19" fill="none" aria-hidden="true">'
    '<path stroke="currentColor" stroke-width="1.8" d="M9.6 8.5A5 5 0 1 0 9.6 15.5 M13.4 2V17 M16.8 12a5 5 0 1 0 10 0a5 5 0 1 0 -10 0 '
    'M26.8 7V17 M35.8 8.6C34.8 7.1 29.8 6.7 29.8 9.6C29.8 12.2 36.2 11.5 36.2 14.3C36.2 17.6 30.6 17.5 29.4 15.6 M39.6 2V17 '
    'M39.6 12a4 4.6 0 0 1 8 0V17 M80.8 7V17 M80.8 11.8a4.6 4.6 0 0 1 5 -4.5 M89.2 7V12a4 4.6 0 0 0 8 0 M97.2 7V17 '
    'M106.2 8.6C105.2 7.1 100.2 6.7 100.2 9.6C100.2 12.2 106.6 11.5 106.6 14.3C106.6 17.6 101 17.5 99.8 15.6 '
    'M111 3.4V13.6a3 3 0 0 0 3.9 3 M109 8H114.4"/>'
    '<path class="wordmark-of" stroke-width="1.6" d="M55.2 12a5 5 0 1 0 10 0a5 5 0 1 0 -10 0 M68.8 17V6.2a3.5 3.5 0 0 1 4.4 -3.4 M66.8 8H72"/></svg>'
)
STAR = '<svg class="star" viewBox="0 0 12 12" aria-hidden="true"><path d="M5 0h2v3h2v1h3v2h-1v1h-1v1h1v2h1v2h-2v-1H9v-1H8V9H4v1H3v1H2v1H0v-2h1V8h1V7H1V6H0V4h3V3h2z"/></svg>'

CAPTURES = {
    "home": ("首页：连接、模式、流量和网络信息", "Home: connection, mode, traffic and network details"),
    "nodes": ("代理：策略组、节点与延迟", "Proxies: policy groups, nodes and latency"),
    "subscriptions": ("订阅：添加、导入与切换配置", "Subscriptions: add, import and switch configurations"),
    "rules": ("规则：快捷规则与自定义分流", "Rules: shortcuts and custom routing"),
    "network": ("网络诊断：选择工具、输入目标并查看结果", "Diagnostics: choose a tool, enter a target and read the result"),
    "protection": ("A÷检测：地区线索与 Claude 保护", "A÷ check: region clues and Claude protection"),
    "applications": ("Android 应用分流：筛选并选择进入 VPN 的应用", "Android routing: find and select the apps that enter the VPN"),
}


def render_capture(name, lang, root, captures):
    capture = captures[name]
    label = CAPTURES[name][0 if lang == "zh" else 1]
    phone = capture["platform"] == "android"
    platform = "Android" if phone else ("桌面" if lang == "zh" else "Desktop")
    note = "演示数据" if lang == "zh" else "Demo data"
    return (f'<figure class="product-shot{" phone" if phone else ""}" data-capture-version="{escape(capture["version"])}">'
            f'<div class="frame"><img src="{root}assets/{escape(capture["file"])}" '
            f'width="{capture["width"]}" height="{capture["height"]}" alt="{escape(label)}" loading="lazy"></div>'
            f'<figcaption>{escape(label)} · {platform} · {escape(capture["version"])} · {note}</figcaption></figure>')


def render_terminal(name, lang, frames):
    frame = frames[name]
    if frame["source"] != "actual-cor-pty":
        raise ValueError("Terminal illustrations must use actual PTY frames")
    tones = {"plain", "muted", "accent", "selected", "warning"}
    lines = []
    for line in frame["lines"]:
        if line["tone"] not in tones:
            raise ValueError("Invalid terminal tone")
        lines.append(f'<span class="tone-{line["tone"]}">{escape(line["text"])}</span>')
    label = "实际终端画面 · 固定测试输入" if lang == "zh" else "Actual terminal frame · Fixed test input"
    return (f'<figure class="product-shot terminal-shot" data-capture-version="{escape(frame["version"])}">'
            '<div class="terminal-title" aria-hidden="true">cor ui</div>'
            f'<pre class="terminal-capture" tabindex="0" aria-label="{label}">' + "\n".join(lines) + "</pre>"
            f'<figcaption>{label} · Linux · {escape(frame["version"])}</figcaption></figure>')


def tool_command(tool):
    return f'cor network {tool["id"]} {tool["arg"]}'.rstrip()


def render_downloads(body, release_data):
    stable = release_data["downloads"]["stable"]
    version = stable["tag"].removeprefix("v")
    for system in ("windows", "linux", "macos", "android"):
        rows = []
        for suffix in SUFFIXES:
            if not suffix.startswith(system + "-"):
                continue
            name = f"Clash-of-Rust-{version}-{suffix}"
            url = f"{REPO_URL}/releases/download/{stable['tag']}/{name}"
            label = suffix.removeprefix(system + "-")
            size = stable["sizes"][suffix] / 1048576
            style = "btn" if not rows else "btn ghost"
            rows.append(f'<li><a class="{style}" href="{escape(url, quote=True)}">{escape(label)}'
                        f'<span>{size:.1f} MiB</span></a>'
                        f'<a class="sum" href="{escape(url, quote=True)}.sha256">SHA256</a></li>')
        pattern = (r'(<section class="os" id="' + system + r'" data-os="' + system
                   + r'">[\s\S]*?<ul class="pkgs" data-pkgs>)</ul>')
        body, count = re.subn(pattern, lambda match: match[1] + "".join(rows) + "</ul>", body, count=1)
        if count != 1:
            raise ValueError("Missing download section: " + system)
    return body


def render_tools(kind, lang, data, root):
    labels = TEXT[lang]["labels"]
    tools = {tool["id"]: tool for tool in data["tools"]}
    if kind == "home":
        parts = []
        for group in data["groups"]:
            items = "".join(
                f'<li><a href="{root}tools.html#{tool["id"]}"><b>{escape(tool[lang]["name"])}</b>'
                f'<span>{escape(tool[lang]["short"])}</span></a></li>'
                for tool in data["tools"] if tool["group"] == group["id"])
            parts.append(f'<div class="tool-group"><h3>{escape(group[lang])}</h3><ul class="tool-list">{items}</ul></div>')
        return "".join(parts)
    if kind == "symptoms":
        rows = "".join(
            f'<tr><th scope="row">{escape(row[lang])}</th><td>' + " ".join(
                f'<a class="chip" href="#{tool}">{escape(tools[tool][lang]["name"])}</a>' for tool in row["tools"]) + "</td></tr>"
            for row in data["symptoms"])
        return (f'<table class="plain symptoms"><thead><tr><th scope="col">{labels["symptom"]}</th>'
                f'<th scope="col">{labels["try"]}</th></tr></thead><tbody>{rows}</tbody></table>')
    if kind == "options":
        rows = ""
        for key, option in data["options"].items():
            users = "、" if lang == "zh" else ", "
            used = users.join(tool[lang]["name"] for tool in data["tools"] if key in tool["options"])
            rows += (f'<tr><th scope="row"><code>{option["flag"]}</code></th><td>{escape(option[lang])}</td>'
                     f'<td>{option["range"]}</td><td>{option["default"]}</td><td>{escape(used)}</td></tr>')
        return (f'<div class="table-scroll"><table class="plain"><thead><tr><th scope="col">{labels["flag"]}</th>'
                f'<th scope="col">{labels["meaning"]}</th><th scope="col">{labels["range"]}</th>'
                f'<th scope="col">{labels["default"]}</th><th scope="col">{labels["used"]}</th></tr></thead>'
                f'<tbody>{rows}</tbody></table></div>')
    if kind == "articles":
        parts = []
        for group in data["groups"]:
            parts.append(f'<div class="group-head" id="{group["id"]}"><h2>{escape(group[lang])}</h2>'
                         f'<p>{escape(group["note_" + lang])}</p></div>')
            for tool in (tool for tool in data["tools"] if tool["group"] == group["id"]):
                copy = tool[lang]
                options = "".join(
                    f'<li><code>{data["options"][key]["flag"]}</code> {data["options"][key]["range"]}</li>'
                    for key in tool["options"]) or f'<li class="none">{labels["none"]}</li>'
                parts.append(
                    f'<article class="tool" id="{tool["id"]}"><div class="tool-side"><h3>{escape(copy["name"])}</h3>'
                    f'<code class="command">{escape(tool_command(tool))}</code>'
                    f'<p class="label">{labels["options"]}</p><ul class="options">{options}</ul></div>'
                    f'<dl class="tool-body"><div><dt>{labels["checks"]}</dt><dd>{escape(copy["checks"])}</dd></div>'
                    f'<div><dt>{labels["when"]}</dt><dd>{escape(copy["when"])}</dd></div>'
                    f'<div><dt>{labels["note"]}</dt><dd>{escape(copy["note"])}</dd></div></dl></article>')
        return "".join(parts)
    raise ValueError(f"unknown tools block: {kind}")


def layout(lang, page, title, description, body, release_data):
    text = TEXT[lang]
    root = "" if lang == "zh" else "../"
    other = f"en/{page}.html" if lang == "zh" else f"../{page}.html"
    page_path = "" if page == "index" else page
    canonical = f"{SITE_URL}/{'en/' if lang == 'en' else ''}{page_path}"
    current = ' aria-current="page"'
    nav = "".join(
        f'<a href="{name}.html"{current if name == page else ""}>{label}</a>'
        for name, label in text["nav"].items())
    foot = text["foot_links"]
    release_json = json.dumps(release_data, ensure_ascii=True).replace("<", "\\u003c")
    return f"""<!doctype html>
<html lang="{text['html_lang']}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{escape(title)}</title>
<meta name="description" content="{escape(description)}">
<meta name="theme-color" content="#f8f3e7">
<link rel="canonical" href="{canonical}">
<link rel="alternate" hreflang="en" href="{SITE_URL}/en/{page_path}">
<link rel="alternate" hreflang="zh-CN" href="{SITE_URL}/{page_path}">
<link rel="alternate" hreflang="x-default" href="{SITE_URL}/{page_path}">
<link rel="icon" href="{root}assets/cat.png">
<link rel="preload" href="{root}assets/fonts/national-park.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="{root}site.css">
<script type="application/json" id="release-data">{release_json}</script><script src="{root}site.js" defer></script>
</head>
<body class="page-{page}">
<a class="skip" href="#main">{text['skip']}</a>
<header class="top">
  <div class="wrap">
    <a class="brand" href="index.html" aria-label="{text['home']}"><img src="{root}assets/cat.png" alt="" width="30" height="30">{WORDMARK}</a>
    <nav aria-label="{text['nav_label']}">{nav}</nav>
    <a class="lang" href="{other}" lang="{'zh-CN' if lang == 'en' else 'en'}" aria-label="{text['other_label']}" data-set-lang="{'zh' if lang == 'en' else 'en'}">{text['other_lang']}</a>
    <a class="btn small ghost" href="{REPO_URL}" target="_blank" rel="noopener">{STAR}{text['star']}<span data-stars></span></a>
  </div>
</header>
<main id="main">
{body}
</main>
<footer class="foot">
  <div class="track" aria-hidden="true"></div>
  <div class="wrap">
    <div><a class="brand" href="index.html" aria-label="{text['home']}">{WORDMARK}</a><p>{text['foot_note']}</p></div>
    <nav aria-label="{text['nav_label']}">{nav}</nav>
    <nav><a href="{REPO_URL}" target="_blank" rel="noopener">GitHub</a><a href="{REPO_URL}/releases" target="_blank" rel="noopener">{foot['releases']}</a><a href="{REPO_URL}/issues" target="_blank" rel="noopener">{foot['issues']}</a><a href="{DOCS_URL}/README.md" target="_blank" rel="noopener">{foot['docs']}</a></nav>
  </div>
</footer>
</body>
</html>
"""


def build():
    data = json.loads((SRC / "content" / "tools.json").read_text(encoding="utf-8"))
    release_data = json.loads((SRC / "content" / "releases.json").read_text(encoding="utf-8"))
    captures = json.loads((SRC / "content" / "screenshots.json").read_text(encoding="utf-8"))
    terminal = json.loads((SRC / "content" / "terminal.json").read_text(encoding="utf-8"))
    validate(release_data)
    if DIST.resolve() != ROOT / "dist" or DIST.is_symlink():
        raise ValueError("Unsafe build output")
    if DIST.exists():
        shutil.rmtree(DIST)
    (DIST / "en").mkdir(parents=True)
    shutil.copytree(SRC / "assets", DIST / "assets")
    for name in ("site.css", "site.js"):
        shutil.copyfile(SRC / name, DIST / name)
    for name in ("_headers", "_redirects", "robots.txt", "sitemap.xml"):
        shutil.copyfile(ROOT / name, DIST / name)
    for lang in ("zh", "en"):
        root = "" if lang == "zh" else "../"
        for page in PAGES:
            path = SRC / "pages" / lang / f"{page}.html"
            if page == "releases":
                body = render_releases(release_data, lang, REPO_URL)
                title = "版本记录 — Clash of Rust" if lang == "zh" else "Versions — Clash of Rust"
                target = DIST / ("" if lang == "zh" else "en") / "releases.html"
                target.write_text(layout(lang, page, title, title, body, release_data), encoding="utf-8")
                print(f"  {target.relative_to(ROOT)}")
                continue
            if not path.is_file():
                raise ValueError(f"Missing site page: {path.relative_to(ROOT)}")
            source = path.read_text(encoding="utf-8")
            meta = dict(re.findall(r"<!--\s*(title|description):\s*(.*?)\s*-->", source))
            body = re.sub(r"<!--\s*(title|description):.*?-->\n?", "", source)
            if page == "index":
                for block in ("memory", "showcase"):
                    body = body.replace("{{"+block+"}}", (SRC / "partials" / lang / (block+".html")).read_text(encoding="utf-8"))
            body = body.replace("{{releases:home}}", render_releases(release_data, lang, REPO_URL, compact=True))
            body = re.sub(r"\{\{tools:(\w+)\}\}", lambda match: render_tools(match.group(1), lang, data, ""), body)
            body = re.sub(r"\{\{shot:(\w+)\}\}", lambda match: render_capture(match.group(1), lang, root, captures), body)
            body = re.sub(r"\{\{terminal:(\w+)\}\}", lambda match: render_terminal(match.group(1), lang, terminal), body)
            body = (body.replace("{{root}}", root).replace("{{repo}}", REPO_URL).replace("{{docs}}", DOCS_URL)
                    .replace("{{star}}", STAR))
            if page == "download":
                body = render_downloads(body, release_data)
            body = re.sub(r' style="margin-top:(' + "|".join(re.escape(value) for value in SPACING)
                          + r')rem"', lambda match: ' data-space-top="' + match[1] + '"', body)
            if re.search(r'\sstyle\s*=', body):
                raise ValueError(f"{lang}/{page}: inline style is not allowed by the site policy")
            leftover = re.findall(r"\{\{[^}]*\}\}", body)
            if leftover:
                raise ValueError(f"{lang}/{page}: unresolved {leftover[:3]}")
            target = DIST / ("" if lang == "zh" else "en") / f"{page}.html"
            target.write_text(layout(lang, page, meta["title"], meta["description"], body, release_data), encoding="utf-8", newline="\n")
            print(f"  {target.relative_to(ROOT)}")


if __name__ == "__main__":
    build()

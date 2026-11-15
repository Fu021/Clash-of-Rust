"""Build the Clash of Rust site into site-next/dist. Standard library only.

    python site-next/build.py

English pages land in dist/, Chinese pages in dist/zh/. Illustrations and fonts
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
PAGES = ("index", "tools", "guide", "download", "releases")

TEXT = {
    "en": {
        "html_lang": "en", "skip": "Skip to content", "home": "Clash of Rust home",
        "nav_label": "Site", "nav": {"tools": "Network tools", "guide": "Guide", "releases": "Versions", "download": "Download"},
        "other_lang": "中文", "other_label": "切换到中文", "star": "Star",
        "foot_note": "Free software under GPL-3.0. Runs on the mihomo core.",
        "foot_art": "Illustrations were made for this site and are not screenshots of the app.",
        "foot_links": {"releases": "All releases", "issues": "Report a problem", "docs": "Developer docs"},
        "labels": {"checks": "What it checks", "when": "Use it when", "note": "Keep in mind", "options": "Options",
                   "none": "No extra options", "symptom": "What you see", "try": "Start with",
                   "flag": "Option", "range": "Range", "default": "Default", "meaning": "Meaning", "used": "Used by"},
    },
    "zh": {
        "html_lang": "zh-CN", "skip": "跳到正文", "home": "Clash of Rust 首页",
        "nav_label": "站内导航", "nav": {"tools": "网络工具", "guide": "使用指南", "releases": "版本记录", "download": "下载"},
        "other_lang": "English", "other_label": "Switch to English", "star": "Star",
        "foot_note": "自由软件，以 GPL-3.0 发布，内核是 mihomo。",
        "foot_art": "插画为本站绘制，不是应用截图。",
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


def tool_command(tool):
    return f'cor network {tool["id"]} {tool["arg"]}'


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
    root = "" if lang == "en" else "../"
    other =f"{root}zh/{page}.html" if lang == "en" else f"{root}{page}.html"
    current = ' aria-current="page"'
    nav = "".join(
        f'<a href="{name}.html"{current if name == page else ""}>{label}</a>'
        for name, label in text["nav"].items())
    redirect = ""
    foot = text["foot_links"]
    return f"""<!doctype html>
<html lang="{text['html_lang']}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{escape(title)}</title>
<meta name="description" content="{escape(description)}">
<meta name="theme-color" content="#f8f3e7">
<link rel="alternate" hreflang="en" href="{root}{page}.html">
<link rel="alternate" hreflang="zh-CN" href="{root}zh/{page}.html">
<link rel="icon" href="{root}assets/cat.png">
<link rel="preload" href="{root}assets/fonts/national-park.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="{root}site.css">
{redirect}<script type="application/json" id="release-data">{json.dumps(release_data, ensure_ascii=True).replace("<", "&#60;")}</script><script src="{root}site.js" defer></script>
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
    <div><a class="brand" href="index.html" aria-label="{text['home']}">{WORDMARK}</a><p>{text['foot_note']}</p><p>{text['foot_art']}</p></div>
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
    validate(release_data)
    if DIST.resolve() != ROOT / "dist" or DIST.is_symlink():
        raise ValueError("Unsafe build output")
    if DIST.exists():
        shutil.rmtree(DIST)
    (DIST / "zh").mkdir(parents=True)
    shutil.copytree(SRC / "assets", DIST / "assets")
    for name in ("site.css", "site.js"):
        shutil.copyfile(SRC / name, DIST / name)
    for lang in ("en", "zh"):
        root = "" if lang == "en" else "../"
        for page in PAGES:
            path = SRC / "pages" / lang / f"{page}.html"
            if page == "releases":
                body = render_releases(release_data, lang, REPO_URL)
                title = "版本记录 — Clash of Rust" if lang == "zh" else "Versions — Clash of Rust"
                target = DIST / ("" if lang == "en" else "zh") / "releases.html"
                target.write_text(layout(lang, page, title, title, body, release_data), encoding="utf-8")
                print(f"  {target.relative_to(ROOT)}")
                continue
            if not path.is_file():
                print(f"  missing {path.relative_to(ROOT)}")
                continue
            source = path.read_text(encoding="utf-8")
            meta = dict(re.findall(r"<!--\s*(title|description):\s*(.*?)\s*-->", source))
            body = re.sub(r"<!--\s*(title|description):.*?-->\n?", "", source)
            if page == "index":
                for block in ("memory", "showcase"):
                    body = body.replace("{{"+block+"}}", (SRC / "partials" / lang / (block+".html")).read_text(encoding="utf-8"))
            body = body.replace("{{releases:home}}", render_releases(release_data, lang, REPO_URL, compact=True))
            body = re.sub(r"\{\{tools:(\w+)\}\}", lambda match: render_tools(match.group(1), lang, data, ""), body)
            body = (body.replace("{{root}}", root).replace("{{repo}}", REPO_URL).replace("{{docs}}", DOCS_URL)
                    .replace("{{star}}", STAR))
            leftover = re.findall(r"\{\{[^}]*\}\}", body)
            if leftover:
                raise ValueError(f"{lang}/{page}: unresolved {leftover[:3]}")
            target = DIST / ("" if lang == "en" else "zh") / f"{page}.html"
            target.write_text(layout(lang, page, meta["title"], meta["description"], body, release_data), encoding="utf-8", newline="\n")
            print(f"  {target.relative_to(ROOT)}")


if __name__ == "__main__":
    build()

"""Build the Clash of Rust site into website/dist. Standard library only.

    python website/build.py

Chinese pages land in dist/, English pages in dist/en/. Illustrations and fonts
come ready-made from src/assets; prepare_assets.py regenerates those.
"""
from argparse import ArgumentParser
from html import escape
import json
from pathlib import Path
import re
import shutil
from release_content import validate, render as render_releases, SUFFIXES, OPTIONAL_SUFFIXES

ROOT = Path(__file__).resolve().parent
SRC = ROOT / "src"
DIST = ROOT / "dist"
REPO_URL = "https://github.com/Fu021/Clash-of-Rust"
DOCS_URL = f"{REPO_URL}/blob/main/docs"
SITE_URL = "https://clash-of-rust.fu021.shop"
PAGES = ("index", "start", "guide", "tui", "cli", "tools", "download", "releases")
SPACING = ("0.9", "1", "1.1", "1.2", "1.3", "1.4", "1.6")

TEXT = {
    "en": {
        "html_lang": "en", "skip": "Skip to content", "home": "Clash of Rust home",
        "nav_label": "Site", "menu": "Menu", "nav": {"start": "Get started", "guide": "Manual", "tools": "Network tools", "download": "Download", "releases": "Versions"},
        "other_lang": "中文", "other_label": "切换到中文", "star": "Star",
        "foot_note": "Free software under GPL-3.0. Runs on the mihomo core.",
        "visitors": "Site visitors", "visitors_scope": "Unique browsers since counting began",
        "foot_links": {"releases": "All releases", "issues": "Report a problem", "docs": "Developer docs"},
        "labels": {"checks": "What it checks", "when": "Use it when", "note": "Keep in mind", "options": "Options",
                   "none": "No extra options", "symptom": "What you see", "try": "Start with",
                   "flag": "Option", "range": "Range", "default": "Default", "meaning": "Meaning", "used": "Used by"},
    },
    "zh": {
        "html_lang": "zh-CN", "skip": "跳到正文", "home": "Clash of Rust 首页",
        "nav_label": "站内导航", "menu": "导航", "nav": {"start": "快速上手", "guide": "功能手册", "tools": "网络工具", "download": "下载", "releases": "版本记录"},
        "other_lang": "English", "other_label": "Switch to English", "star": "Star",
        "foot_note": "开源 mihomo 客户端 · GPL-3.0",
        "visitors": "网站访客", "visitors_scope": "自启用起，按浏览器去重",
        "foot_links": {"releases": "全部版本", "issues": "反馈问题", "docs": "开发文档"},
        "labels": {"checks": "查什么", "when": "什么时候用", "note": "留意", "options": "可调参数",
                   "none": "没有额外参数", "symptom": "现象", "try": "先用",
                   "flag": "参数", "range": "范围", "default": "默认", "meaning": "含义", "used": "适用工具"},
    },
}

STAR = '<svg class="star" viewBox="0 0 12 12" aria-hidden="true"><path d="M5 0h2v3h2v1h3v2h-1v1h-1v1h1v2h1v2h-2v-1H9v-1H8V9H4v1H3v1H2v1H0v-2h1V8h1V7H1V6H0V4h3V3h2z"/></svg>'

THEME_ICON = ('<svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.7">'
              '<g data-theme-icon="auto"><circle cx="12" cy="12" r="8"/><path d="M12 4a8 8 0 0 1 0 16z" fill="currentColor"/></g>'
              '<g data-theme-icon="light" hidden><circle cx="12" cy="12" r="4"/><path d="M12 1v3m0 16v3M1 12h3m16 0h3M4 4l2 2m12 12 2 2M4 20l2-2M18 6l2-2"/></g>'
              '<g data-theme-icon="dark" hidden><path d="M20.5 14.5A9 9 0 0 1 9.5 3.5a9 9 0 1 0 11 11z"/></g></svg>')

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
    capture = captures[lang][name]
    label = CAPTURES[name][0 if lang == "zh" else 1]
    phone = capture["platform"] == "android"
    platform = "Android" if phone else ("桌面" if lang == "zh" else "Desktop")
    note = "演示数据" if lang == "zh" else "Demo data"
    expand = "放大：" if lang == "zh" else "Enlarge: "
    light, dark = (f'{root}assets/{escape(capture["files"][theme])}' for theme in ("light", "dark"))
    image = (f'width="{capture["width"]}" height="{capture["height"]}" '
             f'alt="{escape(label)}" loading="lazy"')
    fallback = "".join(
        f'<a class="frame capture-open capture-fallback-{theme}" href="{source}" '
        f'aria-label="{expand}{escape(label)}"><img src="{source}" {image}></a>'
        for theme, source in (("light", light), ("dark", dark))
    )
    return (f'<figure class="product-shot{" phone" if phone else ""}" data-capture-name="{name}" data-capture-language="{lang}" '
            f'data-capture-version="{escape(capture["version"])}">'
            f'<a class="frame capture-open capture-script" href="{light}" data-enlarge aria-label="{expand}{escape(label)}">'
            f'<picture><source data-night media="(prefers-color-scheme: dark)" srcset="{dark}">'
            f'<img src="{light}" {image}></picture></a><noscript>{fallback}</noscript>'
            f'<figcaption>{escape(label)} · {platform} · {escape(capture["version"])} · {note}</figcaption></figure>')


def render_terminal(name, lang, frames):
    frame = frames[lang][name]
    if frame["source"] != "actual-cor-pty":
        raise ValueError("Terminal illustrations must use actual PTY frames")
    tones = {"plain", "muted", "accent", "selected", "warning"}
    lines = []
    for line in frame["lines"]:
        if line["tone"] not in tones:
            raise ValueError("Invalid terminal tone")
        rule = " terminal-rule" if re.fullmatch(r"─+", line["text"].strip()) else ""
        lines.append(f'<span class="tone-{line["tone"]}{rule}">{escape(line["text"])}</span>')
    label = "实际终端画面 · 固定测试输入" if lang == "zh" else "Actual terminal frame · Fixed test input"
    return (f'<figure class="product-shot terminal-shot" data-capture-language="{lang}" '
            f'data-capture-version="{escape(frame["version"])}">'
            '<div class="terminal-title" aria-hidden="true">cor ui</div>'
            f'<pre class="terminal-capture" tabindex="0" aria-label="{label}">' + "\n".join(lines) + "</pre>"
            f'<figcaption>{label} · Linux · {escape(frame["version"])}</figcaption></figure>')


def tool_command(tool):
    return f'cor network {tool["id"]} {tool["arg"]}'.rstrip()


def render_downloads(body, release_data):
    stable = release_data["downloads"]["stable"]
    version = stable["tag"].removeprefix("v")
    body = body.replace("{{stable_version}}", escape(version)).replace("{{stable_tag}}", escape(stable["tag"]))
    for system in ("windows", "linux", "macos", "android"):
        rows = []
        for suffix in SUFFIXES + OPTIONAL_SUFFIXES:
            if not suffix.startswith(system + "-") or suffix not in stable["sizes"]:
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
            rows += (f'<tr role="row"><th scope="row"><code>{option["flag"]}</code></th>'
                     f'<td role="cell" data-label="{labels["meaning"]}">{escape(option[lang])}</td>'
                     f'<td role="cell" data-label="{labels["range"]}">{option["range"]}</td>'
                     f'<td role="cell" data-label="{labels["default"]}">{option["default"]}</td>'
                     f'<td role="cell" data-label="{labels["used"]}">{escape(used)}</td></tr>')
        return (f'<div class="table-scroll"><table class="plain tool-options" role="table"><thead><tr><th scope="col">{labels["flag"]}</th>'
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
                target = {"URL": "https://example.com", "HOST": "example.com",
                          "HOST[:PORT]": "example.com:443", "SERVER[:PORT]": "example.com:53"}[tool["arg"]]
                name = escape(copy["name"])
                example = escape(target)
                command = escape(f'cor network {tool["id"]} {target}')
                if lang == "zh":
                    usage_label = "操作示例"
                    usage = (f'<summary>GUI、TUI 与 CLI 示例</summary>'
                             f'<p><b>GUI：</b>在“工具 → 网络诊断”选择“{name}”，输入 <code>{example}</code> 后运行。</p>'
                             f'<p><b>TUI：</b>运行 <code>cor</code>，按 <kbd>6</kbd> 并用 <kbd>←</kbd> / <kbd>→</kbd> 选择“{name}”。按 <kbd>/</kbd> 输入 <code>{example}</code>，回车结束输入，再按回车运行。</p>'
                             f'<p><b>CLI：</b><code>{command}</code></p>')
                else:
                    usage_label = "Usage examples"
                    usage = (f'<summary>GUI, TUI and CLI examples</summary>'
                             f'<p><b>GUI:</b> Under Tools → Network diagnostics, select {name}, enter <code>{example}</code> and run the check.</p>'
                             f'<p><b>TUI:</b> Run <code>cor</code>, press <kbd>6</kbd> and use <kbd>←</kbd> / <kbd>→</kbd> to select {name}. Press <kbd>/</kbd>, enter <code>{example}</code> and press Enter to finish input. Press Enter again to run.</p>'
                             f'<p><b>CLI:</b> <code>{command}</code></p>')
                parts.append(
                    f'<article class="tool" id="{tool["id"]}"><div class="tool-side"><h3>{escape(copy["name"])}</h3>'
                    f'<code class="command">{escape(tool_command(tool))}</code>'
                    f'<p class="label">{labels["options"]}</p><ul class="options">{options}</ul></div>'
                    f'<dl class="tool-body"><div><dt>{labels["checks"]}</dt><dd>{escape(copy["checks"])}</dd></div>'
                    f'<div><dt>{labels["when"]}</dt><dd>{escape(copy["when"])}</dd></div>'
                    f'<div><dt>{labels["note"]}</dt><dd>{escape(copy["note"])}</dd></div>'
                    f'<div><dt>{usage_label}</dt><dd><details>{usage}</details></dd></div></dl></article>')
        return "".join(parts)
    raise ValueError(f"unknown tools block: {kind}")


def render_artwork(body, root):
    def night_sources(match):
        picture = match[0]
        image = re.search(r'assets/(hero|start|manual|guide|tools|station|versions)\.webp', picture)
        if not image:
            return picture
        name = image[1]
        return (f'<picture><source data-night data-small media="(prefers-color-scheme: dark) and (max-width: 640px)" '
                f'srcset="{root}assets/{name}-night-small.webp">'
                f'<source data-night media="(prefers-color-scheme: dark)" srcset="{root}assets/{name}-night.webp">'
                + picture.removeprefix("<picture>"))
    return re.sub(r"<picture>.*?</picture>", night_sources, body, flags=re.S)


def site_path(lang, page):
    return ("/en/" if lang == "en" else "/") + ("" if page == "index" else page)


def canonical_links(document, lang):
    def rewrite(match):
        prefix, name = match.groups()
        target = {"../": "zh", "en/": "en"}.get(prefix, lang)
        return f' href="{site_path(target, name)}'

    pages = "|".join(PAGES)
    return re.sub(rf' href="(\.\./|en/)?({pages})\.html(?=[?#"])', rewrite, document)


def layout(lang, page, title, description, body, release_data, file_links=False):
    text = TEXT[lang]
    root = "" if lang == "zh" else "../"
    other = f"en/{page}.html" if lang == "zh" else f"../{page}.html"
    canonical = SITE_URL + site_path(lang, page)
    current = ' aria-current="page"'
    nav = "".join(
        f'<a href="{name}.html"{current if name == page or name == "guide" and page in ("tui", "cli") else ""}>{label}</a>'
        for name, label in text["nav"].items())
    wordmark = (f'<picture class="brand-wordmark"><source data-night media="(prefers-color-scheme: dark)" '
                f'srcset="{root}assets/wordmark-dark.svg"><img class="wordmark" src="{root}assets/wordmark.svg" '
                'width="116" height="19" alt=""></picture>')
    foot = text["foot_links"]
    body = render_artwork(body, root)
    release_json = json.dumps(release_data, ensure_ascii=True).replace("<", "\\u003c")
    structured_data = ""
    if page == "index":
        website = {"@context": "https://schema.org", "@type": "WebSite",
                   "name": "Clash of Rust", "url": SITE_URL + "/"}
        structured_data = '<script type="application/ld+json">' + json.dumps(website) + '</script>\n'
    document = f"""<!doctype html>
<html lang="{text['html_lang']}">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{escape(title)}</title>
<meta name="description" content="{escape(description)}">
<meta name="google-site-verification" content="DVQrkZ9F6W1fmzfe2hot12n9274smsKHaaTPcB_gUNE">
<meta name="theme-color" content="#f8f3e7">
<meta name="color-scheme" content="light dark">
<script src="{root}theme.js"></script>
<link rel="canonical" href="{canonical}">
<link rel="alternate" hreflang="en" href="{SITE_URL}{site_path('en', page)}">
<link rel="alternate" hreflang="zh-CN" href="{SITE_URL}{site_path('zh', page)}">
<link rel="alternate" hreflang="x-default" href="{SITE_URL}{site_path('zh', page)}">
<meta property="og:site_name" content="Clash of Rust">
{structured_data}<link rel="icon" href="{root}assets/cat.png">
<link rel="preload" href="{root}assets/fonts/national-park.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="{root}assets/fonts/departure-mono.woff2" as="font" type="font/woff2" crossorigin>
<link rel="preload" href="{root}assets/fonts/noto-sans-sc-display.woff2" as="font" type="font/woff2" crossorigin>
<link rel="stylesheet" href="{root}site.css">
<script type="application/json" id="release-data">{release_json}</script><script src="{root}site.js" defer></script>
</head>
<body class="page-{page}">
<a class="skip" href="#main">{text['skip']}</a>
<header class="top">
  <div class="wrap">
    <a class="brand" href="index.html" aria-label="{text['home']}"><img src="{root}assets/cat.png" alt="" width="30" height="30">{wordmark}</a>
    <nav class="main-nav" aria-label="{text['nav_label']}">{nav}</nav>
    <div class="header-actions">
      <a class="lang" href="{other}" lang="{'zh-CN' if lang == 'en' else 'en'}" aria-label="{text['other_label']}" data-set-lang="{'zh' if lang == 'en' else 'en'}">{text['other_lang']}</a>
      <a class="btn small ghost" href="{REPO_URL}" target="_blank" rel="noopener">{STAR}{text['star']}<span data-stars></span></a>
      <button class="theme-toggle js-only" type="button" data-theme-toggle aria-label="{'主题：自动' if lang == 'zh' else 'Theme: automatic'}">{THEME_ICON}</button>
    </div>
    <details class="mobile-menu"><summary>{text['menu']}</summary><nav aria-label="{text['nav_label']}">{nav}<a href="{REPO_URL}" target="_blank" rel="noopener">GitHub ↗</a></nav></details>
  </div>
</header>
<main id="main">
{body}
</main>
<dialog class="image-view" data-image-view aria-label="{'软件画面' if lang == 'zh' else 'Application screenshot'}">
  <div class="image-tools"><button class="btn small" type="button" data-image-close aria-label="{'关闭大图' if lang == 'zh' else 'Close image'}">{'关闭' if lang == 'zh' else 'Close'} · Esc</button></div>
  <img alt="">
</dialog>
<footer class="foot">
  <div class="track" aria-hidden="true"></div>
  <div class="wrap">
    <div>
      <a class="brand" href="index.html" aria-label="{text['home']}">{wordmark}</a>
      <p>{text['foot_note']}</p>
      <p class="site-visitors" id="site-visitors" title="{text['visitors_scope']}" hidden>
        <span class="site-visitors-icon" aria-hidden="true"></span>
        <span>{text['visitors']}</span>
        <strong data-visitor-count></strong>
      </p>
    </div>
    <nav aria-label="{text['nav_label']}">{nav}</nav>
    <nav><a href="{REPO_URL}" target="_blank" rel="noopener">GitHub</a><a href="{REPO_URL}/releases" target="_blank" rel="noopener">{foot['releases']}</a><a href="{REPO_URL}/issues" target="_blank" rel="noopener">{foot['issues']}</a><a href="{DOCS_URL}/README.md" target="_blank" rel="noopener">{foot['docs']}</a></nav>
  </div>
</footer>
</body>
</html>
"""
    return document if file_links else canonical_links(document, lang)


def build(file_links=False):
    data = json.loads((SRC / "content" / "tools.json").read_text(encoding="utf-8"))
    release_data = json.loads((SRC / "content" / "releases.json").read_text(encoding="utf-8"))
    memory = json.loads((SRC / "content" / "memory.json").read_text(encoding="utf-8"))
    captures = json.loads((SRC / "content" / "screenshots.json").read_text(encoding="utf-8"))
    terminal = json.loads((SRC / "content" / "terminal.json").read_text(encoding="utf-8"))
    validate(release_data)
    if DIST.resolve() != ROOT / "dist" or DIST.is_symlink():
        raise ValueError("Unsafe build output")
    if DIST.exists():
        shutil.rmtree(DIST)
    (DIST / "en").mkdir(parents=True)
    shutil.copytree(SRC / "assets", DIST / "assets")
    shutil.copyfile(ROOT.parent / "resources/icons/wordmark-light.svg", DIST / "assets/wordmark.svg")
    shutil.copyfile(ROOT.parent / "resources/icons/wordmark-dark.svg", DIST / "assets/wordmark-dark.svg")
    for name in ("site.css", "site.js", "theme.js"):
        shutil.copyfile(SRC / name, DIST / name)
    for name in ("_headers", "_redirects", "robots.txt", "sitemap.xml"):
        shutil.copyfile(ROOT / name, DIST / name)
    for lang in ("zh", "en"):
        root = "" if lang == "zh" else "../"
        for page in PAGES:
            path = SRC / "pages" / lang / f"{page}.html"
            if page == "releases":
                body = render_releases(release_data, lang, REPO_URL, root=root)
                title = "版本记录 — Clash of Rust" if lang == "zh" else "Versions — Clash of Rust"
                target = DIST / ("" if lang == "zh" else "en") / "releases.html"
                target.write_text(layout(lang, page, title, title, body, release_data, file_links), encoding="utf-8")
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
                body = body.replace("{{memory:version}}", escape(memory["version"])).replace("{{memory:run}}", escape(memory["run"]))
                body = re.sub(r"\{\{memory:([\w-]+):(client|total)\}\}",
                              lambda match: escape(memory[match[1]][match[2]]), body)
            body = body.replace("{{releases:home}}", render_releases(release_data, lang, REPO_URL, compact=True))
            manual_labels = ("窗口 GUI", "终端 TUI", "命令行 CLI") if lang == "zh" else ("Window · GUI", "Terminal · TUI", "Commands · CLI")
            manuals = '<nav class="manual-nav" aria-label="' + ("手册类型" if lang == "zh" else "Manuals") + '">'
            manuals += "".join(f'<a href="{name}.html"{" aria-current=page" if name == page else ""}>{label}</a>'
                               for name, label in zip(("guide", "tui", "cli"), manual_labels))
            body = body.replace("{{manuals}}", manuals + "</nav>")
            body = re.sub(r'<nav class="toc" aria-label="([^"]+)">\s*<p class="label">[^<]+</p>(.*?)</nav>',
                          lambda match: f'<details class="toc" data-toc open><summary>{match[1]}</summary>'
                          f'<nav aria-label="{match[1]}">{match[2]}</nav></details>', body, flags=re.S)
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
            target.write_text(layout(lang, page, meta["title"], meta["description"], body, release_data, file_links), encoding="utf-8", newline="\n")
            print(f"  {target.relative_to(ROOT)}")


if __name__ == "__main__":
    parser = ArgumentParser(description=__doc__)
    parser.add_argument("--file-links", action="store_true", help="use .html links for a plain file server")
    build(file_links=parser.parse_args().file_links)

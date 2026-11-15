"""Build the static product website with Python's standard library."""
import argparse
from datetime import datetime, timezone
from html import escape
import json
from pathlib import Path
import re
import shutil
import struct
import urllib.request

SOURCE = Path(__file__).resolve().parent
ROOT = SOURCE.parent
REPOSITORY = "Fu021/Clash-of-Rust"
GITHUB = f"https://github.com/{REPOSITORY}"
PLATFORMS = (
    ("windows", "Windows", "原生桌面客户端", (
        ("x64", "windows-x64-setup.exe", ""),
        ("ARM64", "windows-arm64-setup.exe", ""),
    )),
    ("linux", "Debian / Ubuntu", "DEB · Ubuntu 20.04 起", (
        ("x64", "linux-amd64.deb", ""),
        ("ARM64", "linux-arm64.deb", ""),
    )),
    ("macos", "macOS", "macOS 14 起", (
        ("Apple Silicon", "macos-arm64.dmg", ""),
        ("Intel", "macos-x64.dmg", ""),
    )),
    ("android", "Android", "Android 8.0 起", (
        ("ARM64", "android-arm64.apk", "多数手机和平板"),
        ("ARMv7", "android-armv7.apk", "较早的 32 位设备"),
        ("x64", "android-x64.apk", "x86_64 设备与模拟器"),
    )),
)
PAGES = {
    "index": ("Clash of Rust · 看清连接，从容掌控网络", "home"),
    "features": ("功能 · Clash of Rust", "features"),
    "download": ("下载 · Clash of Rust", "download"),
}


def validate_release(record, prerelease):
    if (not isinstance(record, dict) or record.get("draft") is not False
            or record.get("prerelease") is not prerelease):
        raise ValueError("Unexpected release channel")
    tag = record.get("tag_name")
    if (not isinstance(tag, str)
            or not re.fullmatch(r"v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", tag)):
        raise ValueError("Unsupported release tag")
    if record.get("html_url") != f"{GITHUB}/releases/tag/{tag}":
        raise ValueError("Unexpected release page")
    if not isinstance(record.get("assets"), list):
        raise ValueError("Invalid release assets")
    assets = {}
    for asset in record["assets"]:
        if not isinstance(asset, dict) or not isinstance(asset.get("name"), str):
            raise ValueError("Invalid release asset")
        if asset["name"] in assets:
            raise ValueError("Duplicate release asset")
        assets[asset["name"]] = asset
    for _, _, _, variants in PLATFORMS:
        for _, suffix, _ in variants:
            filename = f"Clash-of-Rust-{tag[1:]}-{suffix}"
            for name in (filename, filename + ".sha256"):
                asset = assets.get(name, {})
                if (asset.get("state") != "uploaded" or type(asset.get("size")) is not int
                        or asset["size"] <= 0
                        or asset.get("browser_download_url") != f"{GITHUB}/releases/download/{tag}/{name}"):
                    raise ValueError(f"Missing or invalid release asset: {name}")
    return record


def request(path):
    command = urllib.request.Request(
        f"https://api.github.com/repos/{REPOSITORY}/{path}",
        headers={"Accept": "application/vnd.github+json", "User-Agent": "Clash-of-Rust-website"},
    )
    with urllib.request.urlopen(command, timeout=30) as response:
        payload = response.read(4 * 1024 * 1024 + 1)
    if len(payload) > 4 * 1024 * 1024:
        raise ValueError("Release metadata exceeds size limit")
    return json.loads(payload)


def compact(record):
    return {
        key: record[key] for key in
        ("tag_name", "html_url", "prerelease", "draft", "published_at")
    } | {
        "assets": [
            {key: asset[key] for key in
             ("name", "size", "state", "browser_download_url")}
            for asset in record["assets"]
        ],
    }


def refresh():
    stable = validate_release(request("releases/latest"), False)
    candidates = sorted(
        (record for record in request("releases?per_page=10")
         if record.get("prerelease") and not record.get("draft")),
        key=lambda record: record["published_at"], reverse=True,
    )
    preview = next((record for record in candidates
                    if all(any(asset["name"] == f"Clash-of-Rust-{record['tag_name'][1:]}-{suffix}"
                               for asset in record["assets"])
                           for _, _, _, variants in PLATFORMS for _, suffix, _ in variants)), None)
    if preview is None:
        raise ValueError("No complete public prerelease was found")
    validate_release(preview, True)
    value = {
        "repository": REPOSITORY,
        "checked_at": datetime.now(timezone.utc).isoformat(),
        "stable": compact(stable),
        "preview": compact(preview),
    }
    (SOURCE / "releases.json").write_text(
        json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )


def icon(name, extra=""):
    return f'<svg class="icon {extra}" aria-hidden="true"><use href="assets/icons.svg#{name}"></use></svg>'


def download_cards(record):
    cards = []
    assets = {asset["name"]: asset for asset in record["assets"]}
    for identifier, title, description, variants in PLATFORMS:
        rows = []
        for architecture, suffix, note in variants:
            name = f"Clash-of-Rust-{record['tag_name'][1:]}-{suffix}"
            asset = assets[name]
            rows.append(
                f'<div class="package-row" data-suffix="{suffix}">'
                f'<div><strong>{architecture}</strong><span class="package-note">{note}</span></div>'
                f'<div class="package-actions"><a class="package-download" data-download '
                f'href="{escape(asset["browser_download_url"], quote=True)}">'
                f'{icon("download")}<span data-size>{asset["size"] / 1048576:.1f} MiB</span>'
                f'<span class="sr-only">，下载 {title} {architecture}</span></a>'
                f'<a class="checksum" data-checksum href="{escape(asset["browser_download_url"], quote=True)}.sha256"'
                f' aria-label="{title} {architecture} SHA256 校验文件">SHA256</a></div></div>'
            )
        cards.append(
            f'<article class="download-card" data-platform="{identifier}">'
            f'<div class="platform-heading"><span class="platform-icon">{icon(identifier)}</span>'
            f'<div><h2>{title}</h2><p>{description}</p></div></div>'
            f'{"".join(rows)}</article>'
        )
    return "".join(cards)


def build(output):
    output = output.resolve()
    if (output == SOURCE or output == ROOT or output in SOURCE.parents
            or output == SOURCE / "static" or SOURCE / "static" in output.parents
            or output == SOURCE / "pages" or SOURCE / "pages" in output.parents):
        raise ValueError("Choose a separate static output directory")
    releases = json.loads((SOURCE / "releases.json").read_text(encoding="utf-8"))
    if releases.get("repository") != REPOSITORY:
        raise ValueError("Unexpected repository")
    validate_release(releases["stable"], False)
    validate_release(releases["preview"], True)
    output.mkdir(parents=True, exist_ok=True)
    shutil.copytree(SOURCE / "static", output / "assets", dirs_exist_ok=True)
    shutil.copy2(ROOT / "resources/icons/wordmark-dark.svg", output / "assets/wordmark.svg")
    shutil.copy2(ROOT / "resources/icons/app.png", output / "assets/app.png")
    shutil.copy2(ROOT / "LICENSE", output / "assets/LICENSE.txt")
    (output / "assets/releases.json").write_text(json.dumps(releases, ensure_ascii=False), encoding="utf-8")
    (output / "assets/releases.js").write_text(
        "export const releases = " + json.dumps(releases, ensure_ascii=False) + ";\n", encoding="utf-8"
    )
    layout = (SOURCE / "layout.html").read_text(encoding="utf-8")
    context = {
        "stable": escape(releases["stable"]["tag_name"][1:]),
        "preview": escape(releases["preview"]["tag_name"][1:]),
        "download_cards": download_cards(releases["stable"]),
    }
    for name in ("home", "nodes", "rules", "applications", "network", "protection"):
        payload = (SOURCE / f"static/screenshots/{name}.png").read_bytes()[:24]
        if payload[:8] != b"\x89PNG\r\n\x1a\n":
            raise ValueError("Invalid product screenshot")
        width, height = struct.unpack(">II", payload[16:24])
        context[name + "_width"] = str(width)
        context[name + "_height"] = str(height)
    for name, (title, current) in PAGES.items():
        values = context | {
            "title": title,
            "page": current,
            "content": (SOURCE / f"pages/{name}.html").read_text(encoding="utf-8"),
        } | {f"nav_{page}": 'aria-current="page"' if page == current else ""
             for page in ("home", "features", "download")}
        text = layout
        for _ in range(2):
            for key, value in values.items():
                text = text.replace("{{" + key + "}}", value)
        if re.search(r"\{\{[a-z_]+\}\}", text):
            raise ValueError("Unresolved page template")
        (output / f"{name}.html").write_text(text, encoding="utf-8")
    shutil.copy2(SOURCE / "_headers", output / "_headers")
    print(json.dumps({"output": str(output), "pages": len(PAGES),
                      "stable": releases["stable"]["tag_name"],
                      "preview": releases["preview"]["tag_name"]}, ensure_ascii=False))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=SOURCE / "dist")
    parser.add_argument("--refresh-releases", action="store_true")
    arguments = parser.parse_args()
    if arguments.refresh_releases:
        refresh()
    build(arguments.output.resolve())

"""Prepare committed site assets from sources: illustrations, screenshots and fonts.

Needs Pillow and fontTools (with brotli). Run it after changing an illustration,
or after changing Chinese headings, so the subset font covers the new characters:

    python website/prepare_assets.py

Font sources are read from website/.cache/fonts (not committed); see README.md.
"""
from html.parser import HTMLParser
import argparse
import json
from pathlib import Path
import re
import shutil
import subprocess
import tomllib

from PIL import Image

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent
SRC = ROOT / "src"
RAW = SRC / "art" / "raw"
OUT = SRC / "assets"
FONT_CACHE = ROOT / ".cache" / "fonts"
ART_NAMES = ("hero", "tools", "terminal", "station", "guide", "start", "manual", "versions")
NIGHT_ART_NAMES = tuple(name + "-night" for name in ART_NAMES if name != "terminal")

BAYER = [
    [0, 32, 8, 40, 2, 34, 10, 42], [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38], [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41], [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37], [63, 31, 55, 23, 61, 29, 53, 21],
]


def webp(image, name, quality=84, lossless=False):
    path = OUT / name
    image.save(path, "WEBP", quality=quality, method=6, lossless=lossless)
    print(f"  {name:28} {image.size[0]}x{image.size[1]}  {path.stat().st_size / 1024:.0f} KiB")


def illustrations(names=None):
    for name in (*ART_NAMES, *NIGHT_ART_NAMES):
        if names and name not in names:
            continue
        banner = Image.open(RAW / f"{name}.png").convert("RGB")
        webp(banner, f"{name}.webp", 86 if name == "hero" else 84)
        webp(banner.resize((banner.width // 2, banner.height // 2), Image.LANCZOS), f"{name}-small.webp", 84)
        if name == "hero":
            webp(banner.resize((banner.width * 2, banner.height * 2), Image.NEAREST), "hero@2x.webp", 76)


def screenshots(options):
    if not options.gui_previews or not options.android_preview:
        raise ValueError("shots requires Chinese and English desktop and Android preview directories")
    version = source_version(options.gui_commit)
    terminal = terminal_records(options.terminal_previews, options.terminal_commit)
    if any(frame["version"] != version for group in terminal.values() for frame in group.values()):
        raise ValueError("GUI and terminal previews must use the same release")
    desktop_pages = {"home": "home", "nodes": "proxies", "subscriptions": "subscriptions",
                     "rules": "rules", "network": "network", "protection": "ai-check"}
    copies = []
    records = {}
    for lang, desktop, android in zip(("zh", "en"), options.gui_previews, options.android_preview, strict=True):
        records[lang] = {}
        sources = {name: desktop / page for name, page in desktop_pages.items()}
        sources["applications"] = android / "android/settings"
        for name, stem in sources.items():
            files = {}
            sizes = set()
            for theme in ("light", "dark"):
                source = stem.with_name(f"{stem.name}-{theme}.png")
                with Image.open(source) as image:
                    if image.format != "PNG":
                        raise ValueError(f"Not a PNG preview: {source}")
                    sizes.add(image.size)
                    image.verify()
                lang_suffix = "-en" if lang == "en" else ""
                theme_suffix = "-dark" if theme == "dark" else ""
                target = OUT / f"app-{name}{lang_suffix}{theme_suffix}.png"
                files[theme] = target.name
                copies.append((source, target))
            if len(sizes) != 1:
                raise ValueError(f"Light and dark preview sizes differ: {stem}")
            (width, height), = sizes
            records[lang][name] = {"files": files, "width": width, "height": height,
                                   "platform": "android" if name == "applications" else "desktop",
                                   "version": version, "commit": options.gui_commit,
                                   "source": "production-slint-preview"}
    for source, target in copies:
        shutil.copyfile(source, target)
    shutil.copyfile(REPO / "resources/icons/app.png", OUT / "cat.png")
    write_captures("screenshots", records)
    write_captures("terminal", terminal)
    print(f"  {len(copies)} bilingual light and dark GUI previews")


def source_version(commit):
    if not commit or not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("Capture source must be a full Git commit")
    manifest = subprocess.check_output(["git", "show", f"{commit}:Cargo.toml"], cwd=REPO, text=True)
    return tomllib.loads(manifest)["workspace"]["package"]["version"]


def terminal_records(previews, commit):
    if not previews:
        raise ValueError("terminal requires Chinese and English actual PTY preview directories")
    version = source_version(commit)
    terminal = {}
    for lang, directory in zip(("zh", "en"), previews, strict=True):
        terminal[lang] = {}
        for name in ("home", "network"):
            record = json.loads((directory / f"background-tui-{name}.json").read_text(encoding="utf-8"))
            text = "\n".join(line["text"] for line in record["lines"])
            navigation = "8 操作" if lang == "zh" else "8 Commands"
            if record.get("source") != "actual-cor-pty" or f"v{version}" not in text or navigation not in text:
                raise ValueError(f"Expected a {lang} {name} PTY capture from {version} in {directory}")
            terminal[lang][name] = {**record, "version": version, "commit": commit}
    return terminal


def write_captures(name, records):
    (SRC / f"content/{name}.json").write_text(json.dumps(records, ensure_ascii=False, indent=2) + "\n",
                                             encoding="utf-8")


def dither_tiles(block=8, columns=32, rows=12):
    """Ordered-dither alpha ramps, used as CSS masks where a picture meets the paper."""
    tile = Image.new("RGBA", (columns * block, rows * block), (0, 0, 0, 0))
    pixels = tile.load()
    for row in range(rows):
        coverage = (row + 0.5) / rows
        for column in range(columns):
            if (BAYER[row % 8][column % 8] + 0.5) / 64 < coverage:
                for y in range(block):
                    for x in range(block):
                        pixels[column * block + x, row * block + y] = (0, 0, 0, 255)
    tile.save(OUT / "dither-in.png", optimize=True)
    tile.transpose(Image.FLIP_TOP_BOTTOM).save(OUT / "dither-out.png", optimize=True)
    print("  dither-in.png, dither-out.png")


class HeadingText(HTMLParser):
    """Collects the text of headings and other elements set in the display face."""
    TAGS = {"h1", "h2", "h3", "summary", "dt", "th", "strong", "b", "a", "button", "nav"}

    def __init__(self):
        super().__init__()
        self.depth = 0
        self.text = set()

    def handle_starttag(self, tag, attributes):
        classes = dict(attributes).get("class") or ""
        if self.depth or tag in self.TAGS or "display" in classes.split():
            if tag not in {"br", "img", "wbr", "hr"}:
                self.depth += 1

    def handle_endtag(self, tag):
        if self.depth:
            self.depth -= 1

    def handle_data(self, data):
        if self.depth:
            self.text.update(data)


def fonts():
    from fontTools.subset import Options, Subsetter
    from fontTools.ttLib import TTFont
    from fontTools.varLib import instancer

    target = OUT / "fonts"
    target.mkdir(parents=True, exist_ok=True)

    def subset(source, name, text=None, unicodes=None, instance=None):
        if not source.is_file():
            print(f"  {name}: source missing ({source.relative_to(ROOT)}), kept the existing file")
            return
        font = TTFont(source)
        options = Options()
        options.layout_features = ["*"]
        options.flavor = "woff2"
        subsetter = Subsetter(options)
        subsetter.populate(text=text or "", unicodes=unicodes or [])
        subsetter.subset(font)
        if instance:
            font = instancer.instantiateVariableFont(font, instance)
        font.flavor = "woff2"
        font.save(target / name)
        print(f"  fonts/{name:22} {(target / name).stat().st_size / 1024:.0f} KiB")

    latin = list(range(0x20, 0x7F)) + list(range(0xA0, 0x100)) + [
        0x2013, 0x2014, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2026, 0x2192, 0x2197, 0x2193, 0x00D7, 0x2605]
    subset(FONT_CACHE / "nationalpark/NationalPark.ttf", "national-park.woff2", unicodes=latin)
    departure = FONT_CACHE / "departure/DepartureMono-1.500/DepartureMono-Regular.woff2"
    if departure.is_file():
        shutil.copyfile(departure, target / "departure-mono.woff2")
        print(f"  fonts/departure-mono.woff2   {departure.stat().st_size / 1024:.0f} KiB")

    collector = HeadingText()
    for page in sorted((SRC / "pages" / "zh").glob("*.html")):
        collector.feed(page.read_text(encoding="utf-8"))
    for page in sorted((SRC / "partials" / "zh").glob("*.html")):
        collector.feed(page.read_text(encoding="utf-8"))
    for extra in (SRC / "content").glob("*.json"):
        collector.text.update(extra.read_text(encoding="utf-8"))
    collector.text.update((ROOT / "build.py").read_text(encoding="utf-8"))
    collector.text.update((ROOT / "release_content.py").read_text(encoding="utf-8"))
    collector.text.update((SRC / "site.js").read_text(encoding="utf-8"))
    cjk = "".join(sorted(character for character in collector.text if ord(character) >= 0x2E80))
    print(f"  Chinese display characters: {len(cjk)}")
    subset(FONT_CACHE / "notosanssc/NotoSansSC.ttf", "noto-sans-sc-display.woff2", text=cjk, instance={"wght": 500})
    for licence, name in (("nationalpark/OFL.txt", "OFL-National-Park.txt"), ("notosanssc/OFL.txt", "OFL-Noto-Sans-SC.txt"),
                          ("departure/DepartureMono-1.500/LICENSE", "OFL-Departure-Mono.txt")):
        if (FONT_CACHE / licence).is_file():
            text = (FONT_CACHE / licence).read_text(encoding="utf-8")
            (target / name).write_text("\n".join(line.rstrip() for line in text.splitlines())+"\n", encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("steps", nargs="*", choices=("art", "shots", "terminal", "dither", "fonts"))
    parser.add_argument("--gui-previews", type=Path, nargs=2, metavar=("ZH", "EN"))
    parser.add_argument("--android-preview", type=Path, nargs=2, metavar=("ZH", "EN"))
    parser.add_argument("--gui-commit")
    parser.add_argument("--terminal-previews", type=Path, nargs=2, metavar=("ZH", "EN"))
    parser.add_argument("--terminal-commit")
    parser.add_argument("--art-name", action="append", choices=(*ART_NAMES, *NIGHT_ART_NAMES))
    options = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    steps = {"art": lambda: illustrations(options.art_name), "shots": lambda: screenshots(options),
             "terminal": lambda: write_captures("terminal", terminal_records(options.terminal_previews, options.terminal_commit)),
             "dither": dither_tiles, "fonts": fonts}
    chosen = options.steps or ["art", "dither", "fonts"]
    for step in chosen:
        print(f"{step}:")
        steps[step]()


if __name__ == "__main__":
    main()

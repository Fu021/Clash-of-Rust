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
    if not names or "hero" in names:
        hero = Image.open(RAW / "hero.png").convert("RGB")
        webp(hero.resize((hero.width // 2, hero.height // 2), Image.LANCZOS), "hero-small.webp", 84)
        webp(hero, "hero.webp", 86)
        webp(hero.resize((hero.width * 2, hero.height * 2), Image.NEAREST), "hero@2x.webp", 76)
    for name in ("tools", "terminal", "station", "guide", "start", "manual", "versions"):
        if names and name not in names:
            continue
        banner = Image.open(RAW / f"{name}.png").convert("RGB")
        webp(banner, f"{name}.webp", 84)
        webp(banner.resize((banner.width // 2, banner.height // 2), Image.LANCZOS), f"{name}-small.webp", 84)


def screenshots(options):
    if not options.gui_previews or not options.android_preview or not options.terminal_previews or not options.terminal_commit:
        raise ValueError("shots requires current GUI, Android and actual PTY preview paths and the terminal source commit")
    version = tomllib.loads((REPO / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip()
    sources = {
        "home": options.gui_previews / "home-light.png",
        "nodes": options.gui_previews / "proxies-light.png",
        "subscriptions": options.gui_previews / "subscriptions-light.png",
        "rules": options.gui_previews / "rules-light.png",
        "network": options.gui_previews / "network-light.png",
        "protection": options.gui_previews / "ai-check-light.png",
        "applications": options.android_preview / "android/settings-light.png",
    }
    records = {}
    for name, source in sources.items():
        image = Image.open(source)
        if image.format != "PNG":
            raise ValueError(f"Not a production PNG preview: {source}")
        target = OUT / f"app-{name}.png"
        shutil.copyfile(source, target)
        records[name] = {"file": target.name, "width": image.width, "height": image.height,
                         "platform": "android" if name == "applications" else "desktop",
                         "version": version, "commit": commit, "theme": "light",
                         "source": "production-slint-preview"}
    terminal_screenshots(options.terminal_previews, options.terminal_commit)
    for obsolete in ("app-home-light.webp", "app-home-dark.webp", "app-network.webp"):
        (OUT / obsolete).unlink(missing_ok=True)
    shutil.copyfile(REPO / "resources/icons/app.png", OUT / "cat.png")
    (SRC / "content/screenshots.json").write_text(json.dumps(records, ensure_ascii=False, indent=2) + "\n")
    print(f"  {len(records)} current GUI previews")


def terminal_screenshots(previews, commit):
    import re
    if not previews or not commit or not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("terminal requires actual PTY previews and their full source commit")
    version = tomllib.loads((REPO / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    terminal = {}
    for name in ("home", "network"):
        record = json.loads((previews / f"background-tui-{name}.json").read_text())
        if record.get("source") != "actual-cor-pty" or not any(version in line["text"] for line in record["lines"]):
            raise ValueError(f"The {name} PTY capture is not from {version}")
        terminal[name] = {**record, "version": version, "commit": commit}
    (SRC / "content/terminal.json").write_text(json.dumps(terminal, ensure_ascii=False, indent=2) + "\n")
    print(f"  {len(terminal)} actual PTY frames")


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
    parser.add_argument("--gui-previews", type=Path)
    parser.add_argument("--android-preview", type=Path)
    parser.add_argument("--terminal-previews", type=Path)
    parser.add_argument("--terminal-commit")
    parser.add_argument("--art-name", action="append", choices=("hero", "tools", "terminal", "station", "guide", "start", "manual", "versions"))
    options = parser.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    steps = {"art": lambda: illustrations(options.art_name), "shots": lambda: screenshots(options),
             "terminal": lambda: terminal_screenshots(options.terminal_previews, options.terminal_commit),
             "dither": dither_tiles, "fonts": fonts}
    chosen = options.steps or ["art", "dither", "fonts"]
    for step in chosen:
        print(f"{step}:")
        steps[step]()


if __name__ == "__main__":
    main()

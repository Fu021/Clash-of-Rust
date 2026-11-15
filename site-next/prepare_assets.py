"""Prepare committed site assets from sources: illustrations, screenshots and fonts.

Needs Pillow and fontTools (with brotli). Run it after changing an illustration,
or after changing Chinese headings, so the subset font covers the new characters:

    python site-next/prepare_assets.py

Font sources are read from site-next/.cache/fonts (not committed); see README.md.
"""
from html.parser import HTMLParser
from pathlib import Path
import shutil
import sys

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


def illustrations():
    hero = Image.open(RAW / "hero.png").convert("RGB")
    webp(hero, "hero.webp", 86)
    webp(hero.resize((hero.width * 2, hero.height * 2), Image.NEAREST), "hero@2x.webp", 76)
    # A tighter crop for phones: the signal cabin, the cat and the three lamps.
    width, height = hero.size
    webp(hero.crop((int(width * 0.36), int(height * 0.10), width, height)), "hero-narrow.webp", 86)
    for name in ("tools", "terminal", "station", "guide"):
        banner = Image.open(RAW / f"{name}.png").convert("RGB")
        webp(banner, f"{name}.webp", 84)
        webp(banner.resize((banner.width // 2, banner.height // 2), Image.LANCZOS), f"{name}-small.webp", 84)


def screenshots():
    sources = {
        "app-home-light.webp": REPO / "docs/images/home-0.7.0-light.png",
        "app-home-dark.webp": REPO / "docs/images/home-0.7.0-dark.png",
        "app-network.webp": REPO / "website/static/screenshots/network.png",
    }
    for name, source in sources.items():
        webp(Image.open(source).convert("RGB"), name, lossless=True)
    shutil.copyfile(REPO / "resources/icons/app.png", OUT / "cat.png")


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
    OUT.mkdir(parents=True, exist_ok=True)
    steps = {"art": illustrations, "shots": screenshots, "dither": dither_tiles, "fonts": fonts}
    chosen = sys.argv[1:] or list(steps)
    for step in chosen:
        print(f"{step}:")
        steps[step]()


if __name__ == "__main__":
    main()

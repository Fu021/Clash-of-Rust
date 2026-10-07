"""Prepare pinned Twemoji flags and CLDR English territory names (development only)."""
import concurrent.futures
import argparse
import io
import json
import re
import urllib.request
from pathlib import Path
from PIL import Image

root = Path(__file__).resolve().parent.parent
dest = root / "resources/flags"
dest.mkdir(parents=True, exist_ok=True)
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--proxy", default="")
options = parser.parse_args()
opener = urllib.request.build_opener(urllib.request.ProxyHandler(
    {"http": options.proxy, "https": options.proxy} if options.proxy else {}))

def fetch(url):
    return opener.open(url, timeout=30).read()

twemoji = "https://raw.githubusercontent.com/twitter/twemoji/v14.0.2/"
codes = re.search(r'ISO_CODES: &str = "([^"]+)"', (root / "src/probe.rs").read_text(encoding="utf-8"))[1].split()

def flag(code):
    name = "-".join(f"{0x1f1e6 + ord(ch) - ord('A'):x}" for ch in code)
    data = fetch(twemoji + f"assets/72x72/{name}.png")
    image = Image.open(io.BytesIO(data)).convert("RGBA")
    assert image.size == (72, 72)
    return code, image.tobytes()

with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
    flags = sorted(pool.map(flag, [code for code in codes if code not in ("HK", "MO", "TW")]))
(dest / "flags.rgba").write_bytes(b"".join(pixels for _, pixels in flags))
(dest / "codes.json").write_text(json.dumps([code for code, _ in flags]), encoding="utf-8")
(dest / "LICENSE-Twemoji.txt").write_bytes(fetch(twemoji + "LICENSE-GRAPHICS"))
cldr = "https://raw.githubusercontent.com/unicode-org/cldr-json/46.0.0/"
territories = json.loads(fetch(cldr + "cldr-json/cldr-localenames-full/main/en/territories.json"))["main"]["en"]["localeDisplayNames"]["territories"]
(dest / "countries.json").write_text(json.dumps({code: territories[code] for code in codes}, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
(dest / "LICENSE-Unicode.txt").write_bytes(fetch(cldr + "LICENSE"))
print(f"Prepared {len(flags)} flags; no network access is needed by the application")

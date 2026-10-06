"""Encode approved PNG artwork as Windows ICO and embedded RGBA assets.

Optional asset-maintenance script: requires Pillow. Normal Rust builds use the
committed outputs and do not need Python. No artwork or colors are edited here.
"""
from pathlib import Path
from PIL import Image

root = Path(__file__).resolve().parent.parent
output = root / "resources" / "icons"
output.mkdir(exist_ok=True)
for name, source in {
    "app": "app-blue-transparent.png",
    "system": "system-orange-transparent.png",
    "tun": "tun-green-transparent.png",
}.items():
    with Image.open(root / "resources" / "icon-concepts" / source) as image:
        image = image.convert("RGBA")
        assert image.getpixel((0, 0))[3] == 0, f"{source} must have a transparent background"
        thumbnail = image.resize((128, 128), Image.Resampling.LANCZOS)
        (output / f"{name}.rgba").write_bytes(thumbnail.tobytes())
        thumbnail.save(output / f"{name}.png")
        if name == "app":
            image.save(output / "app.ico", sizes=[(n, n) for n in (16, 24, 32, 48, 64, 128, 256)])

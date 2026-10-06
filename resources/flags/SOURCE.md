# Offline flags

Flag artwork: Twemoji v14.0.2, Copyright Twitter, Inc and other contributors,
licensed under CC-BY-4.0. Original assets:
https://github.com/twitter/twemoji/tree/v14.0.2/assets/72x72

The original PNG pixels are decoded to a concatenated RGBA file without changing
the artwork; only the requested flag is allocated by the GUI. Hong Kong, Macao,
and Taiwan have no flag in this bundle or in the UI.

English territory names: Unicode CLDR 46.0.0, Unicode License v3:
https://github.com/unicode-org/cldr-json/tree/46.0.0

`scripts/prepare-flags.py` reproduces these offline assets. Its development-only
dependency is Pillow. The application never downloads flags or territory names.
The full upstream licenses are included next to this file and in the installer.

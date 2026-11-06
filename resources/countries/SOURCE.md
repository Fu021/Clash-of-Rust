# Territory names

English territory names come from Unicode CLDR 46.0.0 under Unicode License v3:
https://github.com/unicode-org/cldr-json/tree/46.0.0

`countries.json` contains the ISO territory names used for country lookup.
The complete upstream license is included here and in application installers.
`python scripts/prepare-assets.py countries` reproduces this data.

Flag text is generated from country codes and rendered through the bundled
Noto Color Emoji font; no separate flag artwork or atlas is included.

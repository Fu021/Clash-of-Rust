# Embedded UI fonts

Source Han Sans CN Regular is Adobe's Simplified Chinese region-subset edition of 思源黑体.

- Source: https://github.com/adobe-fonts/source-han-sans
- Download: https://raw.githubusercontent.com/adobe-fonts/source-han-sans/release/SubsetOTF/CN/SourceHanSansCN-Regular.otf
- SHA-256: `e2bc8a2e7f37474b774fff8db758681ece40bb6947a90d571bce9dd60671a8e4`
- License: SIL Open Font License 1.1; see `LICENSE.txt`.

The font is embedded in the application. Segoe UI is loaded from Windows and is not redistributed.

## Noto Color Emoji

- Source: https://github.com/googlefonts/noto-emoji
- Revision: `1ffdd21391dd1f25c081fa93a9dea0c7c029442b`.
- Original file: `2D/fonts/NotoColorEmoji.ttf`, bundled unchanged as `NotoColorEmoji-Regular.ttf`.
- SHA-256: `15671215ab769fdc7162a045d56fd7d7e477c51b04e6b3c761d914d8fdd6cc44`.
- Format: CBDT/CBLC color bitmap font. The COLR v1/SVG edition is not bundled.
- License: SIL Open Font License 1.1; see `LICENSE-NotoEmoji.txt`.

Desktop and Android register this embedded font before generic system fallbacks
through `crates/client-ui/src/fonts.rs`. Android includes that helper directly
without linking the desktop component tree. Node names remain original text;
the font does not rewrite names or infer countries. IP and Claude reports
continue to use the separate offline flag atlas.

# Slint software renderer patch

This directory contains the published `i-slint-renderer-software` 1.18.1 crate,
SHA-256 `2f8c641e138a2cc96537cac4481f5c7c8bb8dcc66d09acaed269329c8738628a`.
Original sources and licenses are retained. The root `Cargo.lock` is authoritative.

The three modified files, `fonts.rs`, `fonts/vectorfont.rs`, and `lib.rs`, adapt
the color glyph changes from Slint PR #14567, merged on October 29, 2026:
https://github.com/slint-ui/slint/pull/14567
Merge commit: `e93604e4bbe9d82463e92ca048997b14ee415322`.

Color glyphs request COLR v0 layers, then CBDT/sbix bitmaps, before falling back
to outlines. Bitmap pixels are premultiplied before blending. Emoji retain
their colors and follow text opacity; COLR foreground layers use the text color.
The existing glyph cache remains bounded by bytes and distinguishes outline
and color requests. The older `GlyphRenderer::render_glyph` coverage path stays
monochrome. Unrelated upstream stroke and synthetic italic changes are excluded.

The application bundles a CBDT Noto Color Emoji font and explicitly registers
it before system generic fallback fonts. COLR v1 and SVG color fonts are not
enabled by this backport. Renderer regression tests live in
`crates/client-ui/tests/emoji.rs`; device validation is tracked in
`docs/agent-failures/ui-and-tests.md`.

Remove this override when a released Slint version includes equivalent color
glyph support, after verifying fallback, flags, joined emoji, opacity, clipping,
partial redraws, and the existing software-rendering regressions.

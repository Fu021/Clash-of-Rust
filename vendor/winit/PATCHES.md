# Winit X11 resize synchronization

This directory contains the published `winit` 0.30.13 crate, with its original
Apache-2.0 license and sources. The archive SHA-256 is
`a6755fa58a9f8350bd1e472d4c3fcc25f824ec358933bba33306d0b63df5978d`;
its source commit is `e9809ef54b18499bb4f2cac945719ecc2a61061b`.
The crate's standalone lockfile is omitted; the root `Cargo.lock` is authoritative.
The README contribution link points to that source commit because its target is
not included in the published crate.

The X11-only changes adapt the protocol introduced by upstream
[PR #3771](https://github.com/rust-windowing/winit/pull/3771), commit
`eef2848c981606552fa18ccf472928d34d7ff7df`, without changing the 0.30 API or
upgrading other platforms to the incompatible 0.31 API.

## Local behavior

- Enable the existing `x11rb` dependency's `sync` feature. No new package is added.
- Initialize SYNC once per X connection and advertise `_NET_WM_SYNC_REQUEST` only
  when SYNC initialization and counter creation succeeded. An absent or
  unsupported extension leaves the normal X11 path available. Counter or
  property setup errors also fall back without aborting window creation;
  partially created counters are released and the protocol is not advertised.
- Give each window its own counter, published with the `CARDINAL` property type.
  Destroy the counter when its window resources are released.
- Separate receipt of a request, its following `ConfigureNotify`, rendering, and
  acknowledgement. Coalesced resizes keep only the latest configured serial.
  An unrelated older frame must never acknowledge an unconfigured request.
- Unlike the original upstream patch's immediate `sync_set_counter` in the
  `ClientMessage` handler, wait for `pre_present_notify` and the return of that
  window's redraw callback. The application's Slint software backend renders
  synchronously and queues `softbuffer` image requests on the same underlying
  XCB connection; the counter update therefore follows those image requests.
  Do not acknowledge while exiting after a redraw failure.
- Deliver `Resized` for a synchronization request even when its dimensions match
  the previous dimensions, allowing the host to invalidate and submit a frame.
- Keep state bounded and ignore the protocol's invalid zero serial. Ordinary
  redraws and move events produce no extra acknowledgements.

The application additionally requests a native Linux redraw on resize after
marking the new extent dirty. This avoids waiting for Slint's X11 frame timer;
it adds no polling loop, frame-rate target, or layout animation.

The protocol is specified in
[EWMH, section 6.2](https://specifications.freedesktop.org/wm-spec/latest/ar01s06.html)
and the [SYNC protocol](https://www.x.org/releases/X11R7.7/doc/xextproto/sync.html).
Wayland and other platform implementations remain the published 0.30.13 source.

## Verification and removal

`apps/desktop-ui/src/repaint.rs` includes the actual pure state module in its
library tests, so normal workspace tests exercise configure/present ordering, coalescing,
late requests, repeated redraws, invalid serials and the Xlib 32-bit word ABI.
These tests do not prove compositor behavior. Changes in this override select
the existing desktop UI regression group; no workflow or permanent test matrix
is added.

The maintainer confirmed on 2026-11-13 that the reported Linux rapid-resize
symptom is fixed in the current version, recorded in the 0.7.1 release notes.
This does not establish a unique cause or cover every compositor and scale
combination. Before extending that conclusion to other environments, verify the built application in
GNOME X11, XWayland and native Wayland, with rapid edge/corner resizing, both
growth and shrink, fractional scaling, maximization and window recreation.
Capture actual presentation timing and frames; Xvfb screenshots alone are
insufficient. Local resource limits permit parsing and static checks only.
The desktop builds and eight synchronization state regressions passed in CI;
the full compositor matrix remains unverified. Current evidence is recorded in
[the review log](../../docs/agent-failures/reviews.md).

Remove this override when a compatible Slint/winit release provides the same
post-render synchronization, after passing the existing desktop lifecycle
checks and the compositor resize checks above.

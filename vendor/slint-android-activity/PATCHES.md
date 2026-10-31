# Slint Android backend patch

This is the published `i-slint-backend-android-activity` 1.18.1 crate,
SHA-256 `70d538003a566d7312572ac1435be0704934e46a63d55abdabede513f72332cb`.
Its original license files are included. The root Cargo.lock is authoritative.

`androidwindowadapter.rs` backports upstream Slint commit
`611357480eb7a7eb4f1957d912512fadf99a6666`: suspend the renderer while
the old Android window is still valid and request a redraw when a new window
is initialized. Source:
https://github.com/slint-ui/slint/commit/611357480eb7a7eb4f1957d912512fadf99a6666

Remove this override after updating Slint to a release containing that fix
and verifying Activity recreation and VPN continuity.

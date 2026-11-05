# Slint Android backend patch

This is the published `i-slint-backend-android-activity` 1.18.1 crate,
SHA-256 `70d538003a566d7312572ac1435be0704934e46a63d55abdabede513f72332cb`.
Its original license files are included. The root Cargo.lock is authoritative.

`androidwindowadapter.rs` backports upstream Slint commit
`611357480eb7a7eb4f1957d912512fadf99a6666`: suspend the renderer while
the old Android window is still valid and request a redraw when a new window
is initialized. Source:
https://github.com/slint-ui/slint/commit/611357480eb7a7eb4f1957d912512fadf99a6666

The Java helper also carries local IME compatibility changes:

- Remove the application `WindowInsetsAnimation.Callback`; API 31/32 screenshot
  comparisons found an active input connection and reported IME visibility
  without a rendered keyboard while this callback was installed.
- Coalesce field-switch show/hide requests on the input view's next frame and
  wait for window focus before showing. Synchronize the input-method focus on
  API 30–32 before using `WindowInsetsController`, following AndroidX's
  `SoftwareKeyboardControllerCompat`; retain the posted `showSoftInput` path.
  A pending request can resume when window focus arrives; an already delivered
  request does not reopen a keyboard dismissed with Back on later focus changes.
- On API 26–29, exclude stable system bars when choosing the keyboard edge.
  Navigation bars alone must report no keyboard, including landscape side bars;
  a real keyboard retains its full occlusion for the shared layout to subtract
  the bottom safe area once.

AndroidX reference:
https://github.com/androidx/androidx/blob/androidx-main/core/core/src/main/java/androidx/core/view/SoftwareKeyboardControllerCompat.java

These IME changes have local static checks only; the production keyboard,
layout, Back, rotation and lifecycle regressions still require Actions and
device verification. The upstream crate license files remain authoritative.

Remove this override after updating Slint to a release containing equivalent
window and IME fixes and verifying Activity recreation, VPN continuity and
multi-version keyboard behavior.

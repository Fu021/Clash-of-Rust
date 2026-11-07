# Slint Android backend patch

This is the published `i-slint-backend-android-activity` 1.18.1 crate,
SHA-256 `70d538003a566d7312572ac1435be0704934e46a63d55abdabede513f72332cb`.
Its original license files are included. The root Cargo.lock is authoritative.

`androidwindowadapter.rs` backports upstream Slint commit
`611357480eb7a7eb4f1957d912512fadf99a6666`: suspend the renderer while
the old Android window is still valid and request a redraw when a new window
is initialized. Source:
https://github.com/slint-ui/slint/commit/611357480eb7a7eb4f1957d912512fadf99a6666

Local lifecycle handling draws the first frame on `InitWindow`, then requires
both an initialized surface and window focus for regular frames and animation
wakeups. Pause, stop and focus loss prevent drawing to an outgoing surface.
The pinned Skia swap-buffer error `[300d]` suspends the invalid surface and waits
for `InitWindow`; unrelated rendering errors still propagate. Foreground input
and rotation tests must prove recovery, not just that the process survived.

The Java helper also carries local IME compatibility changes:

- Remove the application `WindowInsetsAnimation.Callback`; API 31/32 screenshot
  comparisons found an active input connection and reported IME visibility
  without a rendered keyboard while this callback was installed.
- Coalesce show requests on the input view's next frame and wait for window
  focus before showing. Delay a hide by 150 ms so the next field can cancel it
  before the input view loses focus or a system hide animation starts.
  Use posted `showSoftInput` on API 26–32. On API 33+, use
  `WindowInsetsController` when available and only fall back to `showSoftInput`
  when it is unavailable; do not request both for a single show operation.
  A pending request can resume when window focus arrives; an already delivered
  request does not reopen a keyboard dismissed with Back on later focus changes.
- On API 26–29, exclude stable system bars when choosing the keyboard edge.
  Navigation bars alone must report no keyboard, including landscape side bars;
  a real keyboard retains its full occlusion for the shared layout to subtract
  the bottom safe area once.

AndroidX reference:
https://github.com/androidx/androidx/blob/androidx-main/core/core/src/main/java/androidx/core/view/SoftwareKeyboardControllerCompat.java

Current Actions evidence and remaining failures are recorded in
[the IME review](../../docs/agent-failures/android-ime.md).
The same-commit CI on `3dd1f87` passed the production keyboard, layout, Back,
rotation and lifecycle regressions on API 31 and 36. The earlier Gboard
null-pointer crash did not recur, so the native cold-start controls did not run
and its cause remains unconfirmed. Physical-device and OEM verification remain
pending; historical multi-version results do not prove all-device compatibility.
The upstream crate license files remain authoritative.

Remove this override after updating Slint to a release containing equivalent
window and IME fixes and verifying Activity recreation, VPN continuity and
multi-version keyboard behavior.

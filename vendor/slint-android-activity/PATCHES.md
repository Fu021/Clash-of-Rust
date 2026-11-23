# Slint Android backend patch

This is the published `i-slint-backend-android-activity` 1.19.0 crate,
SHA-256 `1c1abcf6132800250b5c70198f5949a4fc42bf5e13f7e54cb6d34aa0d281522f`.
Its original license files are included. The root Cargo.lock is authoritative.

The release already includes upstream Slint commit
`611357480eb7a7eb4f1957d912512fadf99a6666`: suspend the renderer while
the old Android window is still valid and request a redraw when a new window
is initialized. This is no longer a local backport. Source:
https://github.com/slint-ui/slint/commit/611357480eb7a7eb4f1957d912512fadf99a6666

Local lifecycle handling draws the first frame on `InitWindow`, then requires
an initialized surface, a resumed Activity and window focus for regular frames
and animation wakeups. Android can retain the focused decor view across an
Activity relaunch. Read its current focus on `Resume` and `InitWindow` so
rendering can resume before a new focus transition. Pause, stop and focus loss
still prevent drawing to an outgoing surface. Android's window preservation is
described in `ViewRootImpl.notifyChildRebuilt`:
https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android16-release/core/java/android/view/ViewRootImpl.java

The pinned Skia swap-buffer error `[300d]` suspends the invalid surface and waits
for `InitWindow`; unrelated rendering errors still propagate. Foreground input
and rotation tests must prove recovery, not just that the process survived.

Raw Android Back keys are returned as unhandled so the Activity's committed
Back callback owns navigation. Older Android versions can continue dispatching
the key to `Activity.onBackPressed()`. Android 16 can forward the raw key after
invoking the registered callback, so processing both deliveries could dismiss
an error and then background the app. See
[Android's input dispatch](https://android.googlesource.com/platform/frameworks/base/+/refs/heads/android16-release/core/java/android/view/ViewRootImpl.java)
and the [Back callback contract](https://developer.android.com/reference/android/window/OnBackInvokedCallback).

Display density is read from the current Activity's `Resources` at window
creation, configuration changes and resizes, with NDK configuration as a
fallback for unavailable or invalid Activity metrics. Preview run `42685691467`
observed the native window resize to 800×1280 while Slint retained a scale of
2.75 on Android 8.0 and 9–16; this prevented the expected tablet layout. Density,
logical size and safe-area conversion must update together. This local repair
still requires same-commit Actions validation. Android documents that the
Activity's resources reflect the new configuration when its callback runs:
https://developer.android.com/reference/android/app/NativeActivity#onConfigurationChanged(android.content.res.Configuration)

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
the bottom safe area once. In 1.19.0 this remains a local change in
`dispatchLayoutInsets`; upstream still compares the raw system-bar occlusion.

The 1.19.0 rebase retains the published Java package, JNI class name, input
timestamps, text-selection behavior, clipboard handling, system-bar contrast,
reduced-motion preference and low-memory GPU cleanup. It adapts the local
surface recovery to `render_transformed` and keyboard cleanup to `hideCursor`;
the older `render_transformed_with_post_callback` entry no longer exists.

AndroidX reference:
https://github.com/androidx/androidx/blob/androidx-main/core/core/src/main/java/androidx/core/view/SoftwareKeyboardControllerCompat.java

Current Actions evidence and remaining failures are recorded in
[the IME review](../../docs/agent-failures/android-ime.md).
The historical 1.18.1 CI on `3dd1f87` passed the production keyboard, layout, Back,
rotation and lifecycle regressions on API 31 and 36. The earlier Gboard
null-pointer crash did not recur, so the native cold-start controls did not run
and its cause remains unconfirmed. Physical-device and OEM verification remain
pending; historical multi-version results do not validate the 1.19.0 rebase
or prove all-device compatibility.
The upstream crate license files remain authoritative.

Remove this override after updating Slint to a release containing equivalent
remaining surface, density and IME fixes and verifying Activity recreation, VPN continuity and
multi-version keyboard behavior.

# SurfaceView startup experiment (M9)

This is an isolated candidate on top of the strict-evidence follow-up. It is not
a release or a completed startup qualification. All related PRs remain unmerged.

## Rationale and boundaries

[Media3 surface guidance](https://developer.android.com/media/media3/ui/surface)
recommends SurfaceView where possible for lower power and more accurate frame
timing. The existing video panel avoids animation/graphics-layer transitions;
this candidate switches its native player XML from TextureView to SurfaceView,
including fullscreen. Compose-hosted views enable Media3's surface-sync workaround.
Native extraction, quality, media cache, account/network policies and buffering
thresholds stay unchanged. The existing conservative two-frame visibility guard
also stays unchanged for the first comparison.

Idle preparation still creates a detached, unbound view. Its lifecycle test now
checks the appropriate surface type: a SurfaceView holder must have no valid
surface until attachment, just as an idle TextureView must not be available.

## Compositor evidence

Window PixelCopy does not establish the contents of a separate child video
surface plus its overlays. SurfaceView qualification therefore uses a composed
window screenshot (UiAutomation window capture on API34+, display capture on
older devices), cropped to the actual player bounds. Capture runs on a bounded
worker after genuine output/playing has been recorded, avoiding repeated reads
during decoder setup. Request and completed evidence clocks remain separate;
confirmation includes completed image availability and the original deadlines.
TextureView keeps its existing Window PixelCopy observer. Reports identify the
evidence method, so changes in observer cost are not claimed as app speedups.

A new functional test requires actual decoded video in the compositor capture,
adds an opaque window cover and requires the capture to reject it, then removes
the cover and verifies the same frame returns. It pauses a genuine decoded frame
for this occlusion check; it is not a latency sample. Raw-surface readback cannot
pass by bypassing the cover. Clear/covered/restored screenshots are retained.

## Verification

Local app and test APK assembly, 37 JVM tests and lint pass. The first added test
compile required the Compose experimental-test opt-in; it was fixed before the
successful build. Local packaging uses retained JNI, so CI must rebuild both ABIs
and test the exact candidate. Native compositor/occlusion, fullscreen return,
controls/contrast, gesture, rapid-source and one-/two-second regressions plus
physical Tab S9 YouTube acceptance remain pending. A regression or slower result
must remain a failure, without changing clocks, quality or deadline thresholds.

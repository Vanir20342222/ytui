# ytui Android Glow teardown adapter

Upstream eframe 0.33.3, git `44cdd653e2317d300fb8a6c9c36b03f23991e803`,
`crates/eframe`, MIT OR Apache-2.0. Original source and both licenses are retained.

Android Suspended drops the window and makes the EGL context non-current.
Upstream's ordinary save handles an optional window, but save_and_destroy still
unconditionally unwraps it. Actual API 35 logs reproduced `winit window doesn't
exist` on Activity destruction even though Android caught the Rust panic and the
next Activity opened. That skipped the application's on_exit cleanup.

The patch saves with an optional window, makes Android EGL current without a
surface for GL deletion where supported, and always invokes the application's
exit hook. If restoring a context fails, it calls on_exit(None) and lets context
destruction reclaim GPU objects rather than issuing GL commands without one.
Other platforms keep their normal current-context cleanup path.

Acceptance must verify clean exit-hook completion, same-process UI reopening and
the absence of Rust panic records, in addition to main-loop responsiveness. Rebase
or remove this patch when upstream supplies equivalent Android cleanup.

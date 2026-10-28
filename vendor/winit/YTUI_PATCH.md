# ytui Android Activity teardown adapter

Upstream: winit 0.30.13, git `e9809ef54b18499bb4f2cac945719ecc2a61061b`,
under the included Apache-2.0 license. Original source and examples are retained.

Only Android lifecycle handling changes: `MainEvent::Destroy` requests event-loop
exit and permits a later Activity's event loop to be created. Upstream's Android
handler only logged a TODO. GameActivity's `terminateNativeCode` then remained
blocked, which also prevented JavaScriptSandbox's main-thread ServiceConnection
callback from completing. The CI trace reproduced that stack.

The application keeps `run_and_return=true`: eframe's alternative calls
`process::exit`, which would kill the independent background playback service.
The Activity uses `singleTask` to route shares/notifications to the existing Rust
window, so recreation is sequential. Other platforms retain the original event
loop creation policy. Remove/rebase this patch when upstream supplies equivalent
Android Activity destruction and recreation support.

Acceptance must include closing and reopening the Rust Activity while the process
and engine stay alive, with the Android main looper responsive and offline solver
connections still completing. A successful cross-compile does not qualify it.

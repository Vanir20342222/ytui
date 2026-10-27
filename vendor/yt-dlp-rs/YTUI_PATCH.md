# ytui Android runtime adapter

Upstream: https://github.com/aagedal/yt-dlp-rs at
`f67d96bdf98c5b821d0c9bf6175d328a0fa8ceab`, under its Unlicense.
Production source, EJS licenses and upstream tests are retained.

The only functional change adds `set_embedded_js_evaluator` and the explicit
`ytui-android-js` runtime sentinel. An Android caller runs the original bounded
EJS batch in AndroidX JavaScriptSandbox, without network, cookie or host APIs.
The original batch validation still enforces response identity and size limits.
Desktop Node/Deno execution and explicit-runtime validation stay unchanged.

The callback must impose a deadline and isolate teardown. Android's bridge uses
15-second deadlines and a 128 MiB heap limit. This hook is not runtime discovery
and does not install any executable. Rebase/remove the patch when upstream
accepts an equivalent embeddable runtime API.

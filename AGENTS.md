# Working on ytui

Read [RUST_REWRITE_PLAN.md](RUST_REWRITE_PLAN.md) before planning or implementing
the Rust rewrite. It contains confirmed user requirements, provisional choices,
baseline defects, milestones, migration requirements, and acceptance checks.

- The initial planning change is documentation only. Do not claim the Rust rewrite
  or any planned feature is implemented until there is working code and evidence.
- Current target: Omarchy + Ghostty + Brave Origin, per docs/LOCAL_ACCEPTANCE_2026-10-26.md.
  CachyOS + Caelestia is the historical target. The user confirmed
  a terminal app with mpv playback, short muted previews on `v`, native radio with
  optional cliamp launch, and permits yt-dlp/FFmpeg as external tools.
- Reference milestone IDs in changes and update completion evidence in the plan.
  Coordinate shared model/service contracts before independent feature work.
- The user explicitly authorized removing the Python app on 2026-10-21 after
  documenting gaps. Rust is now the repository application. Keep migration and
  existing user settings, queues, history and downloaded files intact. Remaining
  live-service/desktop release gates still apply; see docs/REMAINING_WORK.md.
- Keep tests deterministic and offline by default. Never read a developer's real
  browser cookies or alter VPN/system theme settings as a test side effect.
- Never commit cookies, authorization headers, private feed fixtures, or signed
  stream URLs. Use synthetic sessions and sanitized diagnostics.
- Report checks actually run, environmental blockers, and remaining limitations.
  An empty feature tab or a successful compile does not satisfy its milestone.

- Current backend decision (user confirmed this follow-up): recommend `auto`,
  trying pinned aagedal/yt-dlp-rs first and external yt-dlp for supported extractor
  limitations. Keep explicit `native` and `yt-dlp` modes. Never download/install
  executables or change an existing explicit backend without a user action.
- The later native-playback objective supersedes the shared playback policy:
  `playback_backend` defaults to `native` and never automatically starts Python
  yt-dlp. Keep download backend settings separate and preserve an explicit
  playback override. Node.js 22+ / pinned Deno 2.9.7 run the native extractor's
  offline JS challenges; detect installed tools, never auto-install them.
- A 99% watching-success claim requires representative live evidence. A local
  fixture, startup smoke test or successful fallback cannot establish that claim.
- Implementation evidence and open gates are in docs/VALIDATION.md and
  docs/PARITY.md. Do not equate implemented provider code with a successful live
  account, Ghostty, audio-device or stable-release acceptance result.

- M9 adds the Android preview and monorepo. Desktop source is `apps/desktop`;
  package/binary names and installed state paths must remain compatible.
  Shared network/provider code is `crates/ytui-providers`; Android's engine
  remains Rust in `crates/ytui-mobile`. On 2026-10-28 the user authorized a
  stack change after reporting unusable text input/themes/account navigation on
  a physical Galaxy Tab S9. Android's current UI target is native Jetpack Compose
  under `apps/android`, with the shared Rust engine. Later on 2026-10-28 the user
  explicitly rejected terminal styling on mobile and requested a polished
  YouTube/Netflix-inspired design: sans-serif typography, artwork-led browsing,
  native navigation and touch-friendly settings. Keep all shared palettes; the
  desktop terminal presentation is unaffected.
  Java/Kotlin also handle activity/media/storage/OS adapters and explicit account
  WebView sign-in. Do not replace application screens with a web UI.
- Read `docs/android/{PLAN,PARITY,VALIDATION}.md` for mobile gates. Unit/fixture,
  APK build, emulator and physical-device/live acceptance are distinct. Do not
  describe development APKs as production releases or claim feature parity while
  listed mobile gaps remain. Keep Android signing keys and SDK/build outputs out
  of Git. Instrumentation mutates only an explicitly selected disposable app.
  Earlier egui headless/render checks did not qualify real Android keyboard,
  theme or account flows. Preserve their backend evidence, and require actual
  native text-entry/theme/account-navigation tests for the replacement UI.

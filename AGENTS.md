# Working on ytui

Read [RUST_REWRITE_PLAN.md](RUST_REWRITE_PLAN.md) before planning or implementing
the Rust rewrite. It contains confirmed user requirements, provisional choices,
baseline defects, milestones, migration requirements, and acceptance checks.

- The initial planning change is documentation only. Do not claim the Rust rewrite
  or any planned feature is implemented until there is working code and evidence.
- Target CachyOS + Caelestia + Ghostty with Brave Origin first. The user confirmed
  a terminal app with mpv playback, short muted previews on `v`, native radio with
  optional cliamp launch, and permits yt-dlp/FFmpeg as external tools.
- Reference milestone IDs in changes and update completion evidence in the plan.
  Coordinate shared model/service contracts before independent feature work.
- Keep the Python app working until the Rust replacement passes the release and
  migration gates. Do not discard existing user settings, queues, or history.
- Keep tests deterministic and offline by default. Never read a developer's real
  browser cookies or alter VPN/system theme settings as a test side effect.
- Never commit cookies, authorization headers, private feed fixtures, or signed
  stream URLs. Use synthetic sessions and sanitized diagnostics.
- Report checks actually run, environmental blockers, and remaining limitations.
  An empty feature tab or a successful compile does not satisfy its milestone.

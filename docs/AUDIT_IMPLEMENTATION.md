# Audit implementation — 23 October 2026 (M2–M8)

Baseline: alpha.11 `e2b01ea`; requirements and original repros are in
[the app audit](APP_AUDIT_2026-10-22.md). Work is on `fix/audit-implementation`.
This record distinguishes implementation from actual desktop/account acceptance.

| Work | Implementation / acceptance |
| --- | --- |
| B01 | Queue entries capture video/audio intent at selection. Tab changes do not change queued playback. Failed command submission preserves the current item; stale EOF cannot advance another item. |
| B02 | Protocol-keyed encoded cards, invalidation generations and retained Auto capability. Actual Ghostty protocol switches still need a visual check. |
| B03 | Separate in-flight/error states with bounded transient retries (2/5/15/30/60 seconds), including detail images. Permanent errors wait for explicit retry. |
| B04 | A valid decoded thumbnail survives optional disk-write/pruning failures. |
| B05 | Existing completed media from partial/error collections appears offline. Downloads Enter and Actions → Files provide a selectable output panel. Collection failure is preserved. |
| B06 | Cancellable preparation, operation IDs, serialized blocking commits, busy draft retention and deferred Disconnect. Synthetic concurrent-store/restart and committed-settings/Disconnect tests pass. Live keyring/browser acceptance remains open. |
| B07 | Shared codec/bitrate/VBR arguments, metadata/template/height handling, private staging and cleanup. Unsupported local thumbnail/caption/SponsorBlock/chapter options fail explicitly. Actual 320K regression and existing real-media checks pass. |
| B08 | Duplicate identity compares requested output settings, format, quality and destination; transport/backend policy is excluded. |
| B09/F12 | Pending: main promotion and versioned release/package acceptance. No release gate is silently waived. |
| R01/F08 | Enabling SponsorBlock after `off` fetches marks. Config changes discard stale categories. An actual loopback API regression passes. Human API/window acceptance remains open. |
| R02 | Current feed window: 2,000 items / 8 MiB estimated media data; back history: eight pages / 12 MiB per tab; continuation-token memory bounded. Forward pagination continues. Prefetch uses card dimensions. Long synthetic feed regression passes. |
| R03 | Pending: qualify explicit HTTP/SOCKS routing across all backends. |
| F01 | Existing explicit import/validation retained; profile chooser and full signed-in journey acceptance pending. |
| F02 | Output picker implemented; full persistent per-entry collection expansion/retry pending. |
| F03 | Full formats/languages/captions dialog pending; existing mpv controls remain. |
| F04 | Default suite and real local media checked; representative multi-video/long-session target-network campaign pending. No 99% claim. |
| F05/F10 | Setup/capability panel and CLI `support [--output NEW.json]`; allowlisted version/revision/backends/tool versions/failure-phase export. No config/session/URL/path/raw-error serialization. No tool auto-installation. Further preflight guidance pending. |
| F06 | Cache inspection/clear UI, bounded recovery and memory limits. Clear/prune touch only owned hashed regular cache files; symlinks/foreign files are preserved. |
| F07 | Existing native live radio/favorites/reconnect/volume retained; actual long station and audio-device acceptance pending. |
| F09 | MPRIS implementation/target desktop acceptance pending. |
| F11 | Native collection guard stays until native multi-entry/resume qualification. Explicit per-video group workflow pending. |
| F13 | Lyrics/localization/destination convenience pending after core fixes. |

## Evidence so far

- `cargo test --workspace --locked -j 2`: **177 passed, 23 opt-in ignored**,
  including the real PTY, settings/theme matrix, account ordering, partial files,
  bounded feeds, cache safety and SponsorBlock API regression.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo fmt --all -- --check`: passed.
- Actual local media campaign: nine checks passed, including the new FFmpeg
  320K conversion case and eight existing mpv/FFmpeg/yt-dlp checks. No public
  YouTube service, account or physical audio output is inferred from these tests.
- The archived red baseline harness stays in `docs/audit`. The regular green
  target is `crates/ytui/tests/audit_regressions.rs`. `tools/test-app-audit.sh`
  executes it directly; `--real` also includes its actual FFmpeg check.

No real browser credentials are read by tests, no VPN/desktop preferences are
changed, and no 99% reliability claim follows from fixture or startup checks.

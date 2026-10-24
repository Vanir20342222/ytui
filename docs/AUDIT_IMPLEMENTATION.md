# Alpha.12 audit implementation — 23 October 2026 (M2–M8)

Baseline: alpha.11 `e2b01ea`; original requirements/repros are in
[the app audit](APP_AUDIT_2026-10-22.md). Implementation is on
[PR #13](https://github.com/Vanir20342222/ytui/pull/13) merged into main.
The user subsequently authorized reviewing and merging all PRs; the
[integration review](PR_REVIEW_2026-10-24.md) records the audit PR merge, delivery
of the later networking commit, additional fixes and final checks.

The [networking follow-up](NETWORK_RELIABILITY_2026-10-23.md) records additional
rate-limit, radio redirect/deadline, cover-proxy and stale-request fixes, with
217 deterministic passes and public service results (5 pass, 2 tunnel failures).
The second pass adds background-watch cooldown/cancellation, radio mirror/cache
isolation, shared discovery and redirect downgrade fixes.

| Work | Implementation / acceptance |
| --- | --- |
| B01 | Queue entries capture audio/video intent at selection. Tabs do not change playback type. Busy commands preserve selection; deferred automatic advance retries and Stop cancels it; stale EOF for another URL is ignored. |
| B02 | Protocol-keyed encoded cards, invalidation generations, detected Auto capability, stale-result rejection. Actual Ghostty protocol switching still requires visual acceptance. |
| B03 | Separate in-flight/error states and bounded 2/5/15/30/60-second transient retries, including wrapped transport errors/detail images. Permanent errors show an unavailable state instead of retrying every frame. |
| B04 | Decoded thumbnails survive optional disk-write/pruning failures. Cache inspection reports storage failures separately. |
| B05 | Existing completed media from partial/error collections appears offline. Downloads Enter / Actions → Files provides a selectable output panel without changing collection failure. Missing files/sidecars stay excluded from playback. |
| B06 | Cancellable preparation, operation IDs, serialized blocking commits, busy draft retention and deferred Disconnect. Synthetic concurrent-store/restart, repeated Disconnect and committed-settings/Disconnect regressions pass. File-only logout does not enlist the keyring; a durable logout marker blocks automatic restoration after failed cleanup. Player account handoff waits for channel capacity before allowing new playback. Live keyring/browser acceptance remains open. |
| B07 | Shared codecs/bitrate/VBR, metadata/template/height options, private staging and cleanup. Unsupported local thumbnail/caption/SponsorBlock/chapter options fail explicitly. Actual 320K regression and existing real-media checks pass. |
| B08 | Duplicate identity compares requested output format, quality, selector, destination and postprocessing; transport/backend policy is excluded. |
| B09/F12 | Alpha.12 source, commit-aware launcher versions, candidate main PR, checksum/license archive tooling, exact-SHA Arch recipe, executable backup/rollback and manual candidate workflow. Main merge, accepted release tag, Arch installation and real copied-data rollback are still gates. |
| R01/F08 | Off → B fetches marks; category changes discard stale marks. Loopback API regression, overlaps/replay and real mpv controls pass. Status panel explains mark/skip/off, API recovery and the limits of ad handling. |
| R02 | Forward pagination continues with a 2,000-item / 8-MiB estimated media window. Back history: eight pages / 12 MiB per tab; continuation-token memory bounded. Prefetch uses actual card dimensions. Long synthetic feed passes. Older released items require navigating back/reloading their feed. |
| R03 | Explicit HTTP and SOCKS5/5h clients tested with an unresolvable `.invalid` destination and loopback proxies; SOCKS uses remote DNS consistently. Actual mpv HTTP proxy playback passes. mpv SOCKS is explicitly rejected before launch, with HTTP/VPN guidance, rather than silently connecting directly. HTTPS proxy auth/tunnels and actual Proton network remain target checks. |
| F01 | Profile chooser discovers directory names only, then opens an editable review form before any import. Consent/verification/keyring/session guidance is visible. Full signed-in Home/Abos/Library/Music/brand-channel journeys require the user's explicit local session and remain unverified here. |
| F02 | Explicit whole-collection expansion through the Rust browser, bounded to 32 pages / 2,000 available entries / 16 MiB. Nothing queues after an incomplete expansion. Each video becomes a persistent individual job with collection title/source/index; retries reuse ready files and retry missing failed entries. Group/history panel has per-entry retry/file access. Restart regression passes. Shared items already belonging to other groups are reported instead of duplicated. |
| F03 | Actual extractor format IDs/heights/codecs/audio languages and VTT/SRT caption sources, plus loaded mpv audio/subtitle tracks. Selection uses real format selectors, retains position/pause on restart, and switches loaded tracks in place. Actual native selector and real mpv paused-track regressions pass. Full availability of every YouTube alternative is not claimed. |
| F04 | Multi-URL/quality/duration matrix runner records build, denominator, elapsed time, decoded height and codecs. CLI accepts up to one hour per case. Existing seek/pause/preview/fault sweep remains. Real broad/long target-network campaign and a population reliability claim remain open. |
| F05/F10 | Setup/capability panel, per-field option guidance, build revision including modified-source marker, failure phases, CLI/UI support export. Export is an allowlist: no config/session/URL/path/raw-error dump. No automatic tool installation. |
| F06 | Cache inspect/clear UI, bounded image/feed recovery, distinct retry/unavailable states and image-size-aware prefetch. Clear/prune touch hashed regular cache files only; foreign files/symlinks are preserved. |
| F07 | Native live stations remain separate from track libraries; ICY/title metadata is surfaced. Existing favorites, reconnect budget, volume persistence and optional cliamp launch are retained. Long station-switch/reconnect and physical audio acceptance remain open. |
| F09 | MPRIS uses the existing player/queue: Play/Pause/Stop, Next/Previous, seek, absolute volume/rate, metadata and change signals. Queue capabilities are published; private-session D-Bus test passes. Caelestia media-widget/physical key/Wayland acceptance remains open. |
| F11 | Rust browser expansion supplies independent native per-video jobs without lifting the extractor's collection guard. Native collection CLI/worker stays guarded until actual multi-entry/range/cancel/restart/partial-failure YouTube qualification. |
| F13 | Recent destinations and explicit portal Open with, reviewed opt-in LRCLIB lookup, offline UTF-8 LRC/text, timed-line following and candidate selection implemented. Initial German navigation/settings/actions/help/lyrics labels plus en/de/system preference and bilingual action search. Raw provider/tool text and some guidance/status prose remain English; full sentence-level localization is not claimed. |

## Executed evidence

- Latest complete offline checkpoint: **204 passed, 25 opt-in ignored**.
  This includes real PTY, all themes/layouts/settings sections, new panels,
  account ordering, partial files, collection restart, formats, queue pressure,
  bounded feeds, cache safety, SponsorBlock API and lyrics/localization checks.
  Archive/checkpoint evidence is distinguished below by exact source revision.
- Strict Clippy, Rust formatting, shell syntax and workflow actionlint pass at
  their recorded checkpoints; newly combined delivery changes are rechecked.
- Nine existing actual local media checks pass; the new actual mpv HTTP proxy
  check also passes. This includes FFmpeg 320K conversion and loaded audio-track
  switching while paused without losing position. Private-session MPRIS passes
  separately. Local tools: mpv 0.40, FFmpeg/ffprobe 7.1.5, yt-dlp 2026.10.21.
- Public alpha.12 native matrix in this cloud: **0/3 passed** at 360p / 3 seconds
  (two video URLs and one Music audio URL). All three fail at the inherited proxy
  tunnel before YouTube extraction. No bypass, cookies or external fallback were
  used. These are failures, not successful skips or evidence of a desktop bug.
- Earlier checkpoint `3d35e0f`: both GitHub Rust checks passed:
  [39921142158](https://github.com/Vanir20342222/ytui/actions/runs/39921142158),
  [39921186366](https://github.com/Vanir20342222/ytui/actions/runs/39921186366).
  Later commits need their own CI evidence.
- Archived red repros stay in `docs/audit`. Regular green tests are in
  `crates/ytui/tests/audit_regressions.rs`; `tools/test-app-audit.sh --real` adds
  the actual FFmpeg case without copying/deleting test source files.

No test imports real browser credentials, changes VPN/desktop preferences, or
modifies user media. Fixture/startup success is not a 99% reliability claim.

## Remaining acceptance / release gates

1. On CachyOS + Caelestia + Ghostty: actual images/protocol changes/theme updates,
   media keys/widget, desktop file portal, audible stations and mpv video output.
2. With explicit Brave Origin import: consent, expiry, brand account switching,
   signed-in Home/subscriptions/library/Music/continuations. No secrets in reports.
3. Run `tools/test-playback-matrix.sh` with independent short/long sources,
   1080p/1440p/2160p where offered, codecs/audio, longer durations, then the existing
   control/fault sweep. Record failures and actual decoded output separately.
4. Real YouTube full-collection expansion, selected ranges, cancellation/restart,
   unavailable entries and shared-group cases. The native extraction guard stays.
5. Review main integration, Arch recipe on Arch, license-only metadata entries,
   release provenance, checksum and update/rollback against a copy of old state.
   See [release/candidate instructions](RELEASE_CANDIDATE.md).
6. Complete remaining German guidance/status prose and target-PC lyric matching/
   synchronization acceptance; the implemented lookup/local-file workflows are
   described in [AUDIT_FEATURES.md](AUDIT_FEATURES.md).

## Verified alpha.12 delivery checkpoint: `e9718f9`

- Both GitHub Rust checks passed:
  [39931230850](https://github.com/Vanir20342222/ytui/actions/runs/39931230850),
  [39931240506](https://github.com/Vanir20342222/ytui/actions/runs/39931240506).
- Combined actual local media campaign: **10 passed**; separate private D-Bus
  MPRIS campaign: **1 passed**. These use generated/loopback media.
- Two clean-source archive builds on the same compiler/machine were byte-identical;
  SHA256 verified:
  `8cc19ce23e37ad9bdc7700b0fec37f0182376b57e32d57f3664f81f43b8cba25`.
  Both launchers reported full revision
  `e9718f981ff3b216e42c1866f1c5601c3474e887`; no missing dynamic libraries.
  The archive contains the license inventory (531 packages; 15 entries provide
  SPDX metadata only and still require release review). This is same-input archive
  repeatability, not cross-toolchain binary reproducibility or a published release.

The lyrics/localization follow-up is newer than that artifact. Its own test and
capture results are recorded below; the checkpoint checksum must
not be used to identify a later build.

## Lyrics/localization follow-up

The full offline suite passes **204 tests, 0 failed, 25 opt-in ignored**.
Settings render in English/German across all themes/sizes (2,340 states); lyric
loading/empty/candidates/text/error adds 650 combinations. Real-executable PTY
checks cover German search/settings and local lyrics. The four new captures are
WQHD; the manifest now lists **61 PNGs**. Settings, action search, main German
navigation and the lyric panel were also visually inspected.

New regressions reject stale/closed lyric and tool-panel responses, bound LRC and
HTTP bodies, reject redirect/error results without exposing query URLs, preserve
Unicode/repeated timestamps/global offsets, use offline local files, and prove
the lookup sends no cookie/auth/history fields. Named pipes are rejected without
blocking a worker. Compact German buttons adapt to their displayed widths.

No live LRCLIB match, signed-in account, physical output or target desktop result
is implied by these synthetic/PTY checks.

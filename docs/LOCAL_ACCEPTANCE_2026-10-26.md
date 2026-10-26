# Local acceptance — 26 October 2026 (M2–M8)

Application checkpoint: main `c6cf246c1a1a1c69e571de5c1d3bfd0575ad7411`
(PR #15). GitHub fetch/remote HEAD verification found no newer main commit.
Both installed launchers now report optimized `0.2.0-alpha.12` and that exact
application commit. The test-only repair below does not change that application.

The PC now runs **Omarchy**, kernel `7.2.5-3-omarchy`, following its October 26
restoration. CachyOS/Caelestia describe the historical target. Proton VPN was
initially disconnected; with explicit user authorization it was connected for
the live checks and restored to **Disconnected** afterwards. No VPN settings,
packages, services or desktop themes were changed.

## Actual live results on Proton VPN

| Check | Result and boundary |
| --- | --- |
| Native video / audio / Music audio | All three isolated live playback tests passed with real mpv null outputs and advancing clocks. Native mode retained its disabled external-extractor sentinel. |
| Installed native corpus, ten seconds | **2/3 passed.** The short YouTube and Music cases advanced beyond ten seconds. `aqz-KE-bpKQ` returned HTTP 429. The full corpus run therefore failed. |
| Public services | **7/7 passed:** YouTube search/pagination/details/cache, independent thumbnail CDN/disk cache, cliamp catalog/library, Radio Browser filters/cache, station/library-track playback, LRCLIB and SponsorBlock. |
| Anonymous doctor | YouTube search and radio passed. Music browsing required consent/sign-in; doctor correctly exited with failure. Music URL playback is a separate successful check. |
| Single-file downloads | Auto video/audio files passed ffprobe and full FFmpeg decoding. The separate strict-native fixture download also passed. |
| Anonymous two-entry playlist | Failed with bot-confirmation/HTTP 429 responses. This failure remains recorded. |
| Explicit Brave Origin import | Selected the discovered `Default` profile through `auth import --browser brave-origin --profile ... --file-store`, with private temporary config/state. Validation succeeded. Home, subscriptions, Watch Later and Music Home returned entries. No account labels or feed contents were retained. |
| Authenticated two-entry playlist | The same public playlist succeeded with the temporary session: **two distinct audio outputs**, 5,800,579 and 8,259,433 bytes, lasting about 362 and 516 seconds. Both passed ffprobe and full FFmpeg decoding. Temporary media and credentials were removed. |
| Session expiry/disconnect | Expired only the isolated session copy. A signed-in feed failed with the explicit expired-session/re-import message. Disconnect succeeded; temporary state was deleted. The real browser session stayed intact. |
| Native desktop completion | `playback_sweep --cases completion --desktop` reached EOF through `gpu-next` and `pipewire`, with mute retained. This establishes actual output opening/completion, not human audio/visual quality. |
| Ghostty images/theme | Inspected a cropped real Ghostty window displaying fictional Music cards and thumbnails. Auto negotiation emitted 474 Kitty image commands; normal exit succeeded. A read-only link to the actual palette in isolated state resolved `Theme: Omarchy · Omarchy`. No system theme was changed. |
| Unmuted radio output | A station from ytui's live catalog advanced to 3.062 seconds through enabled PipeWire, unmuted at 15% volume. This direct mpv probe complements the app's null-output radio checks; human audibility remains unconfirmed. |

These are limited local results, not a universal playback or 99% reliability
claim. Live/DRM/private/age-restricted media, larger account coverage, theme
changes while running, sustained playback, persistent-volume desktop use, and
VPN loss/reconnect during active playback remain separate checks. Controlled
player-recovery and persistence tests still pass.

## Deterministic and actual-tool checks

- `cargo test --workspace --locked`: **254 passed**, 29 opt-in ignored.
- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo test --workspace --locked -- --ignored --skip live_ --test-threads=2`:
  **11 passed**, including actual mpv sleep-timer expiry with queue retention,
  split/HLS streams, proxy routing, conversions and two loopback playlist files.
- `tools/test-native-solver.sh` with installed `/usr/bin/node`: **2 passed**.
- Private-session D-Bus MPRIS: **1 passed**; Lua player-menu check passed;
  CI media-setup fault/guard runner: **7 groups passed**.
- `./install.sh`: optimized build and both launcher replacements passed.
- Saved-shelf restart/disconnect/search and real-executable PTY journeys passed
  with synthetic state, without changing the user's bookmarks.

The first default run failed repeatably when the installer test fixture
truncated a recently executed binary (`ETXTBSY`). A minimal copy → `--version` →
in-place write reproduced it without a remaining executable process. Replacing
the inode succeeded. The test helper now uses the existing atomic-write utility,
matching the actual installer's publication policy. The focused test and full
suite then passed. Application playback behavior was not changed.

The first isolated doctor did not recognize the mise Node shim because its XDG
configuration was also isolated. Later isolated checks selected the installed
`/usr/bin/node` through test-only PATH ordering. The user's normal doctor finds
the mise shim successfully. Node is 26.8.1; installed Deno 2.9.6 is outside the
pinned supported version. No runtime or backend setting was changed.

## Evidence and rollback

Sanitized results, the cropped fictional Ghostty capture and reproduction
scripts are under `~/Work/ytui-local-acceptance-20261026/`. Browser evidence keeps
only exit codes, counts and generic expiry handling. No cookies, authorization
headers, private feed fixtures or signed media URLs are committed.

Previous launchers are in that directory's `before/bin/`; the installer also
saved `~/.local/bin/ytui.previous`. Binary-only rollback is available through
`tools/rollback-install.sh`. Existing app config/account store, queues/history,
bookmarks and downloaded media were preserved. Temporary Ghostty/mpv instances
were closed and the original window focus restored. Host-maintenance records
were updated with installation, VPN restoration, evidence and rollback paths.

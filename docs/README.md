# Documentation

## Current guides

| Document | Use it for |
| --- | --- |
| [Android app (M9)](android/README.md) | Install, navigation, build/check commands, repository layout and an index of every Android document |
| [Android player/library qualification (mobile.4)](android/PLAYER_LIBRARY_VALIDATION_2026-10-29.md) | APK, native UI/controls, offline folders/playlists, signing and tested/open gates |
| [Researched library/playback features](FEATURE_RESEARCH_2026-10-24.md) | Saved-media shelf, sleep timer, sources and verification |
| [Alpha.12 workflows](AUDIT_FEATURES.md) | New account, collection, format, cache and support panels |
| [Implementation tracker](AUDIT_IMPLEMENTATION.md) | Audit fixes, executed checks and remaining acceptance |
| [Candidate release](RELEASE_CANDIDATE.md) | Update, rollback, checksum/license archive and Arch recipe |
| [Usage and settings](RUST.md) | Build, update, navigation, configuration and backends |
| [Example configuration](../examples/config.toml) | Editable TOML showing current defaults |
| [Remaining work](REMAINING_WORK.md) | Unsupported features and release acceptance |
| [App audit and implementation backlog](APP_AUDIT_2026-10-22.md) | Alpha.11 bugs, executable repros, priorities and feature acceptance |
| [Parity](PARITY.md) | Rust implementation compared with the former Python app |
| [Open-PR platform parity, 3 November](PR_PARITY_2026-11-03.md) | Which open PRs cover desktop and Android, desktop counterparts added, remaining gaps |
| [Themes and screenshots](UI_THEMES.md) | Theme gallery and capture instructions |

## Validation and troubleshooting

| Document | Scope |
| --- | --- |
| [PR integration review](PR_REVIEW_2026-10-24.md) | PR dispositions, additional fixes and final integration checks |
| [Vinyl music player](VINYL_PLAYER.md) | Music/Radio sleeve and rotating record, pane/fullscreen modes and validation |
| [Local acceptance, 26 October](LOCAL_ACCEPTANCE_2026-10-26.md) | Actual Brave Origin, Ghostty and Omarchy results, with anonymous-service failures kept |
| [Recovery after local acceptance](RECOVERY_FIXES_2026-10-26.md) | Rate limits, live reconnect controls/backends, archived broadcasts and fixture evidence |
| [Validation record](VALIDATION.md) | Dated results; each section identifies its environment |
| [Single-video sweep](SINGLE_VIDEO_SWEEP.md) | Alpha.11 playback/recovery checks on the target PC |
| [Networking reliability review](NETWORK_RELIABILITY_2026-10-23.md) | Alpha.12 proxy/redirect/recovery fixes and public-service results |
| [Network and caching](NETWORK_PERFORMANCE.md) | Alpha.10 provider checks and measured cache performance |
| [Secret-scan review, 27 October](SECURITY_REVIEW_2026-10-27.md) | Vendored player-fixture API-key matches and their disposition |
| [Download-recovery fixture race](DOWNLOAD_TEST_FIXTURE_RACE.md) | `ETXTBSY` launch race between parallel tests that start freshly written fake executables, and its isolation fix (M8) |
| [CI playback diagnosis](CI_PLAYBACK_DIAGNOSIS.md) | Package-install timeouts versus real hosted-runner extraction failures |
| [Live YouTube checks](LIVE_YOUTUBE_TESTS.md) | Opt-in public-service runners and required tools |

For contributor commands, start with the checks in the [repository README](../README.md).
`tools/test-network.sh` collects the broad acceptance checks;
`tools/test-single-video.sh` runs the focused playback/fault sweep. Both contact
public services and require installed media tools. Ordinary Cargo tests remain
offline by default.

## Design and historical evidence

[The rewrite plan](../RUST_REWRITE_PLAN.md) records requirements and milestone
evidence. Earlier reports describe the code and environment at the time they
were written; their version numbers and failures are historical results.

- [Feature audit](FEATURE_AUDIT.md)
- [Playback and navigation plan](PLAYBACK_NAVIGATION_PLAN.md)
- [Feed recovery](FEED_RECOVERY_PLAN.md)
- [External playback and playlist repair](PLAYBACK_REPAIR.md)
- [Output and preview stability](STABILITY_PASS.md)
- [Native playback integration](NATIVE_PLAYBACK.md)

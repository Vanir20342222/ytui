# Documentation

## Current guides

| Document | Use it for |
| --- | --- |
| [Usage and settings](RUST.md) | Build, update, navigation, configuration and backends |
| [Example configuration](../examples/config.toml) | Editable TOML showing current defaults |
| [Remaining work](REMAINING_WORK.md) | Unsupported features and release acceptance |
| [App audit and implementation backlog](APP_AUDIT_2026-10-22.md) | Alpha.11 bugs, executable repros, priorities and feature acceptance |
| [Parity](PARITY.md) | Rust implementation compared with the former Python app |
| [Themes and screenshots](UI_THEMES.md) | Theme gallery and capture instructions |

## Validation and troubleshooting

| Document | Scope |
| --- | --- |
| [Validation record](VALIDATION.md) | Dated results; each section identifies its environment |
| [Single-video sweep](SINGLE_VIDEO_SWEEP.md) | Alpha.11 playback/recovery checks on the target PC |
| [Network and caching](NETWORK_PERFORMANCE.md) | Alpha.10 provider checks and measured cache performance |
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

# Playback CI diagnosis (M5/M8) — 2026-10-22

## Two distinct failures

The screenshot's [cancelled native run](https://github.com/Vanir20342222/ytui/actions/runs/39782801893)
at `7cc5a43` never built or ran playback. Its media-install step spent almost
25 minutes fetching packages from `azure.archive.ubuntu.com`, with repeated
ignored/retried downloads, before the job deadline cancelled it. This was a CI
setup failure and supplied **no playback result**.

The [latest native run before this repair](https://github.com/Vanir20342222/ytui/actions/runs/39804086027)
at `607dcf0` installed media tools in 36 seconds and built successfully. All
three real public playback cases then failed during native extraction in about
five seconds total, before mpv could play: “All player responses are invalid.”
The extractor also suggests possible IP blocking; that suggestion does not
independently establish the cause. This live failure is correctly a failed
check. [Rust CI on the same commit](https://github.com/Vanir20342222/ytui/actions/runs/39804086016)
passed its deterministic and actual loopback-media checks.

## Repair

- Pin the three media workflows to Ubuntu 24.04 x64, matching the supported
  setup helper instead of letting the runner image change underneath it.
- Give each complete apt update a 60-second deadline and each installation a
  180-second deadline, plus a bounded termination grace period. Per-request
  network timeouts alone cannot bound a hundred sequential package downloads.
- Retry once against official Ubuntu archive/security HTTPS mirrors using a
  temporary apt source override. Do not rewrite runner source files. Require
  update success; fail setup if both attempts fail. Allow only the three media
  packages these workflows need. Refuse execution outside the expected CI host.
- Bound the setup step to ten minutes, external-tool downloads to four minutes,
  and the native public playback step to eight minutes. Cancel superseded native
  runs and report setup/playback step outcomes separately in the job summary.
- Preserve the native live trigger, all public video cases, strict failure
  propagation, and the prohibition on external yt-dlp in native playback. No
  failed live check is relabelled as passed, skipped or an allowed failure.

These changes fix the demonstrated setup hang. They do not repair or bypass
YouTube's rejection of the hosted runner's native player requests.

## Regression evidence

`bash tools/test-ci-install-media.sh /tmp/<legacy-installer>` runs the original
unbounded apt sequence against fake CI tools. A package-install stall reproduces
the screenshot's failure path and fails with exit 124 at the harness deadline.
No real apt/sudo/package operation is performed on the developer's PC.

`bash tools/test-ci-install-media.sh` runs the new installer at the same seam.
All seven groups pass: install stall, update stall, primary success, update
failure, install failure, both mirrors failing, and environment/argument guards.
The mock shortens production deadlines and asserts the real timeout arguments,
requested packages, fallback source content and apt source isolation. A failed
fallback still exits nonzero. Rust CI now runs this regression before media setup.

Shell syntax, workflow YAML parsing and `git diff --check` pass locally. No Rust
application source changed, so this repair does not need a new application build
or launcher installation. Existing optimized alpha.11 was independently rerun:

```sh
YTUI_TEST_BINARY=target/release/ytui YTUI_TEST_PLAY_SECONDS=5 tools/test-native-playback.sh
```

**3/3 public native cases passed on the target PC**: video positions 5.000 and
5.067 seconds, Music audio position 5.010 seconds. No browser cookies were
imported and no VPN/settings were changed. This verifies the runner invokes real
playback successfully on this network; it does not establish universal YouTube
availability or a 99% reliability rate. See [the full prior local sweep](SINGLE_VIDEO_SWEEP.md)
for longer playback/fault coverage. Remote rerun results must be reported separately.

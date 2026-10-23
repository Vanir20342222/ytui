# Real YouTube checks (M3/M4/M5/M8)

For native-only watching on `fix/native-playback`, use installed Node.js 22+ or
Deno 2.9.7 and mpv:

```sh
cargo build -p ytui --locked
tools/test-native-solver.sh
tools/test-native-playback.sh
```

This separate runner never invokes external yt-dlp and measures actual mpv
progress. [Native playback evidence](NATIVE_PLAYBACK.md) records the failed
public runs and the open reliability gate. Pass a public URL file to extend the
corpus; startup success alone does not establish complete watching or 99% success.

For the broader existing download/fallback suite, run from this checkout with Rust, FFmpeg/ffprobe, mpv,
yt-dlp and a current Deno runtime installed:

```sh
tools/test-youtube.sh auto
tools/test-youtube.sh yt-dlp
```

These are explicit network tests, excluded from ordinary `cargo test`. They use
temporary configuration, state, caches and download destinations, never import
browser cookies, and delete downloaded test media afterwards. They preserve the
machine's inherited proxy configuration. No account, consent, VPN or system theme
settings are changed. The default public video is the short public `jNQXAC9IVRw`
("Me at the zoo"). The two-entry Music playlist check uses the public `JODA15` playlist from
yt-dlp's own YouTube tab tests. Public fixtures can change or become unavailable.

Checks include YouTube/Music search, video details, video and audio playback,
Music URL playback, a video download, Music audio conversion, and **two separate
playlist files**. Actual ytui processes run the configured native/auto/external
backend. Playback uses real mpv with null audio/video outputs and must advance at
least three seconds; it is not a check of the physical sound/display device.
ffprobe verifies nonempty completed downloads, duration and audio/video tracks.
The saved playlist job must retain its source URL and both distinct outputs.

A network block, bot challenge, absent runtime, empty response or first-song-only
download fails the suite; none becomes a passing skip. Strict native extraction
can fail YouTube signature challenges; `auto` and `yt-dlp` are separate runs.
The original strict-native download diagnostic remains available separately:

```sh
cargo test -p ytui --locked --test live_youtube live_native_ -- --ignored
```

Optional environment overrides: `YTUI_TEST_MPV`, `YTUI_TEST_YT_DLP`,
`YTUI_LIVE_VIDEO_URL`, `YTUI_LIVE_MUSIC_URL`, `YTUI_LIVE_PLAYLIST_URL`. Use short
public videos longer than three seconds and a public playlist with at least two
distinct playable entries. The media checks have a 240-second per-process limit.
The runner does not install missing tools. On Arch/CachyOS, install the appropriate
`rust`, `mpv`, `ffmpeg`, `yt-dlp` and `deno` packages using your normal package manager.

For a single public playback diagnostic with your configured mpv output:

```sh
ytui playback-check 'https://www.youtube.com/watch?v=jNQXAC9IVRw' --backend auto
ytui playback-check 'https://music.youtube.com/watch?v=jNQXAC9IVRw' --audio --backend yt-dlp
```

This command intentionally loads no browser session. It closes mpv after three
seconds of progress, prints backend/position/duration JSON, and returns nonzero
on playback failure or a 120-second startup timeout. Errors are sanitized rather
than exposing signed media URLs. It does not add playback to the saved queue.
Ctrl+C also stops/reaps the owned playback worker. A separate real-mpv loopback
regression checks both progress JSON and a visible decoder failure:

```sh
cargo test -p ytui --test downloads --locked real_mpv_playback_check_ -- --ignored
```

The separate **YouTube live** GitHub workflow runs a public network preflight
before installing tools/building/running the same tests in both auto and yt-dlp modes. It is triggered by
changes to its test files on the repair branch; `workflow_dispatch` is available
once GitHub registers it on the default branch. Hosted-runner IPs may themselves
be challenged by YouTube; a failed hosted test still needs a run on the target PC.
No cookies or media are uploaded as artifacts.

## Evidence on 2026-10-22

The managed workspace reports enforced unrestricted HTTP policy, but the inherited
proxy rejects the YouTube HTTPS CONNECT request with HTTP 403 (curl exit 56).
This is before any YouTube HTTP response. Retained network diagnostics are absent.
No proxy or route bypass was attempted. A separate GitHub runner is being checked;
Both public roots returned **HTTP 200** on GitHub's Ubuntu runner:
[network job, run 39756304985](https://github.com/Vanir20342222/ytui/actions/runs/39756304985/job/119749053849).
This establishes page reachability there; it does not establish stream extraction
or playback. The repeated [media run 39757202167](https://github.com/Vanir20342222/ytui/actions/runs/39757202167)
ran all nine checks in **auto and yt-dlp modes**: each had **two passing searches
and seven failures**. YouTube required a bot-confirmation sign-in for details and
media on both hosted runners. No successful CDN playback/download is claimed.
Use the local runner for target-network evidence; never upload browser cookies to
make a public CI job pass. These checks intentionally stay failed for that response.

The older `BaW_jenozKc` fixture returned unavailable in the initial run; the second
run used the replacement above and reported the distinct bot challenge.

The previously failing Rust workflow had **no jobs started**: GitHub rejected
case-insensitive duplicate `NO_PROXY` / `no_proxy` environment keys. The duplicate
was removed and both workflow files are checked with actionlint before pushing.

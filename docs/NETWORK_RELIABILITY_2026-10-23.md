# Networking reliability review — 23 October 2026 (M2–M6/M8)

This follow-up reviews alpha.12 on `fix/audit-implementation`, starting at
`504585f`. It covers provider requests, redirects, proxy routing, feed recovery,
thumbnail and cover fetching, settings changes, and media handoff. It is not a
claim that every video, account, VPN region or station works.

## Reproduced bugs and repairs

| Failure | Repair and regression evidence |
| --- | --- |
| A watch-page HTTP 429 with `Retry-After: 120` was discarded by the video-details fallback, which immediately contacted the API. | Preserve transient transport/body/status errors and their cooldown instead of issuing another endpoint request. The new regression failed before the fix and now requires one request and the full 120-second cooldown. Unsupported page metadata still allows API fallback. |
| A `.pls` link redirected to an extensionless catalog and was returned without parsing its actual stream entry. | Follow bounded public redirects, parse the final playlist, resolve relative entries against the final URL and recognize PLS content even without an extension. The reproduction failed before the fix. HLS manifests and direct audio/video responses remain mpv inputs; the resolver does not consume an endless audio stream. |
| Four valid nested playlists ending at a stream incorrectly exceeded the four-level limit. | Check the terminal stream before rejecting another playlist level. Added a previously failing four-playlist regression; cycles and excess nesting remain errors. |
| Radio redirects could multiply per-request timeouts; wrapper playlists inherited the longer general client deadline for each hop. | Whole-operation deadlines: eight seconds for one catalog/discovery/server request, twelve seconds for playlist resolution including redirects and bodies. A real loopback server sends headers then stalls its body; both operations time out within their budgets. A full directory fetch can involve discovery followed by server probes, and cliamp can additionally fall back to the directory, so eight seconds is not an overall tab deadline. |
| Native download cover art used a different proxy client, with local DNS for explicit `socks5` URLs. | Shared explicit-proxy construction for async and blocking HTTP clients. Both use proxy-side DNS for SOCKS5/5h and explicit routing independent of ambient bypass rules. Loopback HTTP/SOCKS proxies require an unresolvable `.invalid` destination and verify the remote DNS request for both clients. Cover redirects are bounded and reject credentials/HTTPS downgrades. |
| After a proxy change, old feed requests could still complete and replace the current view; failed tabs and exhausted thumbnail retries could remain stuck. | Cancel old feed/card requests, invalidate generations, retain visible items and selections, and reschedule pending/failed feeds using the new providers. Radio server changes also reset pending radio requests. The previously failing UI regression verifies cancellation, stale-result rejection, retained content and radio recovery. Existing downloads/playback continue under the documented “changes apply to new operations” policy. |

The interrupted-body path was also checked: the existing bounded body reader
already classifies transfer errors as retryable. A new wire-level regression
truncates both bootstrap and API responses and verifies recovery on the next
request. This is additional coverage, not a newly repaired defect.

## Checks run

- Full deterministic workspace suite: **210 passed, 0 failed, 28 opt-in ignored**.
- Strict all-target workspace Clippy, formatting and diff whitespace checks pass.
- Actual local media suite: **10 passed**. Uses installed mpv, yt-dlp and FFmpeg,
  synthetic/loopback media, HTTP proxy routing, HLS, split streams, playback
  controls, conversions and multiple output files.
- Public networking suite: **5 passed, 2 failed**; failures are kept as failures.

| Public check | Observed result in this cloud environment |
| --- | --- |
| cliamp catalog and track library | Pass: 15 catalog entries and nonempty library. |
| Radio Browser | Pass: jazz search, Germany by name/code, classical tag, pagination and memory cache. |
| Actual radio media | Pass: mpv advanced the station stream 3.111 seconds and a library track 3.093 seconds, with null audio/video outputs. This is a short progress check, not full-length or audible acceptance. |
| LRCLIB | Pass: returned candidates for a fixed public song; no personal listening history was sent. |
| SponsorBlock | Pass: public endpoint returned a supported response for a fixed public video. The API permits no-segment results; this alone does not demonstrate a visible skip during playback. |
| YouTube browsing | Fail: `cannot reach www.youtube.com … tunnel error: unsuccessful`, before receiving a YouTube response. |
| Independent YouTube thumbnail CDN | Fail: the same tunnel failure for `i.ytimg.com`. Separating this test prevents a failed browse bootstrap from masking the CDN check. Local thumbnail decode, sizing, memory/disk cache and recovery remain covered by deterministic tests. |

The environment reports an enforced unrestricted network policy, but these
observed proxy tunnel failures still prevent the two requests. The underlying
proxy failure is not diagnosed by that policy status alone. No inherited proxy,
CA trust, VPN route or browser session was bypassed or modified.

## Reproduce and remaining acceptance

```sh
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
# Requires installed media tools; uses synthetic/loopback fixtures:
cargo test --workspace --locked -- --ignored --skip live_ --test-threads=2
# Explicit public service access; no browser import or VPN changes:
cargo test -p ytui-services --locked --test live_network -- --ignored --test-threads=3 --nocapture
```

`tools/test-network.sh` includes the expanded public suite and returns nonzero
when any group fails. Use it on the target CachyOS/Proton VPN machine for the
remaining live YouTube playback, downloads and account checks. Signed-in
Home/Music, real Brave Origin/keyring access, VPN disconnect/reconnect, HTTPS
proxy authentication, physical audio/visible Ghostty and long playback remain
acceptance gates. mpv still explicitly rejects a configured SOCKS proxy; use an
HTTP proxy or the existing system VPN route for playback. No fallback policy,
account permissions or release gate was silently changed.

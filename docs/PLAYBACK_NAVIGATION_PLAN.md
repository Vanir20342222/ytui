# Playback, browsing and navigation follow-up (M3–M8)

User report: native extraction cannot resolve YouTube signature/n challenges;
volume resets to 70, controls lag, thumbnails load late and have inconsistent
sizes, browsing stops at page boundaries, and command-only features are hard to find.

Implementation order:

1. Make extraction failures actionable in the UI. Initially retain native/default behavior pending the question; the user then
   confirmed recommending automatic fallback (see implementation below).
   Verify upstream capabilities before claiming current YouTube playback works.
2. Preserve volume and mute across station changes/restarts; bound IPC polling
   independently of input. Improve playback controls, queue continuation and live
   station selection. Add non-destructive SponsorBlock marks/skip/undo controls.
3. Cache decoded thumbnails in memory and resized images on disk; select a suitable
   CDN resolution, prefetch nearby items, bound requests/cache size, and remove
   loading text. Keep Music artwork square and give the catalog most of the width.
4. Prefetch continuation pages near the end without duplicating items or retrying
   failures continuously. Preserve section labels and server-provided category
   endpoints for YouTube/Music instead of inventing personalized categories.
5. Replace the bottom command entry with a searchable Actions panel, structured
   parameter forms, visible Settings access and a toggle for shortcut hints.
   Improve download forms and show a clearer interactive playback timeline.
6. Run deterministic provider/player/cache/UI regressions, actual PTY navigation,
   format/lint/build, refresh WQHD screenshots, and attempt opt-in live checks.
   Document exactly what remains blocked by cloud networking or requires the
   user's browser, Ghostty, mpv/audio device or Proton connection.

Player direction: supervised mpv with its on-screen controller, built-in menus,
quality/audio/subtitle/speed controls and ytui settings. Ad-free direct extraction
does not insert website ads; SponsorBlock is a separate public API. Do not promise
YouTube website UI parity or bypass server-side access/bot restrictions.

## Implementation/evidence (alpha.5)

- The user confirmed recommending automatic fallback. New config default is auto;
  strict native and explicit yt-dlp remain. Existing choices are not overwritten.
  Extraction limitations and rejected native playback streams can use the fallback;
  tool installation remains explicit. Pinned upstream has no newer revision.
- Persistent volume/mute/speed, bounded batched IPC, background state persistence,
  direct live stations, loaded-song continuation, interactive timeline/player
  actions and a bundled mpv window menu are implemented.
- Playback SponsorBlock uses public-ID/category API requests without credentials,
  non-destructive marks/skip/undo, and skip/mark/off/category settings. Direct media
  extraction avoids the website ad player; embedded stream ads are not guaranteed.
- Sized advertised thumbnail variants, dynamic CDN size hints, bounded decoded and
  resized disk caches, shared in-flight requests, nearby prefetch and visible-card
  protocol retention replace late full-size image processing/loading text.
- Continuation prefetch preserves selection and deduplicates/stops repeated pages;
  provider section labels/category endpoints remain attached to cards/filters.
- Actions/parameter forms replace bottom command input. Search is query/URL only.
  Nine settings sections / 53 fields, three download sections, visible entry points,
  remapped hint labels and F1 toggling are implemented.
- 94 default tests + separate real FFmpeg and real mpv 0.40 headless tests pass. Settings/download ends render
  in 1,560 additional theme/size states. 50 WQHD PTY captures use synthetic content;
  standalone Lua menu behavior is checked against a synthetic mp API. Real mpv
  also exercises its menu/quality request, audio/subtitle switching, pause/seek,
  preference preservation and SponsorBlock replay with null outputs.
- Four real public YouTube tests fail on this cloud's HTTP/proxy access. Successful
  live YouTube, signed-in browsing, Ghostty/Brave/Proton and displayed mpv/device output
  remain open. Cliamp catalog and Lofi track list HTTP fetches succeeded. Real mpv also decoded
  three seconds of Lofi into null audio output through the proxy; real devices
  and reconnect are unverified. SponsorBlock test-video lookup returned 404/no marks.

Full commands, results and boundaries: [VALIDATION.md](VALIDATION.md).
Update/use instructions: [RUST.md](RUST.md).

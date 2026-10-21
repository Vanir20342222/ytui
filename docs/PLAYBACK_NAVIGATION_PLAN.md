# Playback, browsing and navigation follow-up (M3–M8)

User report: native extraction cannot resolve YouTube signature/n challenges;
volume resets to 70, controls lag, thumbnails load late and have inconsistent
sizes, browsing stops at page boundaries, and command-only features are hard to find.

Implementation order:

1. Make extraction failures actionable in the UI. Keep native as default and the
   existing explicit external yt-dlp fallback unless the user changes that decision.
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

Status/evidence will be recorded here as implementation progresses.

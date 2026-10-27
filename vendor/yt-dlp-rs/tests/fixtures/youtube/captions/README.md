# Saved YouTube WEB caption references

Official pinned source: `897341241f2bfb2439fe0367f4c7c9ccd29a3828`.
Run `python3 tools/compare_youtube_captions.py --upstream /path/to/pinned/yt-dlp`.

Twenty shared extraction cases reference saved `player-*.json` responses, plus
one explicitly source-only missing-language case. Five selection cases compare
normal/automatic precedence with actual `YoutubeDL.process_subtitles`.
`cases.json` records exact ordered maps as `{language,formats}` arrays, each
format's stable `url`, `ext`, `name`, inferred `audio_language`, actual token
provider requests, warning presence and whether the policy skip reporter ran.
Neither URLs, names, languages, format order nor stable query order is discarded
or rewritten for comparison.

The reference injects a single WEB initial player response with its actual
client context and a subtitle token callback returning None. It calls actual
`YoutubeIE._real_extract`, including its complete caption loop. It does not use
missing-token overrides. Only the video format iterator is replaced by one
fixed muxed format and empty manifest captions: this isolates captions without
claiming video token policy, media extraction, sorting or decipher parity. The
saved response still includes a minimal playable itag18 response. Network,
webpage/manifest acquisition, subprocesses and downloads are forbidden. No
Python extraction is used by native runtime code.

The shared cases cover:

- Seven declared formats (`json3`, `srv1`, `srv2`, `srv3`, `ttml`, `srt`, `vtt`),
  actual query updates, grouped duplicate keys, dropped blank values, existing
  fmt replacement, complete xosf removal, reserved encoding and query key order.
- `vssId` before languageCode, one leading dot removal and remaining dots to
  hyphens; names from simpleText or only the first run, including null names.
- Manual translation disabled/enabled via actual write-auto-subs/list-subs
  parameters; `und` remains a special case even when manual translation is off.
  Translated manual keys and tlang values carry the source's language suffix.
- ASR translation independent of that flag; original aliases and inferred audio
  language; original query lang uses its last duplicate value; no added tlang
  when that original language matches. Fallback ASR languages absent from the
  translation list are retained, and generated-name suffixes are removed.
- `exp=xpe` or `exp=xpv` requests a required token once and skips remaining
  tracks for the client when none is returned, even with a pot already present
  in the URL. A comma-valued `exp=xpe,xpv` is not treated as either experiment.
  A protected later track does not cause a second token callback. Warnings
  follow actual source behavior: write-auto/list affects the final summary.
  Native diagnostics explicitly warn for every rejected token experiment; native
  tests compare policy_skip rather than claiming identical warning severity.
- Source URL postprocessing uses actual `sanitize_url` after `_real_extract`,
  as `process_video_result` does. Protocol-relative captions therefore become
  http, while root-relative captions join the YouTube HTTPS base. No scheme
  differences are ignored.

Native caption parser tests consume the saved player responses and expected
ordered metadata. The selection fixtures carry full ordered input maps with
write_manual/write_auto, literal languages, format and exact selected metadata.
When both are enabled, normal subtitles override automatic captions for the same
language. A normal en-prefix also precedes an automatic exact en default. This does not claim general regex, ASR acquisition or payload parity.

Missing manual language is source-only: it is skipped upstream, rather than
assigned a guessed language. Malformed truthy objects, invalid metadata types,
unsupported URL schemes, arbitrary client policies, multiple player responses,
provided token acquisition, and translated malformed language names are outside
the shared strict native contract. Missing ASR names in the supported shape
preserve the actual source quirk: null normal name and `None (Original)` for the
original alias. Sidecar downloads and writes are tested separately by native
local fixtures, never by this offline reference tool.

Source references:

- `extractor/youtube/_video.py:142`: subtitle format declaration order.
- `extractor/youtube/_video.py:4199-4343`: actual caption loop, language/name
  processing, translations, original aliases, language inference, token checks,
  query updates, per-client break and final warning policy.
- `extractor/youtube/_video.py:3208-3225`: token-skip diagnostic policy.
- `extractor/youtube/_base.py:70-94`: WEB policies, including subtitle token not
  ordinarily required; `1184-1204`: `_get_text(..., max_runs=1)` semantics.
- `utils/_utils.py:2570-2571,2619-2620`: parse_qs and update_url_query, used by
  the actual source, rather than a tool-side reimplementation.
- `YoutubeDL.py:2899-2919`: subtitle URL sanitation and selection after extraction;
  `3157-3220`: normal/automatic selection and format preference semantics.

These source files contain broader client/API/token/extractor and renderer
behavior that is explicitly not established by the caption fixtures.

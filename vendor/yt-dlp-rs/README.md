# yt-dlp-rs

An incremental, native Rust adaptation of [yt-dlp](https://github.com/yt-dlp/yt-dlp).
It downloads direct HTTP media and a limited subset of HTML5 media pages, with
a foundation for porting extractors and tracking upstream changes. This is an
early prototype. YouTube (videos, playlists, channel tabs, search), Reddit,
X/Twitter, TikTok, Instagram and Facebook are extracted like pinned upstream on
saved fixtures, but have not been verified against the live sites.

Pre-release binaries for Linux, macOS and Windows are attached to the
[GitHub releases](https://github.com/aagedal/yt-dlp-rs/releases): pushing a
`vX.Y.Z[-PRE]` tag runs `.github/workflows/release.yml`, and a tag with a
pre-release suffix publishes a pre-release.

Requires current stable Rust (including standard-library file locking). Python 3.10+ and Git are needed
only for the upstream development tools. The Rust binary does not invoke Python
or an installed yt-dlp.

```sh
cargo build --locked
cargo run --locked -- 'https://example.com/video.mp4'
cargo run --locked -- --dump-json 'https://example.com/video.mp4'
cargo run --locked -- --list-formats 'https://example.com/page-with-multiple-sources.html'
cargo run --locked -- --list-subs 'https://example.com/video-with-tracks.html'
cargo run --locked -- --write-subs --sub-langs all --skip-download 'https://example.com/video-with-tracks.html'
cargo run --locked -- --write-subs --convert-subs srt --ffmpeg-location /path/to/ffmpeg 'https://example.com/video-with-tracks.html'
cargo run --locked -- --embed-subs --ffmpeg-location /path/to/ffmpeg 'https://example.com/video-with-tracks.html'
cargo run --locked -- --write-auto-subs --sub-langs en --skip-download 'https://www.youtube.com/watch?v=VIDEO_ID'
cargo run --locked -- -f 1 'https://example.com/page-with-multiple-sources.html'
cargo run --locked -- -f 'best[height<=720][ext=mp4]/best' 'https://example.com/page-with-multiple-sources.html'
cargo run --locked -- -f 'bestvideo+bestaudio/best' 'https://www.youtube.com/watch?v=VIDEO_ID'
cargo run --locked -- -I '3,1:2' --playlist-reverse -o '%(playlist_index)03d-%(title)s.%(ext)s' 'https://example.com/multiple-videos.html'
cargo run --locked -- --download-archive downloaded.txt 'https://example.com/video.mp4'
cargo run --locked -- --simulate 'https://example.com/video.mp4'
cargo run --locked -- --cookies cookies.txt 'https://example.com/video.mp4'
cargo run --locked -- -x --audio-format mp3 --ffmpeg-location /path/to/ffmpeg 'https://example.com/video.mp4'
cargo run --locked -- --remux-video mkv --ffmpeg-location /path/to/ffmpeg 'https://example.com/video.mp4'
cargo run --locked -- --proxy 'socks5://127.0.0.1:1080' 'https://example.com/video.mp4'
cargo run --locked -- --user-agent 'My media client' --add-headers 'X-Media-Key:example' 'https://example.com/video.mp4'
cargo run --locked -- -o 'downloads/%(title)s.%(ext)s' 'https://example.com/audio.mp3'
```

Options can also come from configuration files like upstream's (named `yt-dlp-rs`, e.g.
`~/.config/yt-dlp-rs/config`; see `--ignore-config` and `--config-locations`).
ffmpeg and ffprobe are found on `PATH` like upstream; `--ffmpeg-location` accepts
either binary or its containing directory.

The example URLs illustrate syntax; replace them with actual media or supported HTML5 page URLs.
Install locally with `cargo install --locked --path .` to use `yt-dlp-rs` directly.

Supported today:

- HTTP(S) media identified by recognized audio/video MIME types, with a known
  file-extension fallback for absent or `application/octet-stream` content types.
- Redirects, percent-decoded URL filenames, query strings, and streamed downloads.
- UTF-8 HTML pages with `<video>`/`<audio>` elements and nonempty
  `src` or direct child `<source src>` elements. Progressive sources default to
  the highest declared resolution;
  `-f/--format` accepts `best`, `worst`, or a source ID (`0`, `1`, … in declaration order per element). Relative
  sources, the first `<base href>`, HTML entities, page redirects, and page referrers are supported. The media
  response must pass the same MIME/extension checks as a direct URL.
- Native HLS VOD downloads from direct playlist URLs or HTML5 sources:
  master selection, MPEG-TS/AAC segments, one fMP4 initialization map, and exact
  byte ranges. JSON/simulation fetch manifests but never segment bodies.
  Native AES-128 CBC decryption supports identity keys, explicit/sequence IVs and
  key rotation. External audio renditions retain group, language, name and
  default/autoselect metadata; automatic merging stays within the declared audio
  group. Live, SAMPLE-AES/DRM, discontinuous and external video/subtitle
  renditions remain unsupported.
- Static single-period DASH MPDs from direct URLs and HTML5 sources: inherited
  BaseURLs, numbered/time templates, finite timelines, SegmentLists and exact byte
  ranges. Native fragment downloads feed explicit audio/video merging. Dynamic,
  DRM, multi-period and SegmentBase/SIDX presentations remain unsupported.
- Multiple nonempty media elements form an HTML5 playlist, processed in document
  order. Empty elements are skipped. IDs and titles use upstream's numbered
  `-1` / ` (1)`, `-2` / ` (2)` suffixes. Titles prefer
  Open Graph `og:title`, then `<title>`, then the page URL filename stem.
- Typed progressive source metadata (`formats`, selected `format_id`, optional
  `width`/`height`) for HTML5. Child source dimensions come from positive integer
  `width`, `height`, and fallback `res` attributes. Ranking uses the smaller known
  dimension, or the only known one; unknown resolution sorts lowest. Ties select
  the last declaration for best/default and the first for worst.
- Streamable videos (`streamable.com/ID`) through the site's video JSON and
  Megaphone players (`player.megaphone.fm/ID`) through their page,
  ReverbNation songs through the song API, Hypem tracks through their page
  and served source, HistoricFilms tapes through their page, Livestreamfails
  clips through the clip API, EbaumsWorld videos through the player XML,
  Glide messages through their share page, GodTube videos through their
  player XML, FC Zenit videos through the player API, BehindKink posts
  QingTing programs, OnePlace episodes, SzTV videos, Photobucket media and
  This American Life episodes through their pages, JW Platform media through
  the media API (and The Intercept posts that embed it), 23video players
  (and Filmweb articles that embed them), Vocaroo recordings, AudioBoom posts,
  Tweakers videos, Angel episodes, Blogger videos (and their embeds), Nobel
  Prize media, Bild clips, Masters videos, Sen videos, Kommunetv archives,
  Graspop concerts, CozyTV replays, Premiership Rugby videos, CJSW episodes,
  Player.fm episodes, RTRFM shows, HitRecord records, Culture Unplugged films,
  Zhihu videos, CRTVG programmes, Breitbart videos, CGTN news videos, JoVE
  videos, ParlView videos, The Epoch Times videos, eBay items, Bitmovin
  streams and Crooks and Liars videos (and their embeds), Huajiao lives,
  ScreenRec shares, Ku6 shows, Fuyin TV videos, Moving Image Archive films,
  Fathom shares, Maariv players (and their embeds), Harpodeon films,
  Yorkshire Film Archive pages, GodResource videos, PopcornTV videos, Reddit
  posts (hosted videos, text-post videos and followed links), X/Twitter posts
  (GraphQL with a guest token; videos, multi-video playlists, cards), TikTok
  videos (webpage data, the page's JS challenge, short links), Instagram
  posts and reels (logged out; carousels as playlists), Facebook videos
  (relay data with DASH, progressive URLs and captions), Vimeo player and
  clip pages, Dailymotion videos, SoundCloud tracks, Twitch VODs and clips,
  Bandcamp tracks, Rumble videos, Kick VODs, clips and live channels, Bluesky
  posts, Mixcloud shows, Pinterest pins, Odysee/LBRY streams,
  Tumblr posts, Imgur media, galleries and albums, Bilibili videos, VK videos, Niconico videos, Weibo posts, TED talks, playlists and series, Rutube videos, LinkedIn posts, Internet Archive items, BitChute videos, Coub clips, Loom videos, OK.ru videos, plus
  either site's iframe embeds on other pages (one is followed; several form a
  playlist), compared with pinned upstream output on saved responses.
- YouTube single-video watch, short-link, Shorts, embed and live URL shapes,
  extracted like upstream from the watch page plus one player API request per
  default InnerTube client. With a JS runtime (deno, enabled by default as
  upstream does, or `--js-runtimes node`) the clients are `tv_samsung`,
  `visionos` and `web`, and `n`/signature challenges are solved through the
  bundled pinned EJS release. Without one, only `visionos` is used, with
  upstream's warning. HTTPS, HLS and DASH formats follow upstream's
  `_extract_formats_and_subtitles`: PO-token policies skip token-requiring
  formats, since no token providers exist. Playlists (`/playlist?list=` and
  `watch?v=…&list=`) are extracted like upstream's `youtube:tab`, with every
  continuation page, and so are channel tabs (`/@NAME/videos`, `/shorts`,
  ...); a channel URL without a tab reads `/videos` but is an error when the
  channel also has live or shorts tabs. `ytsearch[N|all]:QUERY` and
  `/results?search_query=` searches work like upstream's `youtube:search`.
  Live videos, login and upstream's
  remaining metadata fields are not supported yet. This is
  fixture-tested against pinned upstream, not verified against the live site.
- Separate streams support `-f bestvideo+bestaudio` or literal pairs such as
  `-f 137+140`, with `/` fallbacks such as `bestvideo+bestaudio/best`.
  `--ffmpeg-location PATH` configures an explicit FFmpeg executable for local
  stream copying; default selection then prefers a video/audio pair. Without it,
  default selection requires an independently playable source. `-f best` always
  selects an independently playable source. Chained format filters constrain
  dimensions, filesize, audio/total bitrate, extension, IDs and language/rendition
  text before ranking. For example, `bestvideo[height<=720]+bestaudio[abr>=128]/best`
  and `best[filesize<10MiB]`; `<=?` includes unknown values. This is a bounded
  selector subset; filters do not add metadata that an extractor did not expose.
  Parenthesized groups and aliases (`b`, `bv`, `ba`, `w`, `wv`, `wa`) are supported,
  as are bare filters such as `[height<=720]` (implicit `best`). For example,
  `(bv[height<=720]+ba)/b` preserves the fallback around the complete pair.
  Positive rank suffixes such as `bv.2`, `ba.2` and `worst.2` choose the Nth
  candidate after filters in upstream `FormatSorter` order; out-of-range ranks allow
  fallback. `-S`/`--format-sort`, `--format-sort-force` and `--prefer-free-formats`
  reorder formats like upstream.
- `-F/--list-formats` lists all eligible formats without choosing a source or
  downloading media, including separate-only presentations without FFmpeg.
  Tables show IDs, containers, dimensions, stream kind, size, bitrate and language.
  With `-j` or `-s`, normal selection still applies; `-F -j` prints a table followed
  by selected JSON. Table styling is native, not byte-for-byte upstream output.
- `-I/--playlist-items` selects inclusive ranges, signed indices and steps, preserving
  requested order and deduplicating overlaps. `--playlist-start`, `--playlist-end`
  and `--playlist-reverse` are supported for fully extracted HTML5 playlists.
  Original indices and selected order are exposed in playlist entry JSON.
- Generic results carry upstream `timestamp` from `Last-Modified` with derived
  `upload_date`; single HTML5 pages add `og:description`, `og:image` and RTA
  `age_limit`.
- HTML5 direct-child subtitle/caption `<track>` metadata, `--list-subs`, and
  `--write-subs` sidecars selected with literal `--sub-langs` tags/all/exclusions
  and ordered `--sub-format` preferences. `--skip-download` writes requested
  sidecars without media or archive records. Sidecars use native HTTP retries,
  referrers, temporary cleanup and no overwrite. The supported YouTube response
  subset also discovers manual and automatic captions, translations, original
  aliases and names. `--write-auto-subs` enables automatic sidecars independently;
  manual tracks win shared languages when both writing flags are enabled.
  Caption PO-token restrictions are preserved; no token acquisition is implemented.
  `--convert-subs FORMAT` accepts `srt`, `vtt`, `ass` or `lrc` for written sidecars
  before media, including with `--skip-download`. Native TTML/DFXP-to-SRT needs
  no FFmpeg; other supported conversions use the explicit local executable.
  `none` disables conversion; `-k` retains originals and intermediate SRT.
  [Conversion limits](docs/COMPATIBILITY.md#subtitle-conversion) are documented.
  `--embed-subs` embeds selected sidecars after media processing for
  MP4/MOV/M4A/WebM/Matroska; WebM requires WebVTT. It implies manual subtitle
  acquisition unless automatic writing is enabled. Explicit `--write-subs` keeps
  embedded sidecars; embed-only removes eligible sidecars after successful final
  publication. `-k` also keeps them. [Embedding policies](docs/COMPATIBILITY.md#subtitle-embedding)
  cover automatic tracks, skipped formats and metadata. Regex languages and
  manifest/other-site subtitles remain unsupported.
- `--write-info-json` writes upstream-cleaned video metadata beside the media
  name (after subtitles, before media) and playlist metadata before entries.
  Names follow upstream extension replacement; existing files follow the
  overwrite options and `-j`/`-s`/listing never write. Content covers the
  represented metadata subset; `_version` and `--load-info-json` are not implemented.
- `-q/--quiet`, `--no-quiet` and `--no-warnings` gate informational and warning lines;
  `-v/--verbose` adds upstream's `[debug]` lines.
- `--match-filters`/`--break-match-filters` filter videos with upstream syntax;
  `--date`/`--datebefore`/`--dateafter` and `--age-limit` select by upload date
  and age rating.
- `--break-on-existing`, `--break-per-input`, `--force-write-archive` and
  `--no-download-archive` follow upstream archive control.
- `-r/--limit-rate RATE` throttles media transfers like upstream; `--sleep-interval`,
  `--max-sleep-interval` and `--sleep-subtitles` pause before downloads.
- `--write-link`/`--write-url-link`/`--write-webloc-link`/`--write-desktop-link`
  write internet shortcuts.
- `--load-info-json FILE` processes saved `.info.json` dictionaries without
  extraction (playlists fall back to their URL, as upstream).
- `--exec [WHEN:]CMD` runs shell commands at upstream's stages (POSIX only).
- `--mtime` sets downloaded file times from `Last-Modified`; FFmpeg outputs keep
  their inputs' modification time.
- `--abort-on-error` stops at the first error; `-i` also continues past failed
  postprocessors (the default continues past failed inputs).
- `--max-downloads NUMBER` (exit 101) and `--min-filesize`/`--max-filesize SIZE`
  for whole-file HTTP media follow upstream.
- `--embed-metadata` writes title/date/description/URL/artist/genre/language tags
  with explicit FFmpeg (no chapters or info JSON attachment).
- `--list-thumbnails`, `--write-thumbnail` and `--write-all-thumbnails` use YouTube
  thumbnails, HTML5 `poster` images and page `og:image`. `--convert-thumbnails FORMAT` converts
  them and `--embed-thumbnail` embeds the best one into mp3, mkv/mka and
  mp4/m4a/m4v/mov with explicit FFmpeg (no ogg/opus/flac embedding).
- `--write-description` writes `.description` files; `--no-write-playlist-metafiles`
  disables playlist metadata files.
- `-w/--no-overwrites`, `--force-overwrites` and the default follow upstream:
  existing media is "already downloaded" by default while sidecars are rewritten.
- `--download-archive FILE` skips recorded extractor/video IDs before selection and
  records only successfully published downloads. Metadata/listing/simulation never
  write the archive. Extraction still makes normal requests for recorded inputs.
- Global `--add-headers FIELD:VALUE` (repeatable), `--user-agent` and `--referer`
  defaults across extraction, downloads, fragments, keys and subtitles. Explicit
  extractor/transport headers take precedence, including page referrers and identity
  encoding. A shared in-memory cookie jar learns scoped response cookies. `--cookies FILE` loads/saves Netscape cookies, including session cookies; `--no-cookies`
  disables file persistence. Raw Cookie headers remain unsupported.
  `--cookies-from-browser` reads Firefox, Chromium-based (macOS keychain, Linux
  BASICTEXT/KWallet) and Safari cookie stores natively; Windows DPAPI and the GNOME
  keyring are not implemented.
- `--socket-timeout`, `--source-address`, `-4`/`-6` and `--no-check-certificates`
  configure every native HTTP client.
- Explicit `--proxy URL` for HTTP/HTTPS or SOCKS4/4a/5/5h routing across the
  native HTTP clients. `--proxy ''` forces direct connections. Pinned SOCKS5
  compatibility uses proxy-side DNS; SOCKS4 user-ID credentials remain unsupported.
  HTTPS proxy and CONNECT transfers retain certificate validation.
- `-J/--dump-single-json` prints one object per URL (playlists with selected entries).
- `-j/--dump-json`, `-s/--simulate`, `-o/--output`, multiple URLs, help and version.
- `-a/--batch-file FILE` reads URLs (`-` for stdin) before positional URLs, skipping
  blank and `#`/`;`/`]` comment lines and ` #` trailing comments; `--no-batch-file`
  disables it. See [batch files](upstream/notes/batch-file.md).
- `-O/--print [WHEN:]TEMPLATE` prints field lists (`-O id,title`, `-O title=`)
  or unsanitized output templates per selected video, including `j` JSON values,
  `filename` and selected `urls`; `-e`, `--get-id`, `-g` and `--get-filename` are
  supported aliases. Printing implies quiet simulation. `--print-to-file` appends
  video-stage lines without implying simulation; `--no-simulate` downloads after
  printing. `before_dl`, `post_process`, `after_move` and `after_video` stages
  print with `filepath`/`_filename` and download without simulation;
  `pre_process`, `after_filter` and `playlist` are not implemented; see
  [print templates](docs/COMPATIBILITY.md#print-templates).
- Output templates follow upstream's key grammar and conversions: traversal
  (`%(formats.0.format_id)s`, `%(title.:80)s`), maths (`%(epoch-3600)d`), dates
  (`%(timestamp>%Y-%m-%d)s`), alternates and defaults (`%(width,height|unknown)s`),
  replacements (`%(height&{}p|)s`) and every conversion (`diuoxXeEfFgGcrsa` plus
  upstream's `ljhqBUDS`), with Python-style width/precision formatting.
  `--output-na-placeholder TEXT` changes the default `NA`. Literal `%%` is supported.
  Metadata is sanitized like upstream (full-width look-alikes for reserved
  characters); `--restrict-filenames`, `--windows-filenames`/`--no-windows-filenames`
  and `--trim-filenames LENGTH` follow upstream. Metadata cannot introduce
  directories or `..` components; literal template paths can.
- `-o TYPE:TEMPLATE` for `subtitle`, `infojson` and `pl_infojson` files and
  `-P [TYPE:]PATH` output directories, including `temp:` with `[MoveFiles]`, follow upstream.
- `-x` / `--extract-audio` supports `best`, MP3, AAC, M4A, Opus, Vorbis,
  FLAC, ALAC and WAV through explicitly configured local FFmpeg/FFprobe.
  `--audio-format` accepts ordered `SOURCE>TARGET/TARGET` rules;
  `--audio-quality` accepts VBR 0..10 or a bitrate such as `128K`.
  FFprobe defaults to the sibling of `--ffmpeg-location`, or use
  `--ffprobe-location PATH`. `-k` / `--keep-video` retains processing sources and
  intermediates. [Audio limits and publication behavior](docs/COMPATIBILITY.md#audio-extraction)
  are explicit; no tool discovery or installation is performed.
- `--remux-video FORMAT` copies streams into a new container;
  `--recode-video FORMAT` re-encodes with FFmpeg defaults (AVI uses Xvid).
  Both accept ordered source/target mappings. Audio extraction runs first when
  combined; recode takes precedence when both video options are supplied.
  Same-extension/unmatched mappings skip FFmpeg. Container/codec incompatibility
  remains an error. See [video processing limits](docs/COMPATIBILITY.md#video-remuxing-and-conversion).
- Same-directory temporary files, cleanup on transfer failure, and no overwrite
  of existing destinations. A failed entry or URL sets a nonzero exit status while later
  entries and URLs still run. JSON, screen and progress lines go to stdout; warnings and errors go to stderr.
- Bounded download retries for selected transient HTTP statuses,
  connection/timeouts, and interrupted bodies. Interrupted transfers can resume
  within the same invocation with a strong ETag, known length, and validated range.
  `-R/--retries` and `--fragment-retries` set additional attempts (default 10,
  `infinite` allowed) with upstream's `Got error ... Retrying` messages,
  `--retry-sleep` delays and `--skip-unavailable-fragments`/
  `--abort-on-unavailable-fragments`. Transient HTTP failures honor
  a single `Retry-After` delay-seconds or preferred HTTP-date value, capped at
  60 seconds.
- Upstream-style download progress lines on stdout (`--newline`, `--no-progress`,
  `--progress`, `--progress-delta`).
  Library observers can cancel at transfer checkpoints and during retry waits,
  cleaning up the private temporary file before publication.
- Opt-in whole-extraction retries with `--extractor-retries N` (default 0, maximum
  255), independent of download retries. Transient page/source request failures
  and interrupted HTML reads restart extraction before any output is emitted,
  using the same bounded `Retry-After` policy.

Other HTML pages, general YouTube support, broader DASH/HLS delivery, site playlists, broader
authentication, persistent resume, broader subtitle acquisition and plugins
remain unimplemented (`--use-postprocessor` supports only `Exec`). Format selection
and sorting follow ports of upstream's `build_format_selector` and `FormatSorter`.
Unsupported CLI flags are errors. Filename handling is basic and does not yet
implement yt-dlp's platform-specific filename compatibility rules.

Direct extraction makes a streaming GET and inspects headers without consuming
the body. HTML5 extraction reads at most 2 MiB of uncompressed UTF-8/ASCII HTML,
then probes every declared media source. Extraction must succeed for every entry.
All entries remaining after archive filtering must select successfully before any
media is downloaded or JSON emitted. JavaScript
players, lazy-loading attributes, and embedded site players are not supported by the generic extractor. A download makes another GET. There is a 15-second connection timeout
and a five-minute total timeout per request. Simulation and JSON extraction still make
network requests. MIME identification is not media-container verification.

The library exposes `YoutubeDl`, `ExtractResult`, `PlaylistInfo`, `VideoInfo`,
`VideoFormat`, `StreamKind`, `Fragment`, `ByteRange`, `HlsAes128`, `DownloadOptions`, `DownloadProgress`, `DownloadPhase`,
`ExtractionOptions`, `YoutubeOptions`, and an `Extractor` interface.
`YoutubeDl::with_youtube_options(YoutubeOptions { js_runtime: Some(path), ..Default::default() })`
configures optional YouTube signature/n solving; `YoutubeDl::new()` keeps it disabled.
`include_translated_subtitles: true` additionally exposes translated manual captions
in `automatic_captions`; the CLI enables this with `--write-auto-subs` or `--list-subs`.
ASR translations are discovered independently, without fetching their payloads.
`YoutubeDl::with_options(YoutubeOptions::default(), HttpOptions {
user_agent: Some("My media client".into()), ..Default::default()
})` configures HTTP defaults; `HttpOptions.headers` holds name/value pairs.
`HttpOptions::from_cli(user_agent, referer, &headers)` parses repeated FIELD:VALUE
strings with the same normalization and validation as the binary.
`CookieJar::load(path)` and `save(path)` expose explicit Netscape persistence;
`HttpOptions.cookies` accepts a shared `Arc<CookieJar>`, or the client creates a
fresh in-memory jar. `YoutubeDl::cookie_jar()` returns it for reuse or saving.
`HttpOptions.proxy` accepts an explicit URL or empty string for direct routing;
`None` preserves the existing reqwest environment/system routing policy.
`HttpOptions::validate()` checks configuration without network/DNS requests.
Cookie files are credential state: an explicit `--cookies` path is updated after
requests even in JSON/listing/simulation modes and after extraction failures.
`YoutubeDl::new()?.with_ffmpeg(path)` configures optional local merging.
`extract_result_unselected` / `extract_result_unselected_with_options` expose all
eligible alternatives without library selection; `select_result` applies configured
selection atomically before downloading.
`output::render_template_with_options` accepts `TemplateOptions` for the missing-value
placeholder; `render_template` keeps the default `NA`.
`output::render_formats` renders their inventory, while `validate_format_selector`
checks supported syntax without network or availability checks.
Use `extract_result` or
`extract_result_with_format(url, Some("1"))` for video/playlist results. A format
ID applies to every playlist entry independently; each must offer that ID.
`PlaylistSelection::new(items, start, end, reverse)` parses item selection;
`apply` filters/reorders fully extracted playlist entries and annotates their
original index/count and selected order/count. Single videos are unaffected.
Library extraction methods do not implicitly apply CLI playlist options.
`SubtitleOptions::new(languages, format)?.select(&info.subtitles)` returns selected
subtitle metadata and warnings. `select_sources(&info.subtitles,
&info.automatic_captions, write_normal, write_automatic)` selects enabled sources
with manual precedence. `subtitle_paths` plans sanitized sidecar names;
`download_subtitle_with_options` / `download_subtitle_with_progress` copy one
resource with the normal retry/cancellation policy. Selection, writing and archive
recording remain explicit library operations.
`DownloadArchive::load`, `contains` and `record` expose persistence separately;
library callers choose when to filter and record, after successful publication.
`archive_id` returns the supported lowercase extractor-key plus case-sensitive ID.
The existing `extract_info` methods accept single videos and reject playlists.
`--dump-json` emits one video object per entry.
`download_with_options(info, path, DownloadOptions { retries: 0 })` disables
download retries for a library call; `download` uses the default policy.
`download_with_progress(info, path, options, callback)` reports one-based attempts,
stored bytes, optional total bytes, and the current phase. Fragment downloads also report one-based fragment indexes
and counts, with cumulative assembled bytes and a per-fragment attempt number. Return
`std::ops::ControlFlow::Continue(())` to continue or `Break(())` to cancel.
Cancellation is cooperative: it cannot interrupt an in-flight blocking request,
read, or filesystem operation. See the compatibility inventory for checkpoints.
`extract_result_with_options(url, None, ExtractionOptions { retries: 2 })` enables
whole-extraction retries; existing extraction methods keep retries disabled. See
[the roadmap](docs/ROADMAP.md) for architecture and upcoming milestones.
The [compatibility inventory](docs/COMPATIBILITY.md) records the exact subset and
the saved-fixture upstream comparison command.

To verify locally:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
python3 -m unittest discover -s tools -p 'test_*.py' -v
```

The HTTP integration tests bind localhost sockets and need an environment that
allows them. CI is configured for Linux, macOS, and Windows; tests use no live sites.

Upstream tracking is pinned in [upstream/manifest.json](upstream/manifest.json).
To produce an agent-readable change report:

```sh
git clone https://github.com/yt-dlp/yt-dlp.git .upstream/yt-dlp
python3 tools/upstream_report.py .upstream/yt-dlp > upstream-report.json
```

Use a full clone so the baseline exists. Fetch newer commits explicitly before
subsequent reports. The report is read-only and includes changed files, commits,
affected Rust components, and unmapped files. It does not port code or advance
the baseline. [The upstream workflow](docs/UPSTREAM.md) describes the future
agent update loop. No recurring agent or automatic merge is enabled yet.

Licensed under the Unlicense, matching this repository's original license.

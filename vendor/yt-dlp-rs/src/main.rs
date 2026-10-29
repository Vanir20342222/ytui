use clap::{CommandFactory, FromArgMatches, Parser};
use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{
    ops::ControlFlow,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use yt_dlp_rs::{
    AudioOptions, CookieJar, DownloadArchive, DownloadOptions, DownloadPhase, EntrySelection,
    ExtractResult, ExtractionOptions, HttpOptions, InfoJsonWrite, MatchFilters, MatchOutcome,
    PlaylistCounts, PlaylistSelection, PostprocessOptions, SubtitleConversion, SubtitleEmbedding,
    SubtitleFile, SubtitleOptions, ThumbnailConversion, VideoMode, VideoOptions, YoutubeDl,
    YoutubeOptions,
    dates::DateRange,
    output::{
        ExecCommand, ExecCommands, OutputTemplates, PrintTemplate, PrintToFile, Stage, StagePaths,
        TemplateOptions,
    },
    render_automatic_captions, render_subtitles, subtitle_paths, validate_format_selector,
    write_info_json_with,
};

// Upstream `write_string` encodes every message with `--encoding`; these
// shadow the standard printing macros for the rest of the binary.
macro_rules! println {
    () => {
        yt_dlp_rs::output::encoding::write_message(false, "\n")
    };
    ($($arg:tt)*) => {
        yt_dlp_rs::output::encoding::write_message(false, &format!("{}\n", format_args!($($arg)*)))
    };
}
macro_rules! eprintln {
    () => {
        yt_dlp_rs::output::encoding::write_message(true, "\n")
    };
    ($($arg:tt)*) => {
        yt_dlp_rs::output::encoding::write_message(true, &format!("{}\n", format_args!($($arg)*)))
    };
}

/// `-q`: suppress informational `[...]` screen lines (upstream `to_screen`).
/// `--compat-options allow-unsafe-ext` (upstream `_UnsafeExtensionError._enabled = False`).
static ALLOW_UNSAFE_EXT: AtomicBool = AtomicBool::new(false);
static QUIET: AtomicBool = AtomicBool::new(false);
/// `--no-warnings`: suppress `WARNING:` lines.
static NO_WARNINGS: AtomicBool = AtomicBool::new(false);
/// Upstream `validate_options` warnings, ranked in upstream's order and
/// written together after the `--verbose` header (`YoutubeDL.__init__`).
static VALIDATION_WARNINGS: std::sync::Mutex<Vec<(u8, String)>> = std::sync::Mutex::new(Vec::new());

fn validation_warning(rank: u8, text: String) {
    VALIDATION_WARNINGS
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .push((rank, text));
}

/// Upstream `logtostderr` (`-o -`): screen and progress output use stderr.
static LOG_TO_STDERR: AtomicBool = AtomicBool::new(false);

/// Upstream `_out_files.out`: stdout, or stderr with `logtostderr`.
fn out_stream() -> Box<dyn Write> {
    use yt_dlp_rs::output::encoding::{EncodingWriter, output};
    if LOG_TO_STDERR.load(Ordering::Relaxed) {
        Box::new(EncodingWriter::new(io::stderr().lock(), output()))
    } else {
        Box::new(EncodingWriter::new(io::stdout().lock(), output()))
    }
}

/// Upstream `to_screen`: stdout, or stderr with `logtostderr`; quiet mode
/// suppresses it unless `--verbose`, which writes it to stderr.
macro_rules! screen {
    ($($arg:tt)*) => {{
        let quiet = QUIET.load(Ordering::Relaxed);
        if !quiet || yt_dlp_rs::screen::verbose() {
            if quiet || LOG_TO_STDERR.load(Ordering::Relaxed) {
                eprintln!($($arg)*);
            } else {
                println!($($arg)*);
            }
        }
    }};
}

/// The last `ERROR:` line: upstream's raised `DownloadError` carries it.
static LAST_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Upstream `report_error`: an `ERROR:` line on stderr, remembered for the
/// `--load-info-json` fallback.
macro_rules! report_error {
    ($($arg:tt)*) => {{
        let line = format!($($arg)*);
        eprintln!("{line}");
        *LAST_ERROR.lock().unwrap() = Some(line);
    }};
}

/// Upstream `report_warning`: `--no-warnings` hides warnings, except as
/// `[debug] ` lines with `--verbose`.
macro_rules! warning {
    ($($arg:tt)*) => {
        if !NO_WARNINGS.load(Ordering::Relaxed) {
            eprintln!($($arg)*);
        } else if yt_dlp_rs::screen::verbose() {
            eprintln!("[debug] {}", format!($($arg)*));
        }
    };
}

fn flush_validation_warnings() {
    let mut warnings = std::mem::take(
        &mut *VALIDATION_WARNINGS
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()),
    );
    warnings.sort_by_key(|(rank, _)| *rank);
    for (_, text) in warnings {
        warning!("WARNING: {text}");
    }
}

#[derive(Parser)]
#[command(
    version,
    args_override_self = true,
    about = "Native Rust yt-dlp adaptation (direct HTTP, HTML5 and HLS/DASH VOD)"
)]
struct Args {
    urls: Vec<String>,
    /// Read URLs from FILE ("-" for stdin), one per line; "#", ";" and "]" lines are comments
    #[arg(short = 'a', long, value_name = "FILE", overrides_with_all = ["batch_file", "no_batch_file"])]
    batch_file: Option<String>,
    /// Do not read URLs from a batch file (default)
    #[arg(long, overrides_with = "batch_file")]
    no_batch_file: bool,
    /// Print one JSON info object per video (including playlist entries) without downloading
    #[arg(short = 'j', long)]
    dump_json: bool,
    /// Print one JSON object per URL; playlists include their selected entries (implies simulation)
    #[arg(short = 'J', long)]
    dump_single_json: bool,
    /// Activate quiet mode: suppress informational messages
    #[arg(short = 'q', long, overrides_with_all = ["quiet", "no_quiet"])]
    quiet: bool,
    /// Deactivate quiet mode (default unless printing or JSON output imply it)
    #[arg(long, overrides_with_all = ["quiet", "no_quiet"])]
    no_quiet: bool,
    /// Ignore warnings
    #[arg(long)]
    no_warnings: bool,
    /// Print various debugging information
    #[arg(short = 'v', long)]
    verbose: bool,
    /// Print downloaded pages encoded using base64 to debug problems (very verbose)
    #[arg(long)]
    dump_pages: bool,
    /// Write downloaded intermediary pages to files in the current directory to debug problems
    #[arg(long)]
    write_pages: bool,
    /// Display sent and read HTTP traffic
    #[arg(long)]
    print_traffic: bool,
    /// Enable a postprocessor by NAME with ARGS (NAME=VALUE pairs separated by ";", including "when"); natively only Exec (exec_cmd=CMD)
    #[arg(long, value_name = "NAME[:ARGS]")]
    use_postprocessor: Vec<String>,
    /// Resume partially downloaded files/fragments (default)
    #[arg(short = 'c', long = "continue", overrides_with_all = ["continue_partial", "no_continue"])]
    continue_partial: bool,
    /// Do not resume partially downloaded files (restart from the beginning)
    #[arg(long, overrides_with_all = ["continue_partial", "no_continue"])]
    no_continue: bool,
    /// Use .part files instead of writing directly into the output file (default)
    #[arg(long, overrides_with_all = ["part", "no_part"])]
    part: bool,
    /// Do not use .part files - write directly into the output file
    #[arg(long, overrides_with_all = ["part", "no_part"])]
    no_part: bool,
    /// Output progress as new lines
    #[arg(long)]
    newline: bool,
    /// Do not print progress
    #[arg(long, overrides_with_all = ["no_progress", "progress"])]
    no_progress: bool,
    /// Show progress, even in quiet mode
    #[arg(long, overrides_with_all = ["no_progress", "progress"])]
    progress: bool,
    /// Time between progress output (default: 0)
    #[arg(long, value_name = "SECONDS", allow_hyphen_values = true)]
    progress_delta: Option<f64>,
    /// Extract metadata without writing media
    #[arg(short = 's', long, overrides_with_all = ["simulate", "no_simulate"])]
    simulate: bool,
    /// Download even when printing, JSON or listing options would simulate
    #[arg(long, overrides_with_all = ["simulate", "no_simulate"])]
    no_simulate: bool,
    /// Print a video field list or output template per video, optionally prefixed with video: (implies simulation; repeatable)
    #[arg(
        short = 'O',
        long = "print",
        value_name = "[WHEN:]TEMPLATE",
        allow_hyphen_values = true
    )]
    print: Vec<String>,
    /// Append a video-stage print template to a file named by an output template (repeatable; does not imply simulation)
    #[arg(
        long,
        num_args = 2,
        value_names = ["[WHEN:]TEMPLATE", "FILE"],
        allow_hyphen_values = true
    )]
    print_to_file: Vec<String>,
    #[arg(short = 'g', long, hide = true)]
    get_url: bool,
    #[arg(short = 'e', long, hide = true)]
    get_title: bool,
    #[arg(long, hide = true)]
    get_id: bool,
    #[arg(long, hide = true)]
    get_filename: bool,
    #[arg(long, hide = true)]
    get_thumbnail: bool,
    #[arg(long, hide = true)]
    get_description: bool,
    #[arg(long, hide = true)]
    get_duration: bool,
    #[arg(long, hide = true)]
    get_format: bool,
    /// List available formats (selection still applies with -j or -s; no downloads)
    #[arg(short = 'F', long = "list-formats")]
    list_formats: bool,
    /// Present the list of formats in a more readable table (default)
    #[arg(long, hide = true, overrides_with = "list_formats_old")]
    list_formats_as_table: bool,
    /// Present the list of formats in the older layout
    #[arg(
        long,
        alias = "no-list-formats-as-table",
        overrides_with = "list_formats_as_table"
    )]
    list_formats_old: bool,
    /// List available subtitle languages/formats without downloading
    #[arg(long)]
    list_subs: bool,
    /// Use the Last-modified header to set the file modification time
    #[arg(long, overrides_with_all = ["mtime", "no_mtime"])]
    mtime: bool,
    /// Do not use the Last-modified header to set the file modification time (default)
    #[arg(long, overrides_with_all = ["mtime", "no_mtime"])]
    no_mtime: bool,
    /// Write the best available thumbnail image
    #[arg(long, action = clap::ArgAction::Count)]
    write_thumbnail: u8,
    /// Do not write thumbnail images (default)
    #[arg(long, action = clap::ArgAction::Count)]
    no_write_thumbnail: u8,
    /// Write all thumbnail images
    #[arg(long, action = clap::ArgAction::Count)]
    write_all_thumbnails: u8,
    /// List available thumbnails without downloading
    #[arg(long)]
    list_thumbnails: bool,
    /// Write selected subtitle sidecars before downloading media
    #[arg(long, alias = "write-srt", overrides_with = "no_write_subs")]
    write_subs: bool,
    /// Disable subtitle sidecar writing
    #[arg(long, alias = "no-write-srt", overrides_with = "write_subs")]
    no_write_subs: bool,
    /// Write selected automatically generated caption sidecars
    #[arg(
        long,
        alias = "write-automatic-subs",
        overrides_with = "no_write_auto_subs"
    )]
    write_auto_subs: bool,
    /// Disable automatic caption sidecar writing
    #[arg(
        long,
        alias = "no-write-automatic-subs",
        overrides_with = "write_auto_subs"
    )]
    no_write_auto_subs: bool,
    /// Comma-separated literal subtitle languages, all and -exclusions (no regex)
    #[arg(
        long,
        alias = "srt-langs",
        value_name = "LANGS",
        allow_hyphen_values = true
    )]
    sub_langs: Vec<String>,
    /// Ordered subtitle extension preferences separated by / (default best)
    #[arg(long, default_value = "best", value_name = "FORMAT")]
    sub_format: String,
    /// Execute a command, optionally prefixed with when (video, before_dl, post_process, after_move (default), after_video); fields allow d/i/f/q conversions
    #[arg(long, value_name = "[WHEN:]CMD", allow_hyphen_values = true)]
    exec: Vec<String>,
    /// Remove any previously defined --exec
    #[arg(long, action = clap::ArgAction::Count)]
    no_exec: u8,
    #[arg(long, hide = true, value_name = "CMD", allow_hyphen_values = true)]
    exec_before_download: Vec<String>,
    #[arg(long, hide = true, action = clap::ArgAction::Count)]
    no_exec_before_download: u8,
    /// Embed metadata (title, date, description, URL, artist, genre, languages) with ffmpeg
    #[arg(long, alias = "add-metadata", overrides_with_all = ["embed_metadata", "no_embed_metadata"])]
    embed_metadata: bool,
    /// Do not embed metadata (default)
    #[arg(long, alias = "no-add-metadata", overrides_with_all = ["embed_metadata", "no_embed_metadata"])]
    no_embed_metadata: bool,
    /// Add chapter markers to the video file (implied by --embed-metadata)
    #[arg(long, alias = "add-chapters", overrides_with_all = ["embed_chapters", "no_embed_chapters"])]
    embed_chapters: bool,
    /// Do not add chapter markers
    #[arg(long, alias = "no-add-chapters", overrides_with_all = ["embed_chapters", "no_embed_chapters"])]
    no_embed_chapters: bool,
    /// Parse additional metadata like title/artist from other fields (default WHEN: pre_process)
    #[arg(long, value_name = "[WHEN:]FROM:TO", allow_hyphen_values = true)]
    parse_metadata: Vec<String>,
    /// Replace text in metadata fields using a regex (default WHEN: pre_process)
    #[arg(long, value_names = ["[WHEN:]FIELDS", "REGEX", "REPLACE"], num_args = 3, allow_hyphen_values = true)]
    replace_in_metadata: Vec<String>,
    #[arg(long, hide = true, value_name = "FORMAT")]
    metadata_from_title: Option<String>,
    /// Embed the infojson as an attachment to mkv/mka video files
    #[arg(long, overrides_with = "no_embed_info_json")]
    embed_info_json: bool,
    /// Do not embed the infojson as an attachment to the video file
    #[arg(long, overrides_with = "embed_info_json")]
    no_embed_info_json: bool,
    /// Write metadata to the video file's xattrs (using Dublin Core and XDG standards)
    #[arg(long, alias = "xattr")]
    xattrs: bool,
    /// SponsorBlock categories to create chapters for, separated by commas (all, default (=all), -CATEGORY to exclude)
    #[arg(long, value_name = "CATS")]
    sponsorblock_mark: Vec<String>,
    /// SponsorBlock categories to remove from the video file (default = all,-filler; poi_highlight and chapter are not available)
    #[arg(long, value_name = "CATS")]
    sponsorblock_remove: Vec<String>,
    /// An output template for the title of the SponsorBlock chapters created by --sponsorblock-mark
    #[arg(
        long,
        value_name = "TEMPLATE",
        default_value = "[SponsorBlock]: %(category_names)l"
    )]
    sponsorblock_chapter_title: String,
    /// Disable both --sponsorblock-mark and --sponsorblock-remove
    #[arg(long)]
    no_sponsorblock: bool,
    /// SponsorBlock API location
    #[arg(long, value_name = "URL", default_value = "https://sponsor.ajay.app")]
    sponsorblock_api: String,
    /// Remove chapters whose title matches REGEX ("*START-END" ranges are parsed like upstream, which ignores them); repeatable
    #[arg(long, value_name = "REGEX")]
    remove_chapters: Vec<String>,
    /// Do not remove any chapters (default); clears earlier --remove-chapters
    #[arg(long)]
    no_remove_chapters: bool,
    /// Force keyframes at cuts when removing or splitting chapters (re-encodes)
    #[arg(long, overrides_with_all = ["force_keyframes_at_cuts", "no_force_keyframes_at_cuts"])]
    force_keyframes_at_cuts: bool,
    /// Do not force keyframes around the chapters before cutting (default)
    #[arg(long, overrides_with_all = ["force_keyframes_at_cuts", "no_force_keyframes_at_cuts"])]
    no_force_keyframes_at_cuts: bool,
    /// Concatenate playlist entries' videos: never, always, or multi_video (default; only multi-video results, which this port's extractors do not produce)
    #[arg(long, value_name = "POLICY", default_value = "multi_video", value_parser = ["never", "always", "multi_video"])]
    concat_playlist: String,
    /// Split video into multiple files based on internal chapters (the "chapter" output template)
    #[arg(long, alias = "split-tracks", overrides_with_all = ["split_chapters", "no_split_chapters"])]
    split_chapters: bool,
    /// Do not split video based on chapters (default)
    #[arg(long, alias = "no-split-tracks", overrides_with_all = ["split_chapters", "no_split_chapters"])]
    no_split_chapters: bool,
    /// Embed the best written thumbnail as cover art (mp3, mkv/mka, mp4/m4a/m4v/mov; implies writing it)
    #[arg(long, overrides_with_all = ["embed_thumbnail", "no_embed_thumbnail"])]
    embed_thumbnail: bool,
    /// Do not embed thumbnails (default)
    #[arg(long, overrides_with_all = ["embed_thumbnail", "no_embed_thumbnail"])]
    no_embed_thumbnail: bool,
    /// Convert written thumbnails to jpg, png or webp (rules like webp>jpg/png); none disables conversion
    #[arg(long, value_name = "FORMAT", overrides_with = "convert_thumbnails")]
    convert_thumbnails: Option<String>,
    /// Embed selected subtitles after audio/video processing (implies manual writing unless auto writing is enabled)
    #[arg(long, overrides_with = "no_embed_subs")]
    embed_subs: bool,
    /// Disable subtitle embedding
    #[arg(long, overrides_with = "embed_subs")]
    no_embed_subs: bool,
    /// Convert written subtitles to srt, vtt, ass or lrc; none disables conversion
    #[arg(long, aliases = ["convert-sub", "convert-subtitles"], value_name = "FORMAT", overrides_with = "convert_subs")]
    convert_subs: Option<String>,
    /// Skip media downloads while allowing requested subtitle and info JSON sidecars
    #[arg(long, alias = "no-download")]
    skip_download: bool,
    /// Process the video or playlist information in FILE (written with --write-info-json) instead of URLs
    #[arg(long, alias = "load-info", value_name = "FILE")]
    load_info_json: Option<String>,
    /// Write an internet shortcut file for the platform (.webloc on macOS, .desktop on Linux, .url otherwise)
    #[arg(long)]
    write_link: bool,
    /// Write a .url Windows internet shortcut
    #[arg(long)]
    write_url_link: bool,
    /// Write a .webloc macOS internet shortcut
    #[arg(long)]
    write_webloc_link: bool,
    /// Write a .desktop Linux internet shortcut
    #[arg(long)]
    write_desktop_link: bool,
    /// Write cleaned video and playlist metadata to .info.json files (existing files are kept)
    #[arg(long, overrides_with = "no_write_info_json")]
    write_info_json: bool,
    /// Write the video description to a .description file
    #[arg(long, overrides_with_all = ["write_description", "no_write_description"])]
    write_description: bool,
    /// Do not write the video description (default)
    #[arg(long, overrides_with_all = ["write_description", "no_write_description"])]
    no_write_description: bool,
    /// Write playlist metadata files with --write-info-json/--write-description (default)
    #[arg(long, overrides_with_all = ["write_playlist_metafiles", "no_write_playlist_metafiles"])]
    write_playlist_metafiles: bool,
    /// Do not write playlist metadata files
    #[arg(long, overrides_with_all = ["write_playlist_metafiles", "no_write_playlist_metafiles"])]
    no_write_playlist_metafiles: bool,
    /// Disable info JSON writing
    #[arg(long, overrides_with = "write_info_json")]
    no_write_info_json: bool,
    /// Retrieve video comments to be placed in the infojson (implies --write-info-json unless JSON is printed)
    #[arg(long, alias = "get-comments", overrides_with = "no_write_comments")]
    write_comments: bool,
    /// Do not retrieve video comments unless the extraction is known to be quick (default)
    #[arg(long, alias = "no-get-comments", overrides_with = "write_comments")]
    no_write_comments: bool,
    /// Format ID or indexed/grouped selector with filters, + pairs and / fallbacks
    #[arg(short = 'f', long = "format")]
    format: Option<String>,
    /// Number of retries (default is 10), or "infinite"
    #[arg(
        short = 'R',
        long,
        default_value = "10",
        value_name = "RETRIES",
        allow_hyphen_values = true
    )]
    retries: String,
    /// Number of retries for a fragment (default is 10), or "infinite" (DASH and hlsnative)
    #[arg(
        long,
        default_value = "10",
        value_name = "RETRIES",
        allow_hyphen_values = true
    )]
    fragment_retries: String,
    /// Time to sleep between retries, optionally prefixed by http (default) or fragment: a number, linear=START[:END[:STEP=1]] or exp=START[:END[:BASE=2]]
    #[arg(long, value_name = "[TYPE:]EXPR", allow_hyphen_values = true)]
    retry_sleep: Vec<String>,
    /// Skip unavailable fragments for DASH and hlsnative downloads (default)
    #[arg(long, alias = "no-abort-on-unavailable-fragments", overrides_with_all = ["skip_unavailable_fragments", "abort_on_unavailable_fragments"])]
    skip_unavailable_fragments: bool,
    /// Abort download if a fragment is unavailable
    #[arg(long, alias = "no-skip-unavailable-fragments", overrides_with_all = ["skip_unavailable_fragments", "abort_on_unavailable_fragments"])]
    abort_on_unavailable_fragments: bool,
    /// Retry the entire extraction after transient network failures (0 disables retries)
    #[arg(long, default_value_t = 0)]
    extractor_retries: u8,
    /// Seconds to wait for connections and each read/write before giving up
    #[arg(long, value_name = "SECONDS", allow_hyphen_values = true)]
    socket_timeout: Option<f64>,
    /// Client-side IP address to bind to
    #[arg(long, value_name = "IP", overrides_with_all = ["source_address", "force_ipv4", "force_ipv6"])]
    source_address: Option<String>,
    /// Make all connections via IPv4
    #[arg(short = '4', long, overrides_with_all = ["source_address", "force_ipv4", "force_ipv6"])]
    force_ipv4: bool,
    /// Make all connections via IPv6
    #[arg(short = '6', long, overrides_with_all = ["source_address", "force_ipv4", "force_ipv6"])]
    force_ipv6: bool,
    /// Suppress HTTPS certificate validation
    #[arg(long)]
    no_check_certificates: bool,
    /// HTTP/HTTPS/SOCKS proxy URL; an empty value forces direct connections
    #[arg(long, value_name = "URL", overrides_with = "proxy")]
    proxy: Option<String>,
    /// Read and save scoped cookies in Netscape format (including metadata modes)
    #[arg(long, value_name = "FILE", overrides_with_all = ["no_cookies", "cookies"])]
    cookies: Option<PathBuf>,
    /// Disable cookie-file loading and saving (in-memory response cookies remain enabled)
    #[arg(long, overrides_with = "cookies")]
    no_cookies: bool,
    /// The name of the browser to load cookies from (firefox natively), with an optional KEYRING, PROFILE name/path and Firefox CONTAINER ("none" for no container)
    #[arg(
        long,
        value_name = "BROWSER[+KEYRING][:PROFILE][::CONTAINER]",
        overrides_with_all = ["cookies_from_browser", "no_cookies_from_browser"]
    )]
    cookies_from_browser: Option<String>,
    /// Do not load cookies from browser (default)
    #[arg(long, overrides_with_all = ["cookies_from_browser", "no_cookies_from_browser"])]
    no_cookies_from_browser: bool,
    /// Custom HTTP header FIELD:VALUE (repeatable; last name wins)
    #[arg(long, alias = "add-header", value_name = "FIELD:VALUE")]
    add_headers: Vec<String>,
    /// Override the HTTP User-Agent
    #[arg(long, value_name = "UA", overrides_with = "user_agent")]
    user_agent: Option<String>,
    /// Default HTTP Referer (extractor request headers can override it)
    #[arg(long, value_name = "URL", overrides_with = "referer")]
    referer: Option<String>,
    /// Explicit path to Deno 2.9.7 for YouTube player challenges (no runtime discovery or installation)
    #[arg(long, value_name = "PATH")]
    youtube_js_runtime: Option<PathBuf>,
    /// Extract audio with local ffmpeg/ffprobe after downloading
    #[arg(short = 'x', long)]
    extract_audio: bool,
    /// Audio target or ordered SOURCE>TARGET/TARGET rules
    #[arg(long, default_value = "best", value_name = "FORMAT")]
    audio_format: String,
    /// VBR quality 0..10 or bitrate such as 128K
    #[arg(
        long,
        default_value = "5",
        value_name = "QUALITY",
        allow_hyphen_values = true
    )]
    audio_quality: String,
    /// Don't load any more configuration files except those given to --config-locations (Alias: --no-config)
    #[arg(long, visible_alias = "no-config")]
    ignore_config: bool,
    /// Do not load any custom configuration files (default); inside a configuration file, ignore its previous --config-locations
    #[arg(long)]
    no_config_locations: bool,
    /// Location of the main configuration file; either the path to the config or its containing directory ("-" for stdin)
    #[arg(long, value_name = "PATH")]
    config_locations: Vec<String>,
    /// Sort the formats by the fields given (comma-separated; later values take precedence)
    #[arg(
        short = 'S',
        long,
        value_name = "SORTORDER",
        allow_hyphen_values = true
    )]
    format_sort: Vec<String>,
    /// Disregard previous user specified sort order and reset to the default
    #[arg(long)]
    format_sort_reset: bool,
    /// Force user specified sort order to have precedence over all fields (Alias: --S-force)
    #[arg(
        long,
        visible_alias = "S-force",
        overrides_with = "no_format_sort_force"
    )]
    format_sort_force: bool,
    /// Some fields have precedence over the user specified sort order (default)
    #[arg(long, overrides_with = "format_sort_force")]
    no_format_sort_force: bool,
    /// Prefer video formats with free containers over non-free ones of the same quality
    #[arg(long, overrides_with = "no_prefer_free_formats")]
    prefer_free_formats: bool,
    /// Don't give any special preference to free containers (default)
    #[arg(long, overrides_with = "prefer_free_formats")]
    no_prefer_free_formats: bool,
    /// Automatically correct known faults of the file: never (do nothing), warn
    /// (only emit a warning), detect_or_warn (the default; fix the file if we can,
    /// warn otherwise), force (try fixing even if the file already exists)
    #[arg(long, value_name = "POLICY", value_parser = ["never", "ignore", "warn", "detect_or_warn", "force"])]
    fixup: Option<String>,
    /// Use the mpegts container for HLS videos (no MPEG-TS fixup)
    #[arg(long, overrides_with = "no_hls_use_mpegts")]
    hls_use_mpegts: bool,
    /// Do not use the mpegts container for HLS videos (default)
    #[arg(long, overrides_with = "hls_use_mpegts")]
    no_hls_use_mpegts: bool,
    /// Split HLS playlists into different formats at discontinuities such as ad breaks
    #[arg(long, overrides_with_all = ["hls_split_discontinuity", "no_hls_split_discontinuity"])]
    hls_split_discontinuity: bool,
    /// Do not split HLS playlists into different formats at discontinuities (default)
    #[arg(long, overrides_with_all = ["hls_split_discontinuity", "no_hls_split_discontinuity"])]
    no_hls_split_discontinuity: bool,
    /// Download only chapters matching the regex, or "*START-END" time
    /// ranges (negative values count from the end), through ffmpeg; may be
    /// repeated
    #[arg(long, value_name = "REGEX", allow_hyphen_values = true)]
    download_sections: Vec<String>,
    /// Containers that may be used when merging formats, separated by "/",
    /// e.g. "mp4/mkv"; ignored if no merge is required (currently supported:
    /// avi, flv, mkv, mov, mp4, webm)
    #[arg(long, value_name = "FORMAT")]
    merge_output_format: Option<String>,
    /// Remux media to FORMAT or ordered SOURCE>TARGET/TARGET rules (stream copy)
    #[arg(long, value_name = "FORMAT", overrides_with = "remux_video")]
    remux_video: Option<String>,
    /// Re-encode media to FORMAT or ordered SOURCE>TARGET/TARGET rules
    #[arg(long, value_name = "FORMAT", overrides_with = "recode_video")]
    recode_video: Option<String>,
    /// Overwrite post-processed files (default)
    #[arg(long, overrides_with_all = ["post_overwrites", "no_post_overwrites"])]
    post_overwrites: bool,
    /// Do not overwrite post-processed files
    #[arg(long, overrides_with_all = ["post_overwrites", "no_post_overwrites"])]
    no_post_overwrites: bool,
    /// Keep the intermediate video file on disk after post-processing
    #[arg(short = 'k', long, overrides_with = "no_keep_video")]
    keep_video: bool,
    /// Remove processing sources after success (default)
    #[arg(long, overrides_with = "keep_video")]
    no_keep_video: bool,
    /// Explicit ffprobe executable for audio inspection (otherwise beside ffmpeg)
    #[arg(long, value_name = "PATH")]
    ffprobe_location: Option<PathBuf>,
    /// Location of the ffmpeg binary; either the path to the binary or its
    /// containing directory (default: ffmpeg/ffprobe from PATH)
    #[arg(long, value_name = "PATH")]
    ffmpeg_location: Option<PathBuf>,
    /// Skip recorded extractor/video IDs and record only completed downloads
    #[arg(long, value_name = "FILE", overrides_with_all = ["download_archive", "no_download_archive"])]
    download_archive: Option<PathBuf>,
    /// Do not use an archive file (default)
    #[arg(long, overrides_with_all = ["download_archive", "no_download_archive"])]
    no_download_archive: bool,
    /// Stop when encountering a video that is in the download archive
    #[arg(long, overrides_with_all = ["break_on_existing", "no_break_on_existing"])]
    break_on_existing: bool,
    /// Do not stop at archived videos (default)
    #[arg(long, overrides_with_all = ["break_on_existing", "no_break_on_existing"])]
    no_break_on_existing: bool,
    /// Make --max-downloads and --break-on-existing reset per input URL
    #[arg(long, overrides_with_all = ["break_per_input", "no_break_per_input"])]
    break_per_input: bool,
    /// Let --max-downloads and --break-on-existing stop the whole queue (default)
    #[arg(long, overrides_with_all = ["break_per_input", "no_break_per_input"])]
    no_break_per_input: bool,
    /// Record archive entries even when simulating or skipping downloads
    #[arg(long, aliases = ["force-write-download-archive", "force-download-archive"])]
    force_write_archive: bool,
    /// Comma-separated playlist indices or inclusive START:STOP:STEP ranges
    #[arg(
        short = 'I',
        long,
        value_name = "ITEM_SPEC",
        allow_hyphen_values = true
    )]
    playlist_items: Option<String>,
    /// First playlist item (one-based, default 1; overridden by --playlist-items)
    #[arg(long, default_value_t = 1, allow_hyphen_values = true)]
    playlist_start: i64,
    /// Last playlist item, inclusive (-1 means through the end)
    #[arg(long, allow_hyphen_values = true)]
    playlist_end: Option<i64>,
    /// Reverse the selected playlist item order
    #[arg(long, overrides_with = "no_playlist_reverse")]
    playlist_reverse: bool,
    #[arg(long, hide = true, overrides_with = "playlist_reverse")]
    no_playlist_reverse: bool,
    /// Download playlist videos in random order
    #[arg(long)]
    playlist_random: bool,
    /// Process entries in the playlist as they are received. This disables n_entries, --playlist-random and --playlist-reverse
    #[arg(long, overrides_with = "no_lazy_playlist")]
    lazy_playlist: bool,
    /// Process videos in the playlist only after the entire playlist is parsed (default)
    #[arg(long, overrides_with = "lazy_playlist")]
    no_lazy_playlist: bool,
    /// Download only the video, if the URL refers to a video and a playlist
    #[arg(long, overrides_with = "yes_playlist")]
    no_playlist: bool,
    /// Download the playlist, if the URL refers to a video and a playlist
    #[arg(long, overrides_with = "no_playlist")]
    yes_playlist: bool,
    /// Do not extract a playlist's URL result entries; supported playlists
    /// carry fully extracted entries, which are processed either way
    #[arg(long, overrides_with = "no_flat_playlist")]
    flat_playlist: bool,
    /// Fully extract the videos of a playlist (default)
    #[arg(long, overrides_with = "flat_playlist")]
    no_flat_playlist: bool,
    /// Number of allowed failures until the rest of the playlist is skipped
    #[arg(long, value_name = "N", allow_hyphen_values = true)]
    skip_playlist_after_errors: Option<i64>,
    #[arg(long, hide = true)]
    id: bool,
    #[arg(long, hide = true)]
    print_json: bool,
    #[arg(long, hide = true)]
    all_subs: bool,
    #[arg(long, hide = true, default_value_t = 1, allow_hyphen_values = true)]
    autonumber_start: i64,
    #[arg(long, hide = true, allow_hyphen_values = true)]
    autonumber_size: Option<i64>,
    /// Placeholder for unavailable output-template metadata
    #[arg(long, default_value = "NA", value_name = "TEXT")]
    output_na_placeholder: String,
    /// Filename template: represented metadata with s/d/i/u/f/F conversions and %%
    #[arg(short, long, value_name = "[TYPES:]TEMPLATE")]
    output: Vec<String>,
    /// Do not overwrite any files
    #[arg(short = 'w', long, overrides_with_all = ["no_overwrites", "force_overwrites", "no_force_overwrites"])]
    no_overwrites: bool,
    /// Overwrite all video and metadata files
    #[arg(long, alias = "yes-overwrites", overrides_with_all = ["no_overwrites", "force_overwrites", "no_force_overwrites"])]
    force_overwrites: bool,
    /// Do not overwrite the video, but overwrite related files (default)
    #[arg(long, overrides_with_all = ["no_overwrites", "force_overwrites", "no_force_overwrites"])]
    no_force_overwrites: bool,
    /// Generic video filter: FIELD comparisons/presence joined by &; repeat for OR
    #[arg(long, value_name = "FILTER", allow_hyphen_values = true)]
    match_filters: Vec<String>,
    /// Discard earlier --match-filters
    #[arg(long, action = clap::ArgAction::Count)]
    no_match_filters: u8,
    /// Like --match-filters, but stop the download process when a video is rejected
    #[arg(long, value_name = "FILTER", allow_hyphen_values = true)]
    break_match_filters: Vec<String>,
    /// Discard earlier --break-match-filters
    #[arg(long, action = clap::ArgAction::Count)]
    no_break_match_filters: u8,
    /// Download only videos uploaded on this date: YYYYMMDD or (now|today|yesterday)[-N(day|week|month|year)[s]]
    #[arg(
        long,
        value_name = "DATE",
        allow_hyphen_values = true,
        overrides_with = "date"
    )]
    date: Option<String>,
    /// Download only videos uploaded on or before this date
    #[arg(
        long,
        value_name = "DATE",
        allow_hyphen_values = true,
        overrides_with = "datebefore"
    )]
    datebefore: Option<String>,
    /// Download only videos uploaded on or after this date
    #[arg(
        long,
        value_name = "DATE",
        allow_hyphen_values = true,
        overrides_with = "dateafter"
    )]
    dateafter: Option<String>,
    /// Download only videos suitable for the given age
    #[arg(
        long,
        value_name = "YEARS",
        allow_hyphen_values = true,
        overrides_with = "age_limit"
    )]
    age_limit: Option<i64>,
    #[arg(
        long,
        hide = true,
        value_name = "COUNT",
        allow_hyphen_values = true,
        overrides_with = "min_views"
    )]
    min_views: Option<i64>,
    #[arg(
        long,
        hide = true,
        value_name = "COUNT",
        allow_hyphen_values = true,
        overrides_with = "max_views"
    )]
    max_views: Option<i64>,
    #[arg(
        long,
        hide = true,
        value_name = "REGEX",
        allow_hyphen_values = true,
        overrides_with = "match_title"
    )]
    match_title: Option<String>,
    #[arg(
        long,
        hide = true,
        value_name = "REGEX",
        allow_hyphen_values = true,
        overrides_with = "reject_title"
    )]
    reject_title: Option<String>,
    #[arg(long, hide = true)]
    break_on_reject: bool,
    /// Abort after processing NUMBER videos (0 means no limit)
    #[arg(long, value_name = "NUMBER", allow_hyphen_values = true)]
    max_downloads: Option<i64>,
    /// Ignore download and postprocessing errors; a video whose postprocessing fails still counts as downloaded
    #[arg(short = 'i', long, action = clap::ArgAction::Count)]
    ignore_errors: u8,
    /// Continue with the next video on download errors (default)
    #[arg(long, action = clap::ArgAction::Count)]
    no_abort_on_error: u8,
    /// Abort downloading of further videos if an error occurs
    #[arg(long, alias = "no-ignore-errors", action = clap::ArgAction::Count)]
    abort_on_error: u8,
    /// Maximum download rate in bytes per second, e.g. 50K or 4.2M
    #[arg(short = 'r', long, aliases = ["rate-limit"], value_name = "RATE")]
    limit_rate: Option<String>,
    /// Whether to emit color codes ([STREAM:]POLICY); this port never emits them
    #[arg(long, value_name = "[STREAM:]POLICY")]
    color: Vec<String>,
    #[arg(long, alias = "no-colours", hide = true)]
    no_colors: bool,
    /// Remove some internal metadata such as filenames from the infojson (default)
    #[arg(long, alias = "clean-infojson", overrides_with = "no_clean_info_json")]
    clean_info_json: bool,
    /// Write all fields to the infojson
    #[arg(long, alias = "no-clean-infojson", overrides_with = "clean_info_json")]
    no_clean_info_json: bool,
    /// Extractor names to use separated by commas (regexes, "all", "default", "end", "-" to exclude)
    #[arg(long, alias = "ies", value_name = "NAMES")]
    use_extractors: Vec<String>,
    #[arg(long, hide = true)]
    force_generic_extractor: bool,
    /// Prefix for unqualified URLs ("error", "fixup_error"; search prefixes are unsupported)
    #[arg(long, value_name = "PREFIX")]
    default_search: Option<String>,
    /// Make sure formats are selected only from those that are actually downloadable
    #[arg(long, overrides_with_all = ["check_formats", "check_all_formats", "no_check_formats"])]
    check_formats: bool,
    /// Check all formats for whether they are actually downloadable
    #[arg(long, overrides_with_all = ["check_formats", "check_all_formats", "no_check_formats"])]
    check_all_formats: bool,
    /// Do not check that the formats are actually downloadable
    #[arg(long, overrides_with_all = ["check_formats", "check_all_formats", "no_check_formats"])]
    no_check_formats: bool,
    /// List all supported extractors and the URLs they would handle
    #[arg(long)]
    list_extractors: bool,
    /// Output descriptions of all supported extractors
    #[arg(long)]
    extractor_descriptions: bool,
    /// Number of fragments of a dash/hlsnative video that should be downloaded concurrently
    #[arg(
        short = 'N',
        long,
        value_name = "N",
        default_value_t = 1,
        allow_hyphen_values = true
    )]
    concurrent_fragments: i64,
    /// Keep downloaded fragments on disk after downloading is finished
    #[arg(long, overrides_with = "no_keep_fragments")]
    keep_fragments: bool,
    /// Delete downloaded fragments after downloading is finished (default)
    #[arg(long, overrides_with = "keep_fragments")]
    no_keep_fragments: bool,
    /// Downloader to use, optionally prefixed by protocols (http, ftp, m3u8, dash, rtmp): native or ffmpeg
    #[arg(long, alias = "external-downloader", value_name = "[PROTO:]NAME")]
    downloader: Vec<String>,
    /// Arguments for the downloader (NAME:ARGS; ffmpeg, ffmpeg_i[N], ffmpeg_o[N])
    #[arg(long, alias = "external-downloader-args", value_name = "NAME:ARGS")]
    downloader_args: Vec<String>,
    /// Give these arguments to the postprocessors (PP[+EXE][_i|_o[N]]:ARGS; Merger, ExtractAudio, VideoRemuxer, VideoConvertor, Metadata, EmbedSubtitle, EmbedThumbnail, SubtitlesConvertor, ThumbnailsConvertor, SplitChapters, FixupM4a, FixupM3u8) (Alias: --ppa)
    #[arg(long, visible_alias = "ppa", value_name = "NAME:ARGS")]
    postprocessor_args: Vec<String>,
    #[arg(long, hide = true, overrides_with = "hls_prefer_ffmpeg")]
    hls_prefer_native: bool,
    #[arg(long, hide = true, overrides_with = "hls_prefer_native")]
    hls_prefer_ffmpeg: bool,
    /// Template for progress outputs ([TYPES:]TEMPLATE; "download" (default), "postprocess", "-title" variants)
    #[arg(long, value_name = "[TYPES:]TEMPLATE")]
    progress_template: Vec<String>,
    /// Size of the download buffer (accepted; reads use a fixed native buffer)
    #[arg(long, value_name = "SIZE")]
    buffer_size: Option<String>,
    #[arg(long, hide = true, overrides_with = "no_resize_buffer")]
    resize_buffer: bool,
    #[arg(long, hide = true, overrides_with = "resize_buffer")]
    no_resize_buffer: bool,
    /// Number of times to retry on a file access error (accepted; not retried natively)
    #[arg(long, value_name = "RETRIES", default_value = "3")]
    file_access_retries: String,
    /// Minimum download rate below which throttling is assumed (accepted; not acted on)
    #[arg(long, value_name = "RATE")]
    throttled_rate: Option<String>,
    /// Wait for scheduled streams (accepted; live and upcoming streams are unsupported)
    #[arg(long, value_name = "MIN[-MAX]")]
    wait_for_video: Option<String>,
    #[arg(long, hide = true)]
    no_wait_for_video: bool,
    #[arg(long, hide = true, overrides_with = "no_live_from_start")]
    live_from_start: bool,
    #[arg(long, hide = true, overrides_with = "live_from_start")]
    no_live_from_start: bool,
    #[arg(long, hide = true, overrides_with = "no_mark_watched")]
    mark_watched: bool,
    #[arg(long, hide = true, overrides_with = "mark_watched")]
    no_mark_watched: bool,
    /// Self-updating is not supported; reinstall yt-dlp-rs instead
    #[arg(short = 'U', long)]
    update: bool,
    #[arg(long, hide = true)]
    no_update: bool,
    #[arg(long, hide = true, value_name = "[CHANNEL]@[TAG]")]
    update_to: Option<String>,
    /// Location in the filesystem where cached data is stored (this port caches nothing)
    #[arg(long, value_name = "DIR")]
    cache_dir: Option<String>,
    #[arg(long, hide = true)]
    no_cache_dir: bool,
    /// Delete the cache directory
    #[arg(long)]
    rm_cache_dir: bool,
    /// JavaScript runtimes (RUNTIME[:PATH]; deno and node are supported)
    #[arg(long, value_name = "RUNTIME[:PATH]")]
    js_runtimes: Vec<String>,
    #[arg(long, hide = true)]
    no_js_runtimes: bool,
    #[arg(long, hide = true, value_name = "COMPONENT")]
    remote_components: Vec<String>,
    #[arg(long, hide = true)]
    no_remote_components: bool,
    #[arg(long, hide = true, value_name = "PATH")]
    plugin_dirs: Vec<String>,
    #[arg(long, hide = true)]
    no_plugin_dirs: bool,
    /// Compatibility options (validated; not supported natively)
    #[arg(long, value_name = "OPTS")]
    compat_options: Vec<String>,
    /// Extractor arguments (not supported natively)
    #[arg(long, value_name = "IE_KEY:ARGS")]
    extractor_args: Vec<String>,
    /// Client to impersonate (no impersonation targets are available)
    #[arg(long, value_name = "CLIENT[:OS]")]
    impersonate: Option<String>,
    /// List available clients to impersonate
    #[arg(long)]
    list_impersonate_targets: bool,
    #[arg(long, hide = true, overrides_with = "no_video_multistreams")]
    video_multistreams: bool,
    #[arg(long, hide = true, overrides_with = "video_multistreams")]
    no_video_multistreams: bool,
    #[arg(long, hide = true, overrides_with = "no_audio_multistreams")]
    audio_multistreams: bool,
    #[arg(long, hide = true, overrides_with = "audio_multistreams")]
    no_audio_multistreams: bool,
    #[arg(long, hide = true, overrides_with = "no_allow_unplayable_formats")]
    allow_unplayable_formats: bool,
    #[arg(long, hide = true, overrides_with = "allow_unplayable_formats")]
    no_allow_unplayable_formats: bool,
    /// Download all formats (unsupported: equivalent to -f all)
    #[arg(long)]
    all_formats: bool,
    /// Account credentials (unused by the supported extractors)
    #[arg(short = 'u', long, value_name = "USERNAME")]
    username: Option<String>,
    #[arg(short = 'p', long, value_name = "PASSWORD")]
    password: Option<String>,
    #[arg(short = '2', long, value_name = "TWOFACTOR")]
    twofactor: Option<String>,
    #[arg(short = 'n', long)]
    netrc: bool,
    #[arg(long, value_name = "PATH")]
    netrc_location: Option<String>,
    #[arg(long, value_name = "NETRC_CMD")]
    netrc_cmd: Option<String>,
    #[arg(long, value_name = "PASSWORD")]
    video_password: Option<String>,
    #[arg(long, hide = true, value_name = "MSO")]
    ap_mso: Option<String>,
    #[arg(long, hide = true, value_name = "USERNAME")]
    ap_username: Option<String>,
    #[arg(long, hide = true, value_name = "PASSWORD")]
    ap_password: Option<String>,
    /// List all supported multiple-system operators
    #[arg(long)]
    ap_list_mso: bool,
    /// Geo-restriction options (unused by the supported extractors)
    #[arg(long, value_name = "URL")]
    geo_verification_proxy: Option<String>,
    #[arg(long, value_name = "VALUE")]
    xff: Option<String>,
    #[arg(long, hide = true)]
    geo_bypass: bool,
    #[arg(long, hide = true)]
    no_geo_bypass: bool,
    #[arg(long, hide = true, value_name = "CODE")]
    geo_bypass_country: Option<String>,
    #[arg(long, hide = true, value_name = "IP_BLOCK")]
    geo_bypass_ip_block: Option<String>,
    /// Path to the client certificate file in PEM format (may include the private key)
    #[arg(long, value_name = "CERTFILE")]
    client_certificate: Option<PathBuf>,
    /// Path to the private key file for the client certificate
    #[arg(long, value_name = "KEYFILE")]
    client_certificate_key: Option<PathBuf>,
    /// Password for an encrypted client certificate key (not supported natively)
    #[arg(long, value_name = "PASSWORD")]
    client_certificate_password: Option<String>,
    /// Display progress in the console titlebar (not supported natively)
    #[arg(long)]
    console_title: bool,
    /// Force the specified encoding (only UTF-8 is supported natively)
    #[arg(long, value_name = "ENCODING")]
    encoding: Option<String>,
    #[arg(long, hide = true)]
    legacy_server_connect: bool,
    /// Use an unencrypted connection to retrieve information where an extractor supports it
    #[arg(long, alias = "prefer-unsecure")]
    prefer_insecure: bool,
    #[arg(long, hide = true)]
    bidi_workaround: bool,
    #[arg(long, hide = true, overrides_with = "no_ignore_no_formats_error")]
    ignore_no_formats_error: bool,
    #[arg(long, hide = true, overrides_with = "ignore_no_formats_error")]
    no_ignore_no_formats_error: bool,
    #[arg(
        long,
        hide = true,
        alias = "no-ignore-dynamic-mpd",
        overrides_with = "ignore_dynamic_mpd"
    )]
    allow_dynamic_mpd: bool,
    #[arg(
        long,
        hide = true,
        alias = "no-allow-dynamic-mpd",
        overrides_with = "allow_dynamic_mpd"
    )]
    ignore_dynamic_mpd: bool,
    /// Allow file:// URLs (direct local media files)
    #[arg(long)]
    enable_file_urls: bool,
    /// Size of a chunk for chunk-based HTTP downloading, e.g. 10485760 or 10M
    #[arg(long, value_name = "SIZE")]
    http_chunk_size: Option<String>,
    /// Download only the first 10241 bytes of HTTP media (upstream's test mode)
    #[arg(long, hide = true)]
    test: bool,
    /// Seconds to sleep before each download (the minimum with --max-sleep-interval)
    #[arg(
        long,
        alias = "min-sleep-interval",
        value_name = "SECONDS",
        allow_hyphen_values = true
    )]
    sleep_interval: Option<f64>,
    /// Maximum seconds to sleep before each download; requires --min-sleep-interval
    #[arg(long, value_name = "SECONDS", allow_hyphen_values = true)]
    max_sleep_interval: Option<f64>,
    /// Seconds to sleep before each subtitle download
    #[arg(long, value_name = "SECONDS", allow_hyphen_values = true)]
    sleep_subtitles: Option<f64>,
    /// Seconds to sleep between requests during data extraction
    #[arg(long, value_name = "SECONDS", allow_hyphen_values = true)]
    sleep_requests: Option<f64>,
    /// Skip media downloads smaller than SIZE, e.g. 50k or 44.6M
    #[arg(long, value_name = "SIZE")]
    min_filesize: Option<String>,
    /// Skip media downloads larger than SIZE, e.g. 50k or 44.6M
    #[arg(long, value_name = "SIZE")]
    max_filesize: Option<String>,
    /// Output directories: [TYPES:]PATH with home (default) or an output type such as subtitle/infojson
    #[arg(short = 'P', long, value_name = "[TYPES:]PATH")]
    paths: Vec<String>,
    /// Restrict filenames to ASCII and avoid "&" and spaces
    #[arg(long, overrides_with = "no_restrict_filenames")]
    restrict_filenames: bool,
    /// Allow Unicode characters, "&" and spaces in filenames (default)
    #[arg(long, overrides_with = "restrict_filenames")]
    no_restrict_filenames: bool,
    /// Force Windows-compatible filenames
    #[arg(long, overrides_with = "no_windows_filenames")]
    windows_filenames: bool,
    /// Sanitize filenames only minimally (outside Windows)
    #[arg(long, overrides_with = "windows_filenames")]
    no_windows_filenames: bool,
    /// Limit the filename length (excluding extension) to LENGTH characters
    #[arg(
        long,
        alias = "trim-file-names",
        value_name = "LENGTH",
        default_value_t = 0,
        allow_hyphen_values = true
    )]
    trim_filenames: i64,
}

/// Upstream `simulate`/`quiet` resolution (`__init__` and `YoutubeDL.__init__`).
#[derive(Debug, PartialEq, Eq)]
struct Simulation {
    simulate: bool,
    /// `simulate == 'list_only'`: listing without format selection.
    list_only: bool,
    quiet: bool,
}

impl Args {
    fn simulation(&self) -> Simulation {
        // Upstream `print_only`: print templates imply simulation unless a
        // later stage (`before_dl` onwards) prints; any printing implies quiet.
        let any_getting = self.dump_json
            || self.dump_single_json
            || self.get_title
            || self.get_id
            || self.get_url
            || self.get_filename
            || self.get_thumbnail
            || self.get_description
            || self.get_duration
            || self.get_format;
        let print_only = !self.print.is_empty()
            && self.print.iter().all(|spec| {
                !matches!(
                    yt_dlp_rs::output::print_stage_name(spec).as_str(),
                    "before_dl" | "post_process" | "after_move" | "after_video" | "playlist"
                )
            });
        let getting = any_getting || print_only;
        let listing = self.list_formats || self.list_subs || self.list_thumbnails;
        let explicit = if self.simulate {
            Some(true)
        } else if self.no_simulate {
            Some(false)
        } else {
            None
        };
        let quiet = if self.quiet {
            Some(true)
        } else if self.no_quiet {
            Some(false)
        } else {
            None
        };
        Simulation {
            simulate: explicit.unwrap_or(getting || listing),
            list_only: explicit.is_none() && !getting && listing,
            // Hidden `--print-json` is quiet JSON without simulation.
            quiet: quiet.unwrap_or(any_getting || self.print_json || !self.print.is_empty()),
        }
    }
}

/// Append one UTF-8 line, creating parent directories like upstream.
fn append_line(path: &std::path::Path, line: &str) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(format!("{line}\n").as_bytes())
}

/// The media filename (`prepare_filename(info)`); the default template is never empty.
fn prepare_media(
    templates: &OutputTemplates,
    info: &yt_dlp_rs::VideoInfo,
    options: &TemplateOptions,
) -> yt_dlp_rs::Result<PathBuf> {
    // `-o -`: the media goes to stdout.
    if templates.get(None) == "-" {
        return Ok(PathBuf::from("-"));
    }
    Ok(templates
        .prepare(info, None, options)?
        .expect("default output template is non-empty"))
}

/// Upstream warns when an absolute media template makes `--paths` irrelevant.
fn path_is_absolute_template(templates: &OutputTemplates) -> bool {
    std::path::Path::new(&yt_dlp_rs::batch::expand_path_text(templates.get(None))).is_absolute()
}

/// Whether a regular file exists; other existing entries (including dangling
/// symlinks, which upstream would write through) are refused natively.
fn path_exists(path: &std::path::Path) -> yt_dlp_rs::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) if std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) => Ok(true),
        Ok(_) => Err(yt_dlp_rs::Error::OutputExists(path.into())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error.into()),
    }
}

/// The `-J` object for one URL: the `-j` video object, or the playlist with
/// `_type`, selected counts/indices and every selected entry.
fn single_json(
    result: &ExtractResult,
    counts: Option<PlaylistCounts>,
) -> serde_json::Result<serde_json::Value> {
    match result {
        ExtractResult::Video(info) => serde_json::to_value(info),
        ExtractResult::Playlist(playlist) => {
            let mut value = serde_json::to_value(playlist)?;
            let object = value
                .as_object_mut()
                .expect("playlist serializes to an object");
            let entries = object.shift_remove("entries");
            object.insert("_type".into(), "playlist".into());
            if let Some(counts) = counts {
                object.insert("playlist_count".into(), counts.playlist_count.into());
            }
            let indices: Vec<_> = playlist
                .entries
                .iter()
                .map(|info| info.playlist_index)
                .collect();
            // Lazy playlists have no `requested_entries` upstream, and full
            // playlists drop it ("Do not set for full playlist").
            let full = counts.is_some_and(|counts| {
                indices
                    .iter()
                    .copied()
                    .eq((1..=counts.playlist_count).map(Some))
            });
            if !full && counts.is_none_or(|counts| counts.n_entries.is_some()) {
                object.insert("requested_entries".into(), serde_json::to_value(indices)?);
            }
            object.insert("entries".into(), entries.unwrap_or_default());
            Ok(value)
        }
    }
}

/// Upstream `std_headers`, with `random_user_agent()`'s Chrome major
/// version drawn from 145..=151.
fn std_headers() -> Vec<(String, String)> {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(epoch());
    let version = 145 + hasher.finish() % 7;
    [
        (
            "User-Agent",
            format!(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{version}.0.0.0 Safari/537.36"
            ),
        ),
        (
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8".into(),
        ),
        ("Accept-Language", "en-us,en;q=0.5".into()),
        ("Sec-Fetch-Mode", "navigate".into()),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value))
    .collect()
}

/// Options for [`run_playlist_stage`].
struct PlaylistStage {
    simulate: bool,
    concat: bool,
    /// `FFmpegConcatPP` runs (`--concat-playlist` is not `never`).
    concat_pp: bool,
    keep: bool,
}

/// Upstream `run_all_pps('playlist', ...)` after the entries: playlist
/// `--print` lines, then `FFmpegConcatPP` (`--concat-playlist always`) and
/// `--exec playlist:` commands. Returns false after a reported failure.
#[allow(clippy::too_many_arguments)]
fn run_playlist_stage(
    dict: &serde_json::Value,
    files: &[Option<PathBuf>],
    print_templates: &[PrintTemplate],
    print_files: &[PrintToFile],
    exec: &ExecCommands,
    templates: &OutputTemplates,
    options: &TemplateOptions,
    downloader: &YoutubeDl,
    stage: PlaylistStage,
    metadata_actions: &yt_dlp_rs::metadata_parser::MetadataActions,
) -> Result<bool, Box<dyn std::error::Error>> {
    let lines = print_templates
        .iter()
        .filter(|template| template.stage() == Stage::Playlist)
        .map(|template| template.render_dict(dict, options))
        .collect::<yt_dlp_rs::Result<_>>()?;
    let printed = print_files
        .iter()
        .filter(|pair| pair.stage() == Stage::Playlist)
        .map(|pair| {
            let (path, line) = pair.render_dict(dict, options)?;
            Ok((pair.template().to_owned(), path, line))
        })
        .collect::<yt_dlp_rs::Result<_>>()?;
    StagePrint {
        lines,
        files: printed,
    }
    .emit()?;
    // `MetadataParserPP` follows the prints and precedes Concat and Exec.
    let mut parsed = dict.clone();
    if metadata_actions.has("playlist") {
        if wants_pp_info() {
            *PP_INFO.lock().unwrap() = Some(dict.clone());
        }
        if let Err(error) = metadata_actions.run_reported("playlist", &mut parsed, options) {
            report_error!("ERROR: {error}");
            return Ok(false);
        }
    }
    let dict = &parsed;
    if stage.concat_pp {
        postprocess_progress("Concat", "started");
    }
    // `run_pp` deletes the inputs after the processor's `finished` hook.
    let concat_finished = std::cell::Cell::new(false);
    let finish_concat = || {
        if stage.concat_pp && !concat_finished.replace(true) {
            postprocess_progress("Concat", "finished");
        }
    };
    if stage.concat && !stage.simulate && !files.is_empty() {
        let concat = (|| -> yt_dlp_rs::Result<()> {
            let Some(inputs) = files.iter().cloned().collect::<Option<Vec<PathBuf>>>() else {
                return Err(yt_dlp_rs::Error::Postprocess(
                    "Aborting concatenation because some downloads failed".into(),
                ));
            };
            let exts: Vec<String> = inputs
                .iter()
                .map(|path| {
                    path.extension()
                        .map(|ext| ext.to_string_lossy().into_owned())
                        .unwrap_or_default()
                })
                .collect();
            let ext = if exts.iter().all(|ext| *ext == exts[0]) {
                exts[0].clone()
            } else {
                "mkv".into()
            };
            // `ChainMap({'ext': ...}, info, _playlist_infodict(info))`.
            let mut copy = dict.clone();
            if let Some(object) = copy.as_object_mut() {
                let field = |name: &str| dict.get(name).cloned().unwrap_or_default();
                let title = field("title");
                let fallback = if title.is_null() {
                    field("id")
                } else {
                    title.clone()
                };
                // `max(requested_entries or (0, 0))`.
                let last = dict
                    .get("requested_entries")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|entries| entries.iter().filter_map(serde_json::Value::as_u64).max())
                    .unwrap_or(0);
                for (name, value) in [
                    ("playlist_count", field("playlist_count")),
                    ("playlist", fallback),
                    ("playlist_id", field("id")),
                    ("playlist_title", title),
                    ("playlist_webpage_url", field("webpage_url")),
                    ("playlist_index", 0.into()),
                    ("__last_playlist_index", last.into()),
                ] {
                    object.entry(name).or_insert(value);
                }
                object.insert("ext".into(), ext.into());
            }
            let output = yt_dlp_rs::output::render_dict_filename(
                templates.get(Some("pl_video")),
                &copy,
                options,
                "pl_video",
            )?;
            if let [input] = inputs.as_slice() {
                if std::fs::canonicalize(input).ok() != std::fs::canonicalize(&output).ok() {
                    screen!(
                        "[Concat] Moving \"{}\" to \"{}\"",
                        input.display(),
                        output.display()
                    );
                }
                std::fs::rename(input, &output)?;
                return Ok(());
            }
            screen!(
                "[Concat] Concatenating {} files; Destination: {}",
                inputs.len(),
                output.display()
            );
            downloader.concat_files(&inputs, &output)?;
            finish_concat();
            if !stage.keep {
                for input in &inputs {
                    screen!(
                        "Deleting original file {} (pass -k to keep)",
                        input.display()
                    );
                    if std::fs::remove_file(input).is_err() {
                        warning!("WARNING: Unable to delete file {}", input.display());
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = concat {
            report_error!("ERROR: Postprocessing: {error}");
            return Ok(false);
        }
    }
    finish_concat();
    // One `ExecPP` runs the stage's commands.
    let has_exec = exec.stage(Stage::Playlist).next().is_some();
    if has_exec {
        postprocess_progress("Exec", "started");
    }
    for command in exec.stage(Stage::Playlist) {
        let command = command.render_dict(dict, options)?;
        screen!("[Exec] Executing command: {command}");
        if let Err(error) = ExecCommand::run(&command) {
            report_error!("ERROR: Postprocessing: {error}");
            return Ok(false);
        }
    }
    if has_exec {
        postprocess_progress("Exec", "finished");
    }
    Ok(true)
}

/// Python `str(float)` for ordinary values (`1.0`, `0.5`, `nan`, `inf`).
fn py_float(value: f64) -> String {
    yt_dlp_rs::screen::py_float(value)
}

/// Upstream download/subtitle sleep intervals (`__init__` validation and
/// `FileDownloader.download`).
struct SleepPolicy {
    min: f64,
    max: f64,
    subtitles: f64,
}

impl SleepPolicy {
    fn from_cli(
        min: Option<f64>,
        max: Option<f64>,
        subtitles: Option<f64>,
        requests: Option<f64>,
    ) -> Result<Self, String> {
        for (name, value) in [
            ("subtitles sleep interval", subtitles),
            ("requests sleep interval", requests),
            ("sleep interval", min),
            ("max sleep interval", max),
        ] {
            if let Some(value) = value
                && !(value >= 0.0 && value.is_finite())
            {
                return Err(format!(
                    "{name} \"{}\" must be positive or 0",
                    py_float(value)
                ));
            }
        }
        let max = match (min, max) {
            (None, Some(_)) => {
                return Err(
                    "min sleep interval must be specified; use --min-sleep-interval".into(),
                );
            }
            (Some(min), Some(max)) if max < min => {
                return Err(format!(
                    "max sleep interval \"{}\" must be must be greater than or equal to min sleep interval \"{}\"",
                    py_float(max),
                    py_float(min)
                ));
            }
            (min, max) => max.or(min).unwrap_or(0.0),
        };
        Ok(Self {
            min: min.unwrap_or(0.0),
            max,
            subtitles: subtitles.unwrap_or(0.0),
        })
    }

    fn pause(seconds: f64) {
        if seconds > 0.0 {
            screen!("[download] Sleeping {seconds:.2} seconds ...");
            std::thread::sleep(Duration::from_secs_f64(seconds));
        }
    }

    /// `random.uniform(min, max or min)`.
    fn media(&self) {
        let max = if self.max == 0.0 { self.min } else { self.max };
        let unit = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.subsec_nanos()) as f64
            / 1e9;
        Self::pause(self.min + (max - self.min) * unit);
    }

    fn subtitle(&self) {
        Self::pause(self.subtitles);
    }
}

/// Upstream `parse_bytes`: `NUMBER[.DIGITS][ ]UNIT` with binary K..Y units,
/// rounded half to even. Non-ASCII digits are not accepted natively.
fn parse_bytes(value: &str) -> Option<u64> {
    let upper = value.to_uppercase();
    let digits = upper
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(upper.len());
    if digits == 0 {
        return None;
    }
    let mut end = digits;
    if let Some(fraction) = upper[end..].strip_prefix('.') {
        let length = fraction
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(fraction.len());
        if length == 0 {
            return None;
        }
        end += 1 + length;
    }
    let number: f64 = upper[..end].parse().ok()?;
    let rest = &upper[end..];
    let unit = rest.trim_start_matches(yt_dlp_rs::batch::is_py_space);
    let power = match unit {
        // `\b` after an empty unit cannot follow consumed whitespace.
        "" if unit.len() == rest.len() => 0,
        "" => return None,
        unit => "KMGTPEZY".find(unit).filter(|_| unit.len() == 1)? as i32 + 1,
    };
    let bytes = (number * 1024f64.powi(power)).round_ties_even();
    (bytes.is_finite() && bytes <= u64::MAX as f64).then_some(bytes as u64)
}

fn size_option(name: &str, value: Option<&str>) -> Result<Option<u64>, String> {
    value
        .map(|value| parse_bytes(value).ok_or_else(|| format!("invalid {name} \"{value}\" given")))
        .transpose()
}

fn run() -> Result<bool, Box<dyn std::error::Error>> {
    let mut argv = std::env::args();
    let program = argv.next().unwrap_or_else(|| "yt-dlp-rs".into());
    let (arguments, config_debug) = config_arguments(argv.collect())?;
    let arguments = expand_preset_aliases(arguments).map_err(UsageError)?;
    let (arguments, deprecated) = strip_deprecated_options(arguments);
    let matches = match command().try_get_matches_from(std::iter::once(program).chain(arguments)) {
        Ok(matches) => matches,
        Err(error) if error.kind() == clap::error::ErrorKind::UnknownArgument => {
            return Err(UsageError(unknown_option(&error)).into());
        }
        Err(error) => error.exit(),
    };
    // Upstream `parseOpts` prints the configuration tree with `--verbose`.
    if matches.get_flag("verbose") {
        for line in &config_debug {
            eprintln!("[debug] {line}");
        }
    }
    if !deprecated.is_empty() {
        // Upstream `deprecated_feature`, written directly to stderr.
        eprintln!(
            "Deprecated Feature: The following options have been deprecated: {}\nPlease remove them from your command/configuration to avoid future errors.\nSee  https://github.com/yt-dlp/yt-dlp/issues/14198  for more details",
            deprecated.join(", ")
        );
    }
    let mut args = Args::from_arg_matches(&matches)?;
    yt_dlp_rs::screen::set_verbose(args.verbose);
    yt_dlp_rs::screen::set_page_dumps(
        args.dump_pages,
        args.write_pages,
        usize::try_from(args.trim_filenames).ok().filter(|&n| n > 0),
    );
    yt_dlp_rs::extractor::hls_split_discontinuity(args.hls_split_discontinuity);
    // Upstream `parse_metadata` (shared by both options, in command-line
    // order per stage) plus the deprecated `--metadata-from-title`.
    let metadata_actions = {
        let mut ordered: Vec<(usize, Vec<String>)> = Vec::new();
        for (id, arity) in [("parse_metadata", 1), ("replace_in_metadata", 3)] {
            let indices: Vec<usize> = matches.indices_of(id).into_iter().flatten().collect();
            let values: Vec<String> = matches
                .get_many::<String>(id)
                .into_iter()
                .flatten()
                .cloned()
                .collect();
            for (chunk, index) in values.chunks(arity).zip(indices.chunks(arity)) {
                ordered.push((index[0], chunk.to_vec()));
            }
        }
        ordered.sort_by_key(|(index, _)| *index);
        if let Some(format) = &args.metadata_from_title {
            ordered.push((usize::MAX, vec![format!("pre_process:title:{format}")]));
        }
        let mut actions = yt_dlp_rs::metadata_parser::MetadataActions::default();
        for (_, values) in ordered {
            match values.as_slice() {
                [value] => actions.add_interpret(value),
                [fields, search, replace] => actions.add_replace(fields, search, replace),
                _ => unreachable!("clap arity"),
            }
            .map_err(usage)?;
        }
        actions
    };
    // Upstream `allsubtitles`: every language, implying --write-subs unless
    // embedding or automatic captions are requested.
    if args.all_subs {
        args.sub_langs = vec!["all".into()];
        if !(args.embed_subs || args.write_auto_subs) {
            args.write_subs = true;
        }
    }
    validate_accepted_options(&args)?;
    // `--extractor-args`: GenericIE's are used; other extractors' are not.
    let extractor_args =
        yt_dlp_rs::manifest_args::parse_extractor_args(&args.extractor_args).map_err(UsageError)?;
    for key in extractor_args.keys().filter(|key| *key != "generic") {
        warning!(
            "WARNING: extractor arguments for {key} are not supported natively and are ignored"
        );
    }
    yt_dlp_rs::manifest_args::set_extractor_args(extractor_args);
    yt_dlp_rs::extractor::set_video_password(args.video_password.clone());
    // HttpFD's initial block size and resizing.
    if let Some(size) = args.buffer_size.as_deref().and_then(parse_bytes) {
        yt_dlp_rs::BUFFER_SIZE.store(size as usize, Ordering::Relaxed);
    }
    yt_dlp_rs::RESIZE_BUFFER.store(!args.no_resize_buffer, Ordering::Relaxed);
    // `--no-match-filters` resets values given before it (optparse store_const).
    let after_reset = |values: &str, reset: &str| -> Vec<String> {
        // Defaulted flags also report an index, so require a command-line source.
        let last_reset = (matches.value_source(reset)
            == Some(clap::parser::ValueSource::CommandLine))
        .then(|| matches.indices_of(reset).and_then(|indices| indices.max()))
        .flatten()
        .unwrap_or(0);
        matches
            .indices_of(values)
            .into_iter()
            .flatten()
            .zip(matches.get_many::<String>(values).into_iter().flatten())
            .filter(|(index, _)| *index > last_reset)
            .map(|(_, value)| value.clone())
            .collect()
    };
    // Upstream `set_compat_opts`: compatibility options change the defaults
    // of options not given (an option overriding one shows `*NAME` in the
    // debug header).
    let compat = compat_options(&args.compat_options)?;
    let given = |ids: &[&str]| {
        ids.iter()
            .any(|id| matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine))
    };
    let mut compat_shown = Vec::new();
    let (mut compat_abort, mut keep_subs, mut compat_format_sort) = (false, true, false);
    for name in &compat {
        let used = match name.as_str() {
            "abort-on-error" => {
                compat_abort = !given(&["ignore_errors", "no_abort_on_error", "abort_on_error"]);
                compat_abort
            }
            "no-playlist-metafiles" => {
                let unset = !given(&["write_playlist_metafiles", "no_write_playlist_metafiles"]);
                args.no_write_playlist_metafiles |= unset;
                unset
            }
            "no-clean-infojson" => {
                let unset = !given(&["clean_info_json", "no_clean_info_json"]);
                args.no_clean_info_json |= unset;
                unset
            }
            "no-attach-info-json" => {
                args.no_embed_info_json |= !args.embed_info_json;
                !args.embed_info_json
            }
            "mtime-by-default" => {
                let unset = !given(&["mtime", "no_mtime"]);
                args.mtime |= unset;
                unset
            }
            "filename" => {
                let unset = !OutputTemplates::sets_default(&args.output);
                if unset {
                    args.output.insert(0, "%(title)s-%(id)s.%(ext)s".into());
                }
                unset
            }
            "format-sort" => {
                compat_format_sort = true;
                true
            }
            // `elif`: only without format-sort.
            "prefer-vp9-sort" => {
                yt_dlp_rs::format_sort::PREFER_VP9_COMPAT.store(true, Ordering::Relaxed);
                if !compat.iter().any(|name| name == "format-sort") {
                    yt_dlp_rs::format_sort::PREFER_VP9_SORT.store(true, Ordering::Relaxed);
                }
                true
            }
            // `set_default_compat('multistreams', ..., False,
            // remove_compat=False)` for both stream kinds.
            "multistreams" => {
                let video = given(&["video_multistreams", "no_video_multistreams"]);
                let audio = given(&["audio_multistreams", "no_audio_multistreams"]);
                if !video {
                    args.video_multistreams = true;
                }
                if !audio {
                    args.audio_multistreams = true;
                }
                !(video && audio)
            }
            "manifest-filesize-approx" => {
                yt_dlp_rs::COMPAT_MANIFEST_FILESIZE_APPROX.store(true, Ordering::Relaxed);
                true
            }
            "format-spec" => {
                yt_dlp_rs::COMPAT_FORMAT_SPEC.store(true, Ordering::Relaxed);
                true
            }
            "list-formats" => {
                args.list_formats_old = true;
                true
            }
            "no-keep-subs" => {
                keep_subs = false;
                true
            }
            "filename-sanitization"
            | "allow-unsafe-exec-expansion"
            | "playlist-index"
            | "no-direct-merge" => true,
            "embed-metadata" => {
                yt_dlp_rs::COMPAT_EMBED_METADATA.store(true, Ordering::Relaxed);
                true
            }
            "allow-unsafe-ext" => {
                ALLOW_UNSAFE_EXT.store(true, Ordering::Relaxed);
                validation_warning(
                    12,
                    "Using allow-unsafe-ext opens you up to potential attacks. Use with great care!"
                        .into(),
                );
                true
            }
            // Features this port does not have (YouTube tabs, live chat,
            // certifi, ...) or that pinned upstream never reads, so nothing
            // changes. This port's playlist entries are always complete
            // videos, which `playlist-match-filter` does not affect.
            "no-certifi"
            | "prefer-legacy-http-handler"
            | "no-live-chat"
            | "no-external-downloader-progress"
            | "embed-thumbnail-atomicparsley"
            | "no-youtube-channel-redirect"
            | "no-youtube-unavailable-videos"
            | "no-youtube-prefer-utc-upload-date"
            | "seperate-video-versions"
            | "playlist-match-filter" => true,
            other => {
                warning!(
                    "WARNING: compatibility option {other} is not supported natively and is ignored"
                );
                true
            }
        };
        compat_shown.push(if used {
            name.clone()
        } else {
            format!("*{name}")
        });
    }
    // Upstream `ignoreerrors`: -i (True), --no-abort-on-error ("only_download",
    // the default) and --abort-on-error (False); the last flag wins.
    let error_mode = [
        ("ignore_errors", ErrorMode::Ignore),
        ("no_abort_on_error", ErrorMode::OnlyDownload),
        ("abort_on_error", ErrorMode::Abort),
    ]
    .into_iter()
    .filter(|(id, _)| matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine))
    .flat_map(|(id, mode)| {
        matches
            .indices_of(id)
            .into_iter()
            .flatten()
            .map(move |i| (i, mode))
    })
    .max_by_key(|(index, _)| *index)
    .map_or(
        if compat_abort {
            ErrorMode::Abort
        } else {
            ErrorMode::OnlyDownload
        },
        |(_, mode)| mode,
    );
    // Upstream `writethumbnail`: --write-thumbnail keeps an earlier "all",
    // --no-write-thumbnail clears it, --write-all-thumbnails sets "all".
    let thumbnail_mode = {
        let mut events: Vec<(usize, u8)> = Vec::new();
        for (id, kind) in [
            ("write_thumbnail", 1),
            ("no_write_thumbnail", 0),
            ("write_all_thumbnails", 2),
        ] {
            if matches.value_source(id) == Some(clap::parser::ValueSource::CommandLine) {
                events.extend(
                    matches
                        .indices_of(id)
                        .into_iter()
                        .flatten()
                        .map(|i| (i, kind)),
                );
            }
        }
        events.sort();
        events.into_iter().fold(0u8, |state, (_, kind)| match kind {
            1 => state.max(1),
            other => other,
        })
    };
    let match_filters = MatchFilters::parse(
        &after_reset("match_filters", "no_match_filters"),
        &after_reset("break_match_filters", "no_break_match_filters"),
    )?;
    // Upstream `report_conflict` for --date, then `opts.date` resolution;
    // the warnings print only after validation succeeds.
    let now = epoch() as i64;
    let date = args.date.as_deref().filter(|date| !date.is_empty());
    let (mut dateafter, mut datebefore) = (args.dateafter.as_deref(), args.datebefore.as_deref());
    if date.is_some() {
        for (name, value) in [
            ("--dateafter", &mut dateafter),
            ("--datebefore", &mut datebefore),
        ] {
            if value.take().is_some_and(|value| !value.is_empty()) {
                let rank = if name == "--dateafter" { 7 } else { 8 };
                validation_warning(rank, format!("{name} is ignored since --date was given"));
            }
        }
    }
    let date_range = match date {
        Some(date) => Some(DateRange::day(date, now).map_err(usage)?),
        None if dateafter.is_some() || datebefore.is_some() => {
            Some(DateRange::new(dateafter, datebefore, now).map_err(usage)?)
        }
        None => None,
    };
    let title_pattern = |pattern: Option<&String>| {
        pattern
            .map(|pattern| EntrySelection::title_pattern(pattern))
            .transpose()
    };
    let match_filters = match_filters.with_selection(EntrySelection {
        match_title: title_pattern(args.match_title.as_ref())?,
        reject_title: title_pattern(args.reject_title.as_ref())?,
        date_range,
        min_views: args.min_views,
        max_views: args.max_views,
        age_limit: args.age_limit,
        break_on_reject: args.break_on_reject,
        now,
    });
    QUIET.store(args.simulation().quiet, Ordering::Relaxed);
    yt_dlp_rs::screen::set_sink(|message| match message {
        yt_dlp_rs::screen::Message::Screen(text) => screen!("{text}"),
        yt_dlp_rs::screen::Message::Warning(text) => warning!("WARNING: {text}"),
        yt_dlp_rs::screen::Message::Stderr(text) => eprintln!("{text}"),
        yt_dlp_rs::screen::Message::Debug(text) => eprintln!("[debug] {text}"),
        yt_dlp_rs::screen::Message::Stdout(text) => println!("{text}"),
        yt_dlp_rs::screen::Message::Postprocess {
            postprocessor,
            status,
        } => postprocess_progress(postprocessor, status),
    });
    NO_WARNINGS.store(args.no_warnings, Ordering::Relaxed);
    if args.no_download_archive {
        args.download_archive = None;
    }
    // Like upstream get_urls: batch URLs precede positional URLs, all stripped.
    let mut urls = match args.batch_file.as_deref().filter(|_| !args.no_batch_file) {
        Some(spec) => {
            // Upstream checks only an explicit -q here, before implied quiet.
            if spec == "-" && !args.quiet {
                eprintln!("Reading URLs from STDIN - EOF (Ctrl+D) to end:");
            }
            let urls = yt_dlp_rs::batch::load_batch_file(spec)
                .map_err(|_| format!("batch file {spec} could not be read"))?;
            if args.verbose {
                eprintln!("[debug] Batch file urls: {}", py_list(&urls));
            }
            urls
        }
        None => Vec::new(),
    };
    urls.extend(
        std::mem::take(&mut args.urls)
            .iter()
            .map(|url| yt_dlp_rs::batch::py_strip(url).to_owned()),
    );
    let mut print_templates = args
        .print
        .iter()
        .map(|spec| PrintTemplate::parse(spec))
        .collect::<yt_dlp_rs::Result<Vec<_>>>()?;
    print_templates.extend(PrintTemplate::forced_fields(
        yt_dlp_rs::output::ForcedFields {
            title: args.get_title,
            id: args.get_id,
            url: args.get_url,
            thumbnail: args.get_thumbnail,
            description: args.get_description,
            filename: args.get_filename,
            duration: args.get_duration,
            format: args.get_format,
        },
    ));
    let print_files = args
        .print_to_file
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[spec, file]| PrintToFile::parse(spec, file))
        .collect::<yt_dlp_rs::Result<Vec<_>>>()?;
    let prints_stage = |stage: Stage| {
        print_templates
            .iter()
            .any(|template| template.stage() == stage)
            || print_files.iter().any(|pair| pair.stage() == stage)
    };
    // `quiet` was applied to screen output at startup.
    let Simulation {
        simulate,
        list_only,
        ..
    } = args.simulation();
    let quiet_screen = QUIET.load(Ordering::Relaxed);
    let progress_options = ProgressOptions {
        enabled: if args.no_progress {
            false
        } else if args.progress {
            true
        } else {
            !quiet_screen
        },
        newline: args.newline,
        quiet: quiet_screen,
        delta: args
            .progress_delta
            .filter(|delta| *delta > 0.0 && delta.is_finite())
            .map(Duration::from_secs_f64),
        // Upstream `progress_template` (`_dict_from_options_callback` keys).
        template: {
            let mut template = None;
            let mut console = ConsoleOptions {
                title: args.console_title,
                ..Default::default()
            };
            for value in &args.progress_template {
                static KEYS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                    regex::Regex::new(r"(?is)^(?P<keys>(?:download|postprocess)(?:-title)?(?:,(?:download|postprocess)(?:-title)?)*):(?P<val>.*)$")
                        .unwrap()
                });
                let (keys, text) = match KEYS.captures(value) {
                    Some(captures) => (
                        captures["keys"].to_lowercase(),
                        captures.name("val").unwrap().as_str().to_owned(),
                    ),
                    None => ("download".into(), value.clone()),
                };
                for key in keys.split(',') {
                    let slot = match key {
                        "download" => &mut template,
                        "download-title" => &mut console.download_title,
                        "postprocess" => &mut console.postprocess,
                        _ => &mut console.postprocess_title,
                    };
                    *slot = Some(text.clone());
                }
            }
            let _ = CONSOLE.set(console);
            template
        },
    };
    let write_subs = args.write_subs || (args.embed_subs && !args.write_auto_subs);
    // Upstream `validate_regex('audio format', ...)` and
    // `validate_positive('audio quality', int_or_none(float_or_none(q), default=0))`.
    AudioOptions::from_cli(&args.audio_format, "5").map_err(|_| {
        usage(format!(
            "invalid audio format \"{}\" given",
            args.audio_format
        ))
    })?;
    if let Some(quality) = yt_dlp_rs::batch::py_strip(&args.audio_quality)
        .parse::<f64>()
        .ok()
        .filter(|quality| quality.is_finite() && quality.trunc() < 0.0)
    {
        return Err(usage(format!(
            "audio quality \"{}\" must be positive or 0",
            quality.trunc()
        ))
        .into());
    }
    // Upstream `-S` (`_list_from_options_callback` with append=-1): each value
    // is split on commas and prepended; `--format-sort-reset` clears earlier ones.
    let mut format_sort: Vec<String> = Vec::new();
    for value in after_reset("format_sort", "format_sort_reset") {
        let mut fields: Vec<String> = value
            .split(',')
            .map(|field| field.trim().to_owned())
            .filter(|field| !field.is_empty())
            .collect();
        fields.append(&mut format_sort);
        format_sort = fields;
    }
    static SORT_FIELD: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^ *(?:\+?[a-zA-Z0-9_]+(?:[~:].*?)?)? *$").unwrap()
    });
    if compat_format_sort {
        // `FormatSorter.ytdl_default`.
        format_sort.extend(
            [
                "hasaud",
                "lang",
                "quality",
                "tbr",
                "filesize",
                "vbr",
                "height",
                "width",
                "proto",
                "vext",
                "abr",
                "aext",
                "fps",
                "fs_approx",
                "source",
                "id",
            ]
            .map(str::to_owned),
        );
    }
    if let Some(field) = format_sort.iter().find(|field| !SORT_FIELD.is_match(field)) {
        return Err(usage(format!("invalid format sorting \"{field}\" given")).into());
    }
    // Upstream `validate_regex` uses `re.match`: only a valid prefix is required.
    if let Some(format) = &args.merge_output_format {
        static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            let ext = "(?:avi|flv|mkv|mov|mp4|webm)";
            regex::Regex::new(&format!("^{ext}(?:/{ext})*")).unwrap()
        });
        if !PATTERN.is_match(format) {
            return Err(usage(format!("invalid merge output format \"{format}\" given")).into());
        }
    }
    let mut audio_options = AudioOptions::from_cli(&args.audio_format, &args.audio_quality)?;
    audio_options.keep_video = args.keep_video;
    // Source validation applies to both values even when recode takes precedence.
    let remux = args
        .remux_video
        .as_deref()
        .map(|format| {
            VideoOptions::from_cli(format, VideoMode::Remux)
                .map_err(|_| usage(format!("invalid remux video format \"{format}\" given")))
        })
        .transpose()?;
    let recode = args
        .recode_video
        .as_deref()
        .map(|format| {
            VideoOptions::from_cli(format, VideoMode::Recode)
                .map_err(|_| usage(format!("invalid recode video format \"{format}\" given")))
        })
        .transpose()?;
    if remux.is_some() && recode.is_some() {
        validation_warning(
            11,
            "--remux-video is ignored since --recode-video was given".into(),
        );
    }
    // Upstream `final_ext`: the expected extension after processing, used to
    // find an earlier run's processed file.
    let final_ext = {
        const VIDEO: [&str; 18] = [
            "avi", "flv", "gif", "mkv", "mov", "mp4", "webm", "aac", "aiff", "alac", "flac", "m4a",
            "mka", "mp3", "ogg", "opus", "vorbis", "wav",
        ];
        const AUDIO: [&str; 8] = ["mp3", "aac", "m4a", "opus", "vorbis", "flac", "alac", "wav"];
        let video = |value: &Option<String>| {
            value
                .as_deref()
                .filter(|value| VIDEO.contains(value))
                .map(str::to_owned)
        };
        video(&args.recode_video)
            .or_else(|| video(&args.remux_video))
            .or_else(|| {
                (args.extract_audio && AUDIO.contains(&args.audio_format.as_str()))
                    .then(|| args.audio_format.clone())
            })
    };
    // Upstream `parse_chapters('--remove-chapters', ...)`. Pinned upstream
    // only adds `ModifyChaptersPP` for chapter regexes, and fails building it
    // when time ranges are also given.
    let (remove_patterns, remove_ranges, _) = yt_dlp_rs::sections::parse_chapters(
        "--remove-chapters",
        &after_reset("remove_chapters", "no_remove_chapters"),
        false,
    )
    .map_err(usage)?;
    if !remove_patterns.is_empty() && !remove_ranges.is_empty() {
        return Err(
            "--remove-chapters time ranges together with chapter regexes are unsupported (pinned upstream fails with a TypeError)"
                .into(),
        );
    }
    // Upstream `--sponsorblock-mark`/`--sponsorblock-remove` sets.
    let (mut sponsorblock_mark, mut sponsorblock_remove) = (Vec::new(), Vec::new());
    for (values, current, option, remove) in [
        (
            &args.sponsorblock_mark,
            &mut sponsorblock_mark,
            "--sponsorblock-mark",
            false,
        ),
        (
            &args.sponsorblock_remove,
            &mut sponsorblock_remove,
            "--sponsorblock-remove",
            true,
        ),
    ] {
        for value in values {
            yt_dlp_rs::sponsorblock::parse_categories(current, value, option, remove)
                .map_err(usage)?;
        }
    }
    if let Err(error) = yt_dlp_rs::output::evaluate_dict_template(
        &args.sponsorblock_chapter_title,
        &serde_json::json!({}),
        &TemplateOptions::default(),
    ) {
        return Err(usage(format!(
            "invalid SponsorBlock chapter title \"{}\": {error}",
            args.sponsorblock_chapter_title
        ))
        .into());
    }
    if args.no_sponsorblock {
        sponsorblock_mark.clear();
        sponsorblock_remove.clear();
    }
    // `sponsorblock_query`, in upstream's category order.
    let sponsorblock_query: Vec<String> = yt_dlp_rs::sponsorblock::CATEGORIES
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .filter(|name| sponsorblock_mark.contains(name) || sponsorblock_remove.contains(name))
        .collect();
    let modify_chapters =
        (!remove_patterns.is_empty() || !sponsorblock_query.is_empty()).then(|| {
            yt_dlp_rs::ModifyChapters {
                patterns: remove_patterns,
                remove_sponsor: sponsorblock_remove.clone(),
                sponsor_title: args.sponsorblock_chapter_title.clone(),
                force_keyframes: args.force_keyframes_at_cuts,
                subtitles: Vec::new(),
            }
        });
    let processors = PostprocessOptions {
        existing: None,
        // Filled per video with the template options.
        metadata_parser: None,
        modify_chapters,
        ignore_errors: error_mode == ErrorMode::Ignore,
        no_post_overwrites: args.no_post_overwrites,
        thumbnail: None,
        metadata: args.embed_metadata,
        // Upstream: `--embed-metadata` implies `--embed-chapters` unless
        // `--no-embed-chapters` was given.
        // `--sponsorblock-mark` implies it as well.
        chapters: args.embed_chapters
            || ((args.embed_metadata || !sponsorblock_mark.is_empty()) && !args.no_embed_chapters),
        // Filled per video; see `embed_info_json` below.
        infojson: None,
        fixup: Some(yt_dlp_rs::FixupOptions {
            policy: args.fixup.clone(),
            hls_use_mpegts: hls_use_mpegts(&args),
            converted: false,
        }),
        audio: args.extract_audio.then_some(audio_options),
        video: recode.or(remux),
        embed: None,
        keep_video: args.keep_video,
    };
    let mut subtitle_conversion = args
        .convert_subs
        .as_deref()
        .map(|format| {
            SubtitleConversion::new(format)
                .map_err(|_| usage(format!("invalid subtitle format \"{format}\" given")))
        })
        .transpose()?
        .filter(SubtitleConversion::is_enabled);
    // `convert_subtitles` deletes the originals itself, like `run_pp`.
    if let Some(conversion) = &mut subtitle_conversion {
        conversion.keep_original = true;
    }
    let format = args
        .format
        .as_deref()
        .or(if args.extract_audio && !args.keep_video {
            Some("bestaudio/best")
        } else {
            None
        });
    // Upstream `validate_options`: common `-f` mistakes.
    match args.format.as_deref() {
        Some("best") => validation_warning(
            0,
            [
                "\"-f best\" selects the best pre-merged format which is often not the best option",
                "To let yt-dlp download and merge the best available formats, simply do not pass any format selection",
                "If you know what you are doing and want only the best pre-merged format, use \"-f b\" instead to suppress this warning",
            ]
            .join(".\n         "),
        ),
        Some("mp4") => validation_warning(
            1,
            [
                "\"-f mp4\" selects the best pre-merged mp4 format which is often not what's intended",
                "Pre-merged mp4 formats are not available from all sites, or may only be available in lower quality",
                "To prioritize the best h264 video and aac audio in an mp4 container, use \"-t mp4\" instead",
                "If you know what you are doing and want a pre-merged mp4 format, use \"-f b[ext=mp4]\" instead to suppress this warning",
            ]
            .join(".\n         "),
        ),
        _ => {}
    }
    if let Some(format) = &args.format {
        validate_format_selector(format)?;
    }
    // Upstream `report_conflict` between the playlist ordering options.
    let mut playlist_reverse = args.playlist_reverse;
    let mut playlist_random = args.playlist_random;
    if playlist_random && playlist_reverse {
        validation_warning(
            4,
            "--playlist-reverse is ignored since --playlist-random was given".into(),
        );
        playlist_reverse = false;
    }
    if args.lazy_playlist {
        for (rank, enabled, name) in [
            (5, &mut playlist_reverse, "--playlist-reverse"),
            (6, &mut playlist_random, "--playlist-random"),
        ] {
            if *enabled {
                validation_warning(
                    rank,
                    format!("{name} is ignored since --lazy-playlist was given"),
                );
                *enabled = false;
            }
        }
    }
    let playlist_selection = PlaylistSelection::new(
        args.playlist_items.as_deref(),
        args.playlist_start,
        args.playlist_end,
        playlist_reverse,
    )
    .map(|selection| {
        selection
            .random(playlist_random)
            .lazy(args.lazy_playlist)
            .selection_index(compat.iter().any(|name| name == "playlist-index"))
    })
    .map_err(|error| -> Box<dyn std::error::Error> {
        match (&error, &args.playlist_items) {
            // Upstream validates start/end, then parses the item segments.
            (yt_dlp_rs::Error::PlaylistSelection(message), _)
                if message.starts_with("playlist ") =>
            {
                usage(message).into()
            }
            (yt_dlp_rs::Error::PlaylistSelection(message), Some(items))
                if message.ends_with("is not a valid specification")
                    || message.ends_with("cannot be zero")
                    || message.ends_with("consecutive commas") =>
            {
                usage(format!(
                    "Invalid playlist-items {}: {message}",
                    yt_dlp_rs::batch::py_repr(items)
                ))
                .into()
            }
            _ => error.into(),
        }
    })?;
    if args.playlist_items.is_some()
        && (args.playlist_start != 1 || args.playlist_end.is_some_and(|end| end != -1 && end != 0))
    {
        warning!(
            "WARNING: Ignoring --playlist-start and --playlist-end because --playlist-items was given"
        );
    }
    let subtitle_languages = (!args.sub_langs.is_empty()).then(|| args.sub_langs.join(","));
    let subtitle_options = SubtitleOptions::new(subtitle_languages.as_deref(), &args.sub_format)?;
    if let (Some(cookies), Some(archive)) = (&args.cookies, &args.download_archive) {
        let identical = std::path::absolute(cookies)? == std::path::absolute(archive)?;
        let canonical_alias = std::fs::canonicalize(cookies)
            .ok()
            .zip(std::fs::canonicalize(archive).ok())
            .is_some_and(|(left, right)| left == right);
        if identical || canonical_alias {
            return Err(yt_dlp_rs::Error::Cookies(
                "cookie file and download archive must have different paths".into(),
            )
            .into());
        }
    }
    let cookie_file_was_missing = args.cookies.as_ref().is_some_and(|path| !path.exists());
    let browser_cookies = args
        .cookies_from_browser
        .as_deref()
        .filter(|_| !args.no_cookies_from_browser)
        .map(yt_dlp_rs::browser_cookies::BrowserSpec::parse)
        .transpose()
        .map_err(usage)?;
    let mut http_options = HttpOptions::from_cli(args.user_agent, args.referer, &args.add_headers)?;
    // Upstream `std_headers` precede `--add-headers` (which override them).
    http_options.headers.splice(0..0, std_headers());
    let args_proxy = args.proxy.clone();
    http_options.proxy = args.proxy;
    http_options.socket_timeout = args.socket_timeout;
    http_options.print_traffic = args.print_traffic;
    http_options.no_check_certificates = args.no_check_certificates;
    http_options.client_certificate = args.client_certificate.clone();
    http_options.client_certificate_key = args.client_certificate_key.clone();
    http_options.client_certificate_password = args.client_certificate_password.clone();
    // Upstream -4/-6 store 0.0.0.0/:: into the shared source_address option.
    http_options.source_address = if args.force_ipv4 {
        Some("0.0.0.0".into())
    } else if args.force_ipv6 {
        Some("::".into())
    } else {
        args.source_address
    };
    http_options.validate()?;
    let external_source_address = http_options.source_address.clone();
    if let Some(spec) = &browser_cookies {
        // Upstream `load_cookies`: browser cookies, then the cookie file's.
        let extracted = yt_dlp_rs::browser_cookies::extract(spec);
        http_options.cookies = Some(Arc::new(CookieJar::from_browser(
            extracted,
            args.cookies.as_deref(),
        )?));
    } else if let Some(path) = &args.cookies {
        http_options.cookies = Some(Arc::new(CookieJar::load(path)?));
    }
    let mut downloader = YoutubeDl::with_options(
        YoutubeOptions {
            playback_format: None,
            playback_visitor_data: None,
            // `--js-runtimes deno[:PATH]`/`node[:PATH]` (`deno` is enabled by
            // default; `--no-js-runtimes` clears it): the last value for each,
            // a bare name looked up on PATH. Upstream prefers Deno's provider
            // to Node.js's.
            js_runtime: args.youtube_js_runtime.or_else(|| {
                let mut enabled: Vec<&str> = if args.no_js_runtimes {
                    Vec::new()
                } else {
                    vec!["deno"]
                };
                enabled.extend(args.js_runtimes.iter().map(String::as_str));
                ["deno", "node"].into_iter().find_map(|runtime| {
                    let value = enabled.iter().rev().find(|value| {
                        let name = value.split_once(':').map_or(**value, |(name, _)| name);
                        name.eq_ignore_ascii_case(runtime)
                    })?;
                    let path = value.split_once(':').map_or("", |(_, path)| path);
                    if !path.is_empty() {
                        return Some(PathBuf::from(path));
                    }
                    std::env::var_os("PATH").and_then(|paths| {
                        std::env::split_paths(&paths)
                            .map(|dir| {
                                dir.join(format!("{runtime}{}", std::env::consts::EXE_SUFFIX))
                            })
                            .find(|candidate| candidate.is_file())
                    })
                })
            }),
            include_translated_subtitles: args.write_auto_subs || args.list_subs,
            no_playlist: args.no_playlist,
        },
        http_options,
    )?;
    downloader = downloader.with_subtitles_requested(args.write_subs || args.list_subs);
    let (ffmpeg, ffprobe) = ffmpeg_executables(args.ffmpeg_location.as_deref());
    if let Some(path) = ffmpeg {
        downloader = downloader.with_ffmpeg(path);
    }
    let ffprobe = args.ffprobe_location.or(ffprobe);
    if args.verbose {
        print_debug_header(
            QUIET.load(Ordering::Relaxed),
            downloader.ffmpeg_path(),
            ffprobe.as_deref(),
            args_proxy.as_deref(),
            &compat_shown,
            args.encoding.as_deref(),
        );
        // Upstream's request director (`Request Handlers`) loads the jar.
        if let Err(error) = yt_dlp_rs::screen::load_cookies() {
            report_error!("ERROR: {error}");
            return Ok(false);
        }
    }
    if let Some(path) = ffprobe {
        downloader = downloader.with_ffprobe(path);
    }
    downloader = downloader.with_format_sort(yt_dlp_rs::FormatSortOptions {
        fields: format_sort,
        force: args.format_sort_force,
        prefer_free: args.prefer_free_formats,
    });
    downloader = downloader.with_multistreams(args.video_multistreams, args.audio_multistreams);
    if let Some(formats) = &args.merge_output_format {
        downloader =
            downloader.with_merge_output_format(formats.split('/').map(str::to_owned).collect());
    }
    let archive_path = args.download_archive.clone();
    let mut archive = args
        .download_archive
        .map(DownloadArchive::load)
        .transpose()?;
    let windows_filenames = if args.windows_filenames {
        Some(true)
    } else if args.no_windows_filenames {
        Some(false)
    } else {
        None
    };
    // Upstream `overwrites`: None (default), Some(false) for -w, Some(true) forced.
    let overwrites = if args.force_overwrites {
        Some(true)
    } else if args.no_overwrites {
        Some(false)
    } else {
        None
    };
    // Upstream `parse_chapters('--download-sections', ..., advanced=True)`.
    let download_sections =
        yt_dlp_rs::sections::DownloadSections::parse(&args.download_sections).map_err(usage)?;
    if args.concurrent_fragments <= 0 {
        return Err(usage(format!(
            "concurrent fragments \"{}\" must be positive",
            args.concurrent_fragments
        ))
        .into());
    }
    // Upstream `validate_positive` for the autonumber options.
    if args.autonumber_start < 0 {
        return Err(usage(format!(
            "autonumber start \"{}\" must be positive or 0",
            args.autonumber_start
        ))
        .into());
    }
    if let Some(size) = args.autonumber_size.filter(|size| *size <= 0) {
        return Err(usage(format!("autonumber size \"{size}\" must be positive")).into());
    }
    // Upstream `--id`: the default template becomes `%(id)s.%(ext)s` unless
    // `-o` sets one (`report_conflict`).
    if args.id {
        if OutputTemplates::sets_default(&args.output) {
            validation_warning(10, "--id is ignored since --output was given".into());
        } else {
            warning!("WARNING: --id is deprecated. Use -o \"%(id)s.%(ext)s\" instead");
            args.output.push("%(id)s.%(ext)s".into());
        }
    }
    let templates = OutputTemplates::parse(&args.output, args.restrict_filenames);
    LOG_TO_STDERR.store(templates.get(None) == "-", Ordering::Relaxed);
    let downloader = downloader
        .with_prefer_best_format(templates.get(None) == "-")
        .with_external_quiet(QUIET.load(Ordering::Relaxed))
        .with_hls_use_mpegts(if args.hls_use_mpegts {
            Some(true)
        } else if args.no_hls_use_mpegts {
            Some(false)
        } else {
            None
        })
        .with_live_from_start(args.live_from_start);
    let output_paths = yt_dlp_rs::output::parse_paths(&args.paths)?;
    let template_options = TemplateOptions {
        na_placeholder: args.output_na_placeholder,
        filename_mode: TemplateOptions::filename_mode(args.restrict_filenames, windows_filenames),
        windows_filenames: windows_filenames == Some(true),
        trim_file_name: args.trim_filenames,
        paths: output_paths,
        autonumber: Default::default(),
        autonumber_size: args.autonumber_size.map_or(5, |size| size as usize),
        video_autonumber: Default::default(),
        legacy_sanitization: compat.iter().any(|name| name == "filename-sanitization"),
        allow_unsafe_exec: compat
            .iter()
            .any(|name| name == "allow-unsafe-exec-expansion"),
        list_formats_old: args.list_formats_old,
    };
    // Upstream `exec_cmd`: --no-exec clears earlier values; the hidden
    // --exec-before-download is ignored when "--exec before_dl:" is given.
    let exec_values = after_reset("exec", "no_exec");
    let before_download = after_reset("exec_before_download", "no_exec_before_download");
    // Upstream `--use-postprocessor NAME[:ARGS]` definitions come before
    // the other postprocessors of their stage.
    let mut added_pps: Vec<(String, String)> = Vec::new();
    for value in &args.use_postprocessor {
        let (name, rest) = value.split_once(':').unwrap_or((value, ""));
        let key = name.strip_suffix("PP").unwrap_or(name);
        let mut when = "post_process".to_owned();
        let mut exec_cmd = None;
        for item in rest.split(';').filter(|_| !rest.is_empty()) {
            let (field, setting) = item.split_once('=').ok_or_else(|| {
                usage(format!(
                    "invalid --use-postprocessor argument \"{item}\" (expected NAME=VALUE)"
                ))
            })?;
            match field {
                "when" => when = setting.to_owned(),
                "exec_cmd" if key == "Exec" => exec_cmd = Some(setting.to_owned()),
                other => {
                    return Err(yt_dlp_rs::Error::Reported(format!(
                        "postprocessor argument {other:?} of {key} is not supported natively"
                    ))
                    .into());
                }
            }
        }
        match (key, exec_cmd) {
            ("Exec", Some(command)) => added_pps.push((when, command)),
            ("Exec", None) => {
                return Err(yt_dlp_rs::Error::Reported(
                    "the Exec postprocessor needs exec_cmd=COMMAND".into(),
                )
                .into());
            }
            (key, _) => {
                return Err(yt_dlp_rs::Error::Reported(format!(
                    "postprocessor {key:?} is not available natively (plugins are not supported)"
                ))
                .into());
            }
        }
    }
    let mut exec_specs: Vec<(Option<&str>, &str)> = added_pps
        .iter()
        .map(|(when, command)| (Some(when.as_str()), command.as_str()))
        .chain(exec_values.iter().map(|value| (None, value.as_str())))
        .collect();
    let mut exec = ExecCommands::parse(&exec_specs, &template_options)?;
    if !before_download.is_empty() {
        if exec.has(Stage::BeforeDownload) {
            validation_warning(
                9,
                "--exec-before-download is ignored since \"--exec before_dl:\" was given".into(),
            );
        } else {
            exec_specs.extend(
                before_download
                    .iter()
                    .map(|value| (Some("before_dl"), value.as_str())),
            );
            exec = ExecCommands::parse(&exec_specs, &template_options)?;
        }
    }
    // Like upstream, simulation never writes info JSON.
    // Upstream: comments without a JSON destination imply --write-info-json.
    let comments_info_json = args.write_comments
        && !args.write_info_json
        && !args.no_write_info_json
        && !args.embed_info_json
        && (!args.dump_json || args.print_json || args.dump_single_json);
    let writes_info_json = (args.write_info_json || comments_info_json) && !simulate;
    // Upstream `embed_infojson`: explicit, or `if_exists` with --embed-metadata.
    let embed_info_json = if args.embed_info_json {
        Some(true)
    } else if args.no_embed_info_json || !args.embed_metadata {
        None
    } else {
        Some(false)
    };
    let writes_description = args.write_description && !simulate;
    // Upstream `write_links`: --write-link picks the platform's type.
    let platform_link = if cfg!(target_os = "macos") {
        "webloc"
    } else if cfg!(target_os = "linux") {
        "desktop"
    } else {
        "url"
    };
    let link_kinds: Vec<&str> = [
        ("url", args.write_url_link),
        ("webloc", args.write_webloc_link),
        ("desktop", args.write_desktop_link),
    ]
    .into_iter()
    .filter(|(kind, enabled)| *enabled || (args.write_link && *kind == platform_link))
    .map(|(kind, _)| kind)
    .collect();
    let writes_links = !link_kinds.is_empty() && !simulate;
    // Upstream --embed-thumbnail writes the thumbnail when not otherwise asked
    // and then deletes it after embedding.
    let keeps_thumbnails = thumbnail_mode != 0;
    let thumbnail_mode = if args.embed_thumbnail {
        thumbnail_mode.max(1)
    } else {
        thumbnail_mode
    };
    let writes_thumbnails = thumbnail_mode != 0 && !simulate;
    let thumbnail_conversion = args
        .convert_thumbnails
        .as_deref()
        .map(|format| {
            ThumbnailConversion::from_cli(format)
                .map_err(|_| usage(format!("invalid thumbnail format \"{format}\" given")))
        })
        .transpose()?
        .flatten();
    let playlist_files = !args.no_write_playlist_metafiles;
    let sleep = SleepPolicy::from_cli(
        args.sleep_interval,
        args.max_sleep_interval,
        args.sleep_subtitles,
        args.sleep_requests,
    )
    .map_err(usage)?;
    if let Some(seconds) = args.sleep_requests {
        yt_dlp_rs::screen::set_sleep_requests(seconds);
    }
    // Upstream `parse_retries` and `parse_sleep_func`.
    let parse_retries = |name: &str, value: &str| -> Result<u32, UsageError> {
        let text = yt_dlp_rs::batch::py_strip(value);
        if matches!(text, "inf" | "infinite") {
            return Ok(u32::MAX);
        }
        let digits = text.strip_prefix('+').unwrap_or(text);
        let negative = digits.starts_with('-');
        let magnitude = digits.trim_start_matches('-');
        if magnitude.is_empty()
            || digits.matches('-').count() > 1
            || !magnitude.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(usage(format!(
                "invalid {name} retry count \"{value}\" given"
            )));
        }
        if negative && magnitude.bytes().any(|b| b != b'0') {
            return Err(usage(format!(
                "{name} retry count \"{}\" must be positive or 0",
                text.trim_start_matches('+')
            )));
        }
        Ok(magnitude
            .parse::<u64>()
            .map_or(u32::MAX - 1, |n| n.min(u64::from(u32::MAX - 1)) as u32))
    };
    let retries = parse_retries("download", &args.retries)?;
    let fragment_retries = parse_retries("fragment", &args.fragment_retries)?;
    let mut retry_sleep = yt_dlp_rs::RetrySleep::default();
    for spec in &args.retry_sleep {
        let (key, expression) = match spec.split_once(':') {
            Some((key, rest))
                if ["http", "fragment", "file_access", "extractor"]
                    .iter()
                    .any(|name| name.eq_ignore_ascii_case(key)) =>
            {
                (key.to_ascii_lowercase(), rest)
            }
            _ => ("http".to_owned(), spec.as_str()),
        };
        let parsed = if expression.is_empty() {
            None
        } else {
            Some(
                yt_dlp_rs::SleepExpression::parse(expression).ok_or_else(|| {
                    usage(format!(
                        "invalid {key} retry sleep expression {}",
                        yt_dlp_rs::batch::py_repr(expression)
                    ))
                })?,
            )
        };
        match key.as_str() {
            "http" => retry_sleep.http = parsed,
            "fragment" => retry_sleep.fragment = parsed,
            // File access and extractor retries are not represented.
            _ => {}
        }
    }
    let skip_unavailable_fragments = Some(!args.abort_on_unavailable_fragments);
    // Upstream `continuedl` and `nopart` for whole-file media downloads.
    let part_files = if args.no_part {
        yt_dlp_rs::PartFiles::NoPart {
            resume: !args.no_continue,
        }
    } else {
        yt_dlp_rs::PartFiles::Part {
            resume: !args.no_continue,
        }
    };
    let min_filesize = size_option("min filesize", args.min_filesize.as_deref()).map_err(usage)?;
    let max_filesize = size_option("max filesize", args.max_filesize.as_deref()).map_err(usage)?;
    // Upstream validates the rate limit as strictly positive.
    let rate_limit = size_option("rate limit", args.limit_rate.as_deref()).map_err(usage)?;
    if rate_limit == Some(0) {
        return Err(usage("rate limit \"0\" must be positive").into());
    }
    let http_chunk_size =
        size_option("http chunk size", args.http_chunk_size.as_deref()).map_err(usage)?;
    // Upstream `--downloader` (`_dict_from_options_callback`) validation.
    let mut downloader = downloader;
    let mut default_downloader: Option<String> = None;
    for value in &args.downloader {
        static KEYS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(r"(?is)^(?P<keys>(?:http|ftp|m3u8|dash|rtmp)(?:,(?:http|ftp|m3u8|dash|rtmp))*):(?P<val>.*)$")
                .unwrap()
        });
        let (keys, name): (Vec<String>, &str) = match KEYS.captures(value) {
            Some(captures) => (
                captures["keys"].split(',').map(str::to_lowercase).collect(),
                captures.name("val").unwrap().as_str(),
            ),
            None => (vec!["default".into()], value.as_str()),
        };
        let name = name.trim();
        if keys == ["default"] {
            default_downloader = Some(name.to_owned());
        }
        // `get_external_downloader`: a name or a path to one.
        let base = std::path::Path::new(name)
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if name != "native" && !(base.contains("ffmpeg") || base.contains("avconv")) {
            let known = ["aria2c", "axel", "curl", "httpie", "http", "wget"]
                .iter()
                .any(|exe| base.contains(exe));
            let proto = keys
                .first()
                .filter(|key| *key != "default")
                .map_or_else(String::new, |key| format!("{key} "));
            if !known {
                return Err(usage(format!("No such {proto}external downloader \"{name}\"")).into());
            }
        }
        for key in keys {
            downloader = downloader.with_downloader_name(&key, name);
        }
    }
    // Upstream `--downloader-args` (shlex-split, by downloader key).
    let mut downloader_args: Vec<(String, Vec<String>)> = Vec::new();
    for value in &args.downloader_args {
        static KEYS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(r"(?is)^(?P<keys>(?:ffmpeg_[io]\d*|aria2c|avconv|axel|curl|ffmpeg|httpie|wget)(?:,(?:ffmpeg_[io]\d*|aria2c|avconv|axel|curl|ffmpeg|httpie|wget))*):(?P<val>.*)$")
                .unwrap()
        });
        let (keys, text): (Vec<String>, &str) = match KEYS.captures(value) {
            Some(captures) => (
                captures["keys"].split(',').map(str::to_lowercase).collect(),
                captures.name("val").unwrap().as_str(),
            ),
            None => (vec!["default".into()], value.as_str()),
        };
        let parts = yt_dlp_rs::config::shlex_split(text)
            .map_err(|error| usage(format!("wrong --downloader-args formatting; {error}")))?;
        for key in keys {
            downloader_args.retain(|(name, _)| *name != key);
            downloader_args.push((key, parts.clone()));
        }
    }
    // Upstream `report_args_compat`: unnamed arguments go to the default
    // downloader (when one is given).
    if let Some(index) = downloader_args.iter().position(|(key, _)| key == "default") {
        validation_warning(
            2,
            format!(
                "External Downloader arguments given without specifying name. The arguments will be given to {}",
                default_downloader
                    .as_deref()
                    .unwrap_or("all external downloaders")
            ),
        );
        if let Some(name) = &default_downloader {
            let (_, parts) = downloader_args.remove(index);
            if !downloader_args.iter().any(|(key, _)| key == name) {
                downloader_args.push((name.clone(), parts));
            }
        }
    }
    for (key, parts) in downloader_args {
        downloader = downloader.with_downloader_args(&key, parts);
    }
    // The external downloaders' `params`.
    downloader = downloader.with_external_settings(yt_dlp_rs::ExternalSettings {
        no_progress: if args.no_progress {
            true
        } else if args.progress {
            false
        } else {
            QUIET.load(Ordering::Relaxed)
        },
        continue_download: !args.no_continue,
        update_time: if args.mtime {
            Some(true)
        } else if args.no_mtime {
            Some(false)
        } else {
            None
        },
        source_address: external_source_address,
        no_check_certificate: args.no_check_certificates,
        cookie_file: args.cookies.clone(),
        ..Default::default()
    });
    // Upstream `--postprocessor-args` (shlex-split, by postprocessor key).
    let mut postprocessor_args: std::collections::HashMap<String, Vec<String>> = Default::default();
    // optparse names the option as given.
    let postprocessor_args_flag =
        if std::env::args().any(|arg| arg == "--ppa" || arg.starts_with("--ppa=")) {
            "--ppa"
        } else {
            "--postprocessor-args"
        };
    for value in &args.postprocessor_args {
        static KEYS: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(r"(?is)^(?P<key>\w+(?:\+\w+)?):(?P<val>.*)$").unwrap()
        });
        let (key, text) = match KEYS.captures(value) {
            Some(captures) => (
                captures["key"].to_lowercase(),
                captures.name("val").unwrap().as_str(),
            ),
            None => ("default-compat".to_owned(), value.as_str()),
        };
        let parts = yt_dlp_rs::config::shlex_split(text).map_err(|error| {
            usage(format!(
                "wrong {postprocessor_args_flag} formatting; {error}"
            ))
        })?;
        postprocessor_args.insert(key, parts);
    }
    if postprocessor_args.contains_key("default-compat")
        && !postprocessor_args.contains_key("default")
    {
        validation_warning(
            3,
            "Post-Processor arguments given without specifying name. The arguments will be given to all post-processors".into(),
        );
        let parts = postprocessor_args
            .remove("default-compat")
            .unwrap_or_default();
        postprocessor_args.insert("default".into(), parts);
    }
    yt_dlp_rs::pp_args::set(postprocessor_args);
    flush_validation_warnings();
    if args.verbose {
        // `html5`, `youtube` and `generic` (see `--list-extractors`).
        eprintln!("[debug] Loaded 62 extractors");
        if let Some(path) = &archive_path {
            eprintln!(
                "[debug] Loading archive file {}",
                yt_dlp_rs::batch::py_repr(&path.to_string_lossy())
            );
        }
    }
    let downloader = downloader
        .with_http_chunk_size(http_chunk_size)
        .with_test(args.test)
        .with_force_generic_extractor(args.force_generic_extractor)
        .with_concurrent_fragments(args.concurrent_fragments as usize)
        .with_keep_fragments(args.keep_fragments)
        .with_file_urls(args.enable_file_urls)
        .with_check_formats(if args.check_formats {
            yt_dlp_rs::CheckFormats::Selected
        } else if args.check_all_formats {
            yt_dlp_rs::CheckFormats::All
        } else if args.no_check_formats {
            yt_dlp_rs::CheckFormats::Never
        } else {
            yt_dlp_rs::CheckFormats::Default
        })
        .with_default_search(args.default_search.clone())
        // Upstream `_list_from_options_callback`: comma-separated, stripped.
        .with_allowed_extractors(
            &args
                .use_extractors
                .iter()
                .flat_map(|value| value.split(','))
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        )
        .map_err(usage)?;
    // Upstream `--color` validation; output is always uncolored.
    for value in &args.color {
        let policy = match value.split_once(':') {
            Some((stream, policy))
                if matches!(stream.to_lowercase().as_str(), "stdout" | "stderr") =>
            {
                policy
            }
            _ => value.as_str(),
        }
        .trim();
        if !matches!(
            policy,
            "always" | "auto" | "auto-tty" | "no_color" | "no_color-tty" | "never"
        ) {
            return Err(usage(format!("\"{policy}\" is not a valid color policy")).into());
        }
    }
    if args.update || args.update_to.is_some() {
        report_error!("ERROR: yt-dlp-rs cannot update itself; install a newer release instead");
        return Ok(false);
    }
    // Upstream `Cache.remove` (this port stores no cache).
    if args.rm_cache_dir {
        if args.no_cache_dir {
            screen!("Cache is disabled (Did you combine --no-cache-dir and --rm-cache-dir?)");
        } else {
            let directory = args
                .cache_dir
                .clone()
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    std::env::var_os("XDG_CACHE_HOME")
                        .map(PathBuf::from)
                        .or_else(|| {
                            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache"))
                        })
                        .unwrap_or_default()
                        .join("yt-dlp-rs")
                });
            let mut line = format!("Removing cache dir {} .", directory.display());
            if directory.is_dir() {
                std::fs::remove_dir_all(&directory)?;
                line.push('.');
            }
            line.push('.');
            screen!("{line}");
        }
        if urls.is_empty() && args.load_info_json.is_none() {
            return Ok(true);
        }
    }
    if args.list_impersonate_targets {
        screen!("[info] Available impersonate targets");
        writeln!(
            io::stdout().lock(),
            "Client    OS   Source\n--------------------------------------------\n\
             Tor       -    curl_cffi>=0.11 (unavailable)\nEdge      -    curl_cffi (unavailable)\n\
             Firefox   -    curl_cffi>=0.10 (unavailable)\nSafari    -    curl_cffi (unavailable)\n\
             Chrome    -    curl_cffi (unavailable)"
        )?;
        return Ok(true);
    }
    if args.ap_list_mso {
        // No supported extractor uses Adobe Pass.
        writeln!(
            io::stdout().lock(),
            "Supported TV Providers:\nmso  mso name"
        )?;
        return Ok(true);
    }
    if let Some(target) = &args.impersonate {
        report_error!(
            "ERROR: Impersonate target \"{}\" is not available. Use --list-impersonate-targets to see available targets. You may be missing dependencies required to support this target.",
            target.split(':').next().unwrap_or(target).to_lowercase()
        );
        return Ok(false);
    }
    // Upstream `print_extractor_information`, for this port's extractors
    // (sorted, `generic` last); the generic extractor lists unmatched URLs.
    if args.list_extractors {
        let mut out = io::stdout().lock();
        // Sorted by `IE_NAME.lower()`, generic last; each URL under the
        // first extractor that is suitable.
        let mut remaining: Vec<&String> = urls.iter().collect();
        type Suitable = fn(&str) -> bool;
        let suitable: [(&str, Suitable); 110] = [
            ("23video", yt_dlp_rs::extractor::twentythreevideo_suitable),
            ("Angel", yt_dlp_rs::extractor::angel_suitable),
            ("archive.org", yt_dlp_rs::extractor::archiveorg_suitable),
            ("AudioBoom", yt_dlp_rs::extractor::audioboom_suitable),
            ("Bandcamp", yt_dlp_rs::extractor::bandcamp_suitable),
            (
                "BehindKink (CURRENTLY BROKEN)",
                yt_dlp_rs::extractor::behindkink_suitable,
            ),
            ("Bild", yt_dlp_rs::extractor::bild_suitable),
            ("BiliBili", yt_dlp_rs::extractor::bilibili_suitable),
            ("BitChute", yt_dlp_rs::extractor::bitchute_suitable),
            ("Bitmovin", yt_dlp_rs::extractor::bitmovin_suitable),
            ("blogger.com", yt_dlp_rs::extractor::blogger_suitable),
            ("Bluesky", yt_dlp_rs::extractor::bluesky_suitable),
            ("BreitBart", yt_dlp_rs::extractor::breitbart_suitable),
            ("CGTN", yt_dlp_rs::extractor::cgtn_suitable),
            ("CJSW", yt_dlp_rs::extractor::cjsw_suitable),
            ("Coub", yt_dlp_rs::extractor::coub_suitable),
            ("CozyTV", yt_dlp_rs::extractor::cozytv_suitable),
            (
                "CrooksAndLiars",
                yt_dlp_rs::extractor::crooksandliars_suitable,
            ),
            ("Crtvg", yt_dlp_rs::extractor::crtvg_suitable),
            (
                "CultureUnplugged",
                yt_dlp_rs::extractor::cultureunplugged_suitable,
            ),
            ("dailymotion", yt_dlp_rs::extractor::dailymotion_suitable),
            ("EbaumsWorld", yt_dlp_rs::extractor::ebaumsworld_suitable),
            ("Ebay", yt_dlp_rs::extractor::ebay_suitable),
            ("Epoch", yt_dlp_rs::extractor::epoch_suitable),
            ("facebook", yt_dlp_rs::extractor::facebook_suitable),
            (
                "facebook:reel",
                yt_dlp_rs::extractor::facebook_reel_suitable,
            ),
            (
                "FacebookPluginsVideo",
                yt_dlp_rs::extractor::facebook_plugins_suitable,
            ),
            ("Fathom", yt_dlp_rs::extractor::fathom_suitable),
            ("Fczenit", yt_dlp_rs::extractor::fczenit_suitable),
            ("Filmweb", yt_dlp_rs::extractor::filmweb_suitable),
            ("FuyinTV", yt_dlp_rs::extractor::fuyintv_suitable),
            ("Glide", yt_dlp_rs::extractor::glide_suitable),
            ("GodResource", yt_dlp_rs::extractor::godresource_suitable),
            // `_WORKING = False` extractors are marked.
            (
                "GodTube (CURRENTLY BROKEN)",
                yt_dlp_rs::extractor::godtube_suitable,
            ),
            ("Graspop", yt_dlp_rs::extractor::graspop_suitable),
            ("Harpodeon", yt_dlp_rs::extractor::harpodeon_suitable),
            (
                "HistoricFilms",
                yt_dlp_rs::extractor::historicfilms_suitable,
            ),
            ("HitRecord", yt_dlp_rs::extractor::hitrecord_suitable),
            ("html5", |_| false),
            ("Huajiao", yt_dlp_rs::extractor::huajiao_suitable),
            ("Hypem", yt_dlp_rs::extractor::hypem_suitable),
            ("Imgur", yt_dlp_rs::extractor::imgur_suitable),
            ("imgur:album", yt_dlp_rs::extractor::imgur_album_suitable),
            (
                "imgur:gallery",
                yt_dlp_rs::extractor::imgur_gallery_suitable,
            ),
            ("Instagram", yt_dlp_rs::extractor::instagram_suitable),
            ("Jove", yt_dlp_rs::extractor::jove_suitable),
            ("JWPlatform", yt_dlp_rs::extractor::jwplatform_suitable),
            ("kick:clips", yt_dlp_rs::extractor::kick_clip_suitable),
            ("kick:live", yt_dlp_rs::extractor::kick_live_suitable),
            ("kick:vod", yt_dlp_rs::extractor::kick_vod_suitable),
            ("Kommunetv", yt_dlp_rs::extractor::kommunetv_suitable),
            ("Ku6", yt_dlp_rs::extractor::ku6_suitable),
            ("lbry", yt_dlp_rs::extractor::lbry_suitable),
            ("LinkedIn", yt_dlp_rs::extractor::linkedin_suitable),
            (
                "Livestreamfails",
                yt_dlp_rs::extractor::livestreamfails_suitable,
            ),
            ("loom", yt_dlp_rs::extractor::loom_suitable),
            ("maariv.co.il", yt_dlp_rs::extractor::maariv_suitable),
            ("Masters", yt_dlp_rs::extractor::masters_suitable),
            ("megaphone.fm", yt_dlp_rs::extractor::megaphone_suitable),
            ("mixcloud", yt_dlp_rs::extractor::mixcloud_suitable),
            ("MovingImage", yt_dlp_rs::extractor::movingimage_suitable),
            ("niconico", yt_dlp_rs::extractor::niconico_suitable),
            ("NobelPrize", yt_dlp_rs::extractor::nobelprize_suitable),
            (
                "Odnoklassniki",
                yt_dlp_rs::extractor::odnoklassniki_suitable,
            ),
            ("OnePlacePodcast", yt_dlp_rs::extractor::oneplace_suitable),
            ("Parlview", yt_dlp_rs::extractor::parlview_suitable),
            ("Photobucket", yt_dlp_rs::extractor::photobucket_suitable),
            ("Pinterest", yt_dlp_rs::extractor::pinterest_suitable),
            ("PlayerFm", yt_dlp_rs::extractor::playerfm_suitable),
            ("PopcornTV", yt_dlp_rs::extractor::popcorntv_suitable),
            (
                "PremiershipRugby",
                yt_dlp_rs::extractor::premiershiprugby_suitable,
            ),
            ("QingTing", yt_dlp_rs::extractor::qingting_suitable),
            ("Reddit", yt_dlp_rs::extractor::reddit_suitable),
            ("ReverbNation", yt_dlp_rs::extractor::reverbnation_suitable),
            ("RTRFM", yt_dlp_rs::extractor::rtrfm_suitable),
            ("Rumble", yt_dlp_rs::extractor::rumble_suitable),
            ("RumbleEmbed", yt_dlp_rs::extractor::rumble_embed_suitable),
            ("rutube", yt_dlp_rs::extractor::rutube_suitable),
            ("rutube:embed", yt_dlp_rs::extractor::rutube_embed_suitable),
            ("ScreenRec", yt_dlp_rs::extractor::screenrec_suitable),
            ("Sen", yt_dlp_rs::extractor::sen_suitable),
            ("soundcloud", yt_dlp_rs::extractor::soundcloud_suitable),
            ("Streamable", yt_dlp_rs::extractor::streamable_suitable),
            ("SztvHu", yt_dlp_rs::extractor::sztvhu_suitable),
            ("TedEmbed", yt_dlp_rs::extractor::ted_embed_suitable),
            ("TedPlaylist", yt_dlp_rs::extractor::ted_playlist_suitable),
            ("TedSeries", yt_dlp_rs::extractor::ted_series_suitable),
            ("TedTalk", yt_dlp_rs::extractor::ted_talk_suitable),
            ("TheIntercept", yt_dlp_rs::extractor::theintercept_suitable),
            (
                "ThisAmericanLife",
                yt_dlp_rs::extractor::thisamericanlife_suitable,
            ),
            ("TikTok", yt_dlp_rs::extractor::tiktok_suitable),
            ("Tumblr", yt_dlp_rs::extractor::tumblr_suitable),
            ("Tweakers", yt_dlp_rs::extractor::tweakers_suitable),
            ("twitch:clips", yt_dlp_rs::extractor::twitch_clips_suitable),
            ("twitch:vod", yt_dlp_rs::extractor::twitch_vod_suitable),
            ("twitter", yt_dlp_rs::extractor::twitter_suitable),
            ("twitter:card", yt_dlp_rs::extractor::twitter_card_suitable),
            ("vimeo", yt_dlp_rs::extractor::vimeo_suitable),
            ("vk", yt_dlp_rs::extractor::vk_suitable),
            ("vm.tiktok", yt_dlp_rs::extractor::tiktok_vm_suitable),
            ("Vocaroo", yt_dlp_rs::extractor::vocaroo_suitable),
            ("Weibo", yt_dlp_rs::extractor::weibo_suitable),
            ("WeiboVideo", yt_dlp_rs::extractor::weibo_video_suitable),
            ("yfanefa", yt_dlp_rs::extractor::yfanefa_suitable),
            ("youtube", yt_dlp_rs::extractor::youtube_suitable),
            (
                "youtube:search",
                yt_dlp_rs::extractor::youtube_search_suitable,
            ),
            (
                "youtube:search_url",
                yt_dlp_rs::extractor::youtube_search_url_suitable,
            ),
            ("youtube:tab", yt_dlp_rs::extractor::youtube_tab_suitable),
            ("Zhihu", yt_dlp_rs::extractor::zhihu_suitable),
            ("generic", |_| true),
        ];
        for (name, matches) in suitable {
            writeln!(out, "{name}")?;
            let (matched, rest): (Vec<&String>, Vec<&String>) =
                remaining.into_iter().partition(|url| matches(url));
            for url in matched {
                writeln!(out, "  {url}")?;
            }
            remaining = rest;
        }
        return Ok(true);
    }
    if args.extractor_descriptions {
        // `description(search_examples=...)`: a random count and example.
        use std::hash::{BuildHasher, Hasher};
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(epoch());
        let random = hasher.finish();
        let count = ["", "5", "10", "all"][(random % 4) as usize];
        let example = [
            "cute kittens",
            "slithering pythons",
            "falling cat",
            "angry poodle",
            "purple fish",
            "running tortoise",
            "sleeping bunny",
            "burping cow",
        ][((random / 4) % 8) as usize];
        // Upstream writes the joined descriptions without a final newline.
        write!(
            io::stdout().lock(),
            "23video\nAngel\narchive.org: archive.org video and audio\nAudioBoom\nBandcamp\nBild\nBiliBili\nBitChute\nBitmovin\nblogger.com\nBluesky\nBreitBart\nCGTN\nCJSW\nCoub\nCozyTV\nCrooksAndLiars\nCrtvg\nCultureUnplugged\ndailymotion: [dailymotion]\nEbaumsWorld\nEbay\nEpoch\nfacebook\nfacebook:reel\nFacebookPluginsVideo\nFathom\nFczenit\nFilmweb\nFuyinTV\nGlide: Glide mobile video messages (glide.me)\nGodResource\nGraspop\nHarpodeon\nHistoricFilms\nHitRecord\nhtml5\nHuajiao: 花椒直播\nHypem\nImgur\nimgur:album\nimgur:gallery\nInstagram\nJove\nJWPlatform\nkick:clips\nkick:live\nkick:vod\nKommunetv\nKu6\nlbry: odysee.com\nLinkedIn\nLivestreamfails\nloom\nmaariv.co.il\nMasters\nmegaphone.fm: megaphone.fm embedded players\nmixcloud\nMovingImage\nniconico: [niconico] ニコニコ動画\nNobelPrize\nOdnoklassniki\nOnePlacePodcast\nParlview\nPhotobucket\nPinterest\nPlayerFm\nPopcornTV\nPremiershipRugby\nQingTing\nReddit\nReverbNation\nRTRFM\nRumble\nRumbleEmbed\nrutube: Rutube videos\nrutube:embed: Rutube embedded videos\nScreenRec\nSen\nsoundcloud: [soundcloud]\nStreamable\nSztvHu\nTedEmbed\nTedPlaylist\nTedSeries\nTedTalk\nTheIntercept\nThisAmericanLife\nTikTok\nTumblr: [tumblr]\nTweakers\ntwitch:clips: [twitch]\ntwitch:vod: [twitch]\ntwitter\ntwitter:card\nvimeo: [vimeo]\nvk: [vk] VK\nvm.tiktok\nVocaroo\nWeibo\nWeiboVideo\nyfanefa\nyoutube: [youtube] YouTube\nyoutube:search: [youtube] YouTube search; \"ytsearch:\" prefix (e.g. \"ytsearch{count}:{example}\")\nyoutube:search_url: [youtube] YouTube search URLs with sorting and filter support\nyoutube:tab: [youtube] YouTube Tabs\nZhihu\ngeneric: Generic downloader that works on some sites"
        )?;
        return Ok(true);
    }
    // Upstream reports a missing URL after validating the options.
    if urls.is_empty() && args.load_info_json.is_none() {
        return Err(UsageError(
            "You must provide at least one URL.\nType yt-dlp-rs --help to see a list of all options."
                .into(),
        )
        .into());
    }
    // Upstream compares against `float(max_downloads or 'inf')`.
    let max_downloads = args.max_downloads.filter(|&limit| limit != 0);
    let abort_on_error = error_mode == ErrorMode::Abort;
    let _console_title = ConsoleTitle::enter(simulate);
    let mut aborted = false;
    let mut processed: i64 = 0;
    let mut videos: u64 = 0;
    let mut cancelled: Option<Cancel> = None;
    let mut failed = false;
    // Upstream `_out_files.out`: JSON, listings and prints use stderr with
    // `logtostderr` (`-o -`), whose stdout carries the media.
    let mut stdout = out_stream();
    // Upstream `download_with_info_file` replaces the URLs.
    let inputs: Vec<(String, Option<ExtractResult>)> = match &args.load_info_json {
        Some(file) => {
            if !urls.is_empty() {
                warning!("WARNING: URLs are ignored due to --load-info-json");
            }
            let path = yt_dlp_rs::batch::expand_path_text(file);
            let mut loaded = yt_dlp_rs::load_info_json(std::path::Path::new(&path))?;
            // Upstream runs process_video_result (fills, sorting, IDs) again.
            for result in &mut loaded {
                downloader.process_result(result)?;
            }
            loaded
                .into_iter()
                .map(|result| (path.clone(), Some(result)))
                .collect()
        }
        None => urls.into_iter().map(|url| (url, None)).collect(),
    };
    let mut inputs: std::collections::VecDeque<_> = inputs.into();
    while let Some((url, loaded)) = inputs.pop_front() {
        // Upstream `download_with_info_file` retries a raised DownloadError
        // with the webpage URL.
        let from_info_json = matches!(loaded, Some(ExtractResult::Video(_)));
        let mut fallback: Option<(String, String)> = None;
        let options = ExtractionOptions {
            retries: args.extractor_retries,
        };
        let mut pending: Option<Cancel> = None;
        let extracted =
            (|| -> yt_dlp_rs::Result<Option<(ExtractResult, Option<PlaylistCounts>)>> {
                let mut result = match loaded {
                    // Loaded playlists have no entries (`sanitize_info`), so
                    // upstream's `EntryNotInPlaylist` falls back to the URL.
                    Some(ExtractResult::Playlist(playlist)) => {
                        let title = [&playlist.title, &playlist.id]
                            .into_iter()
                            .find(|text| !text.is_empty())
                            .map_or("<Untitled>", |text| text.as_str());
                        screen!("[download] Downloading playlist: {title}");
                        warning!(
                            "WARNING: The info failed to download: There are no entries; trying with URL {}",
                            playlist.webpage_url
                        );
                        downloader.extract_result_unselected_with_options(
                            playlist.webpage_url.as_str(),
                            options,
                        )?
                    }
                    Some(result) => result,
                    None => downloader.extract_result_unselected_with_options(&url, options)?,
                };
                let counts = playlist_selection.apply(&mut result);
                let result = if let Some(archive) = &archive {
                    let (result, reached) =
                        filter_archived(result, archive, args.break_on_existing)?;
                    if reached {
                        pending = Some(Cancel::Existing);
                    }
                    let Some(result) = result else {
                        return Ok(None);
                    };
                    result
                } else {
                    result
                };
                // Upstream `_match_entry` before selection: format fields are incomplete.
                let Some(mut result) = prefilter(result, &match_filters, &mut pending)? else {
                    return Ok(None);
                };
                if write_subs || args.write_auto_subs {
                    let select = |info: &mut yt_dlp_rs::VideoInfo| -> yt_dlp_rs::Result<()> {
                        let selection = subtitle_options.select_sources(
                            &info.subtitles,
                            &info.automatic_captions,
                            write_subs,
                            args.write_auto_subs,
                        )?;
                        for warning in selection.warnings {
                            warning!("WARNING: {}: {warning}", info.id);
                        }
                        info.requested_subtitles = ((write_subs && !info.subtitles.is_empty())
                            || (args.write_auto_subs && !info.automatic_captions.is_empty()))
                        .then_some(selection.requested);
                        Ok(())
                    };
                    match &mut result {
                        ExtractResult::Video(info) => select(info)?,
                        ExtractResult::Playlist(playlist) => {
                            for info in &mut playlist.entries {
                                select(info)?;
                            }
                        }
                    }
                }
                Ok(Some((result, counts)))
            })();
        let (extracted, counts) = match extracted {
            Ok(Some(result)) => result,
            Ok(None) if pending.is_some() => {
                match cancel(pending.take().unwrap(), args.break_per_input) {
                    Some(cancel) => {
                        cancelled = Some(cancel);
                        break;
                    }
                    None => {
                        processed = 0;
                        continue;
                    }
                }
            }
            Ok(None) => continue,
            Err(error) => {
                // `CookieLoadError` is re-raised and ends the run.
                if let yt_dlp_rs::Error::CookieLoad(message) = &error {
                    report_error!("ERROR: {message}");
                    failed = true;
                    break;
                }
                match &error {
                    yt_dlp_rs::Error::Reported(message) => report_error!("ERROR: {message}"),
                    error @ (yt_dlp_rs::Error::Webpage { .. }
                    | yt_dlp_rs::Error::ExtractorRequest { .. }
                    | yt_dlp_rs::Error::UnsupportedUrl(_)) => {
                        report_error!("ERROR: {error}")
                    }
                    _ => report_error!("ERROR: {url}: {error}"),
                }
                failed = true;
                if abort_on_error {
                    break;
                }
                // `ignoreerrors` returns `None` from `extract_info`, which
                // `-J` still dumps.
                if args.dump_single_json {
                    writeln!(stdout, "null")?;
                    stdout.flush()?;
                }
                continue;
            }
        };
        // Upstream `__process_playlist` progress lines use the playlist
        // title (or ID).
        let playlist_title = match &extracted {
            ExtractResult::Playlist(playlist) => Some(
                [&playlist.title, &playlist.id]
                    .into_iter()
                    .find(|text| !text.is_empty())
                    .map_or("<Untitled>".to_owned(), |text| text.to_string()),
            ),
            ExtractResult::Video(_) => None,
        };
        if let Some(title) = &playlist_title {
            screen!("[download] Downloading playlist: {title}");
        }
        // The playlist info JSON written before the entries, rewritten after
        // them (`_infojson_written is True`).
        let mut playlist_info_json: Option<(PathBuf, serde_json::Value)> = None;
        if let (ExtractResult::Playlist(playlist), Some(counts), true) = (
            &extracted,
            counts,
            playlist_files && (writes_info_json || writes_description),
        ) {
            // Upstream writes playlist metadata before processing any entry,
            // and a failed write skips the playlist.
            let written = (|| -> Result<(), Box<dyn std::error::Error>> {
                if writes_info_json
                    && let Some(path) = templates.prepare_playlist(
                        playlist,
                        counts,
                        "pl_infojson",
                        &template_options,
                    )?
                {
                    reject_reserved(&path, [&archive_path, &args.cookies])?;
                    let value = yt_dlp_rs::playlist_info_json_with(
                        playlist,
                        counts,
                        epoch(),
                        !args.no_clean_info_json,
                    )?;
                    let written = write_info_json_with(&path, &value, overwrites != Some(false))?;
                    if matches!(written, InfoJsonWrite::Written) {
                        playlist_info_json = Some((path.clone(), value));
                    }
                    report_info_json("playlist", &path, written);
                }
                if writes_description
                    && templates
                        .prepare_playlist(playlist, counts, "pl_description", &template_options)?
                        .is_some()
                {
                    // Playlist results carry no represented description.
                    screen!("[info] There's no playlist description to write");
                }
                Ok(())
            })();
            if let Err(error) = written {
                report_error!("ERROR: {url}: {error}");
                failed = true;
                if abort_on_error {
                    break;
                }
                continue;
            }
        }
        if let (ExtractResult::Playlist(playlist), true) =
            (&extracted, args.list_thumbnails && playlist_files)
        {
            // Playlist results carry no represented thumbnails.
            screen!("{} has no thumbnails", playlist.id);
        }
        if let (ExtractResult::Playlist(playlist), Some(counts), Some(title)) =
            (&extracted, counts, &playlist_title)
        {
            screen!(
                "[{}] Playlist {title}: Downloading {} items of {}",
                playlist.extractor,
                counts
                    .n_entries
                    .map_or_else(|| "N/A".into(), |count| count.to_string()),
                counts.playlist_count
            );
            // Upstream keeps resolved entries unless `extract_flat` became
            // `discard`/`discard_in_playlist` (nothing needs the playlist's
            // entries but the default multi-video concatenation).
            let keeps_entries = args.flat_playlist
                || args.dump_single_json
                || prints_stage(Stage::Playlist)
                || (args.write_info_json && args.no_clean_info_json && playlist_files)
                || exec.has(Stage::Playlist)
                || args.concat_playlist == "always";
            if keeps_entries && yt_dlp_rs::screen::verbose() {
                eprintln!("[debug] The information of all playlist entries will be held in memory");
            }
        }
        // Upstream's playlist dict for the `playlist` stage.
        let playlist_dict = playlist_title
            .is_some()
            .then(|| single_json(&extracted, counts))
            .transpose()?;
        // Upstream `-J` prints the processed entries (`null` for entries whose
        // processing failed) after the playlist.
        let mut single_json = args
            .dump_single_json
            .then(|| single_json(&extracted, counts))
            .transpose()?;
        let mut json_entries = single_json
            .as_mut()
            .and_then(|value| value.get_mut("entries"))
            .map(serde_json::Value::take);
        // Unresolved `url_transparent` entries are `null` unless processed.
        if let (Some(serde_json::Value::Array(entries)), ExtractResult::Playlist(playlist)) =
            (&mut json_entries, &extracted)
        {
            for (entry, info) in entries.iter_mut().zip(&playlist.entries) {
                if YoutubeDl::is_transparent(info) {
                    *entry = serde_json::Value::Null;
                }
            }
        }
        // Each entry's downloaded file (`requested_downloads[0].filepath`).
        let entry_files = std::cell::RefCell::new(vec![None::<PathBuf>; extracted.entry_count()]);
        let mut url_cancelled = false;
        // Upstream `__process_playlist` `failures`.
        let mut extraction_failures = 0;
        let failed_before = failed;
        for (entry_index, mut info) in extracted.into_entries().into_iter().enumerate() {
            let mut counted = false;
            let postprocess_failed = std::cell::Cell::new(false);
            // Upstream `__pending_error` from a failed preprocessing command.
            let pending_error = std::cell::Cell::new(None::<String>);
            let defer = |error: String| {
                report_error!("ERROR: Preprocessing: {error}");
                let earlier = pending_error.take();
                pending_error.set(earlier.or(Some(format!("Preprocessing: {error}"))));
            };
            let mut entry_cancel = None;
            // Upstream `_num_videos` (`process_video_result`).
            videos += 1;
            template_options.video_autonumber.set(Some(videos));
            if let Some(index) = info.playlist_autonumber
                && playlist_title.is_some()
            {
                let count = info
                    .n_entries
                    .map_or_else(|| "N/A".into(), |count| count.to_string());
                screen!("[download] Downloading item {index} of {count}");
            }
            // `--flat-playlist`: a `url_transparent` entry is not extracted;
            // its forced printings see the raw entry.
            if args.flat_playlist
                && let Some((raw, full)) = YoutubeDl::flat_entry(&info, epoch())
            {
                if let Some(entry) = json_entries
                    .as_mut()
                    .and_then(|entries| entries.get_mut(entry_index))
                {
                    *entry = serde_json::Value::Object(raw);
                }
                let full = serde_json::Value::Object(full);
                for template in print_templates
                    .iter()
                    .filter(|template| template.stage() == Stage::Video)
                {
                    writeln!(
                        stdout,
                        "{}",
                        template.render_dict(&full, &template_options)?
                    )?;
                    // One `write_string` call per message.
                    stdout.flush()?;
                }
                if args.dump_json || args.print_json {
                    writeln!(stdout, "{}", yt_dlp_rs::output::python_json_dumps(&full))?;
                    // One `write_string` call per message.
                    stdout.flush()?;
                }
                continue;
            }
            let result = (|| -> Result<(), Box<dyn std::error::Error>> {
                // A `url_transparent` entry (RSS items) is extracted now.
                if YoutubeDl::is_transparent(&info) {
                    info = downloader.resolve_entry(info.clone())?;
                }
                // Upstream `process_subtitles` names the selected languages.
                if let Some(requested) = &info.requested_subtitles
                    && !requested.entries.is_empty()
                {
                    let languages: Vec<&str> = requested
                        .entries
                        .iter()
                        .map(|(language, _)| language.as_deref().unwrap_or("None"))
                        .collect();
                    screen!(
                        "[info] {}: Downloading subtitles: {}",
                        info.id,
                        languages.join(", ")
                    );
                }
                // Upstream `process_video_result`: `--check-all-formats` tests
                // every format first.
                downloader.check_all_formats(&mut info);
                // Upstream `process_video_result`: `pre_process` and
                // `after_filter` processors precede listing and selection.
                for when in ["pre_process", "after_filter"] {
                    run_metadata_parser(&metadata_actions, when, &mut info, &template_options)?;
                }
                // Upstream `SponsorBlockPP`, the last `after_filter` processor.
                // Its failure is a deferred preprocessing error, like upstream's
                // `__pending_error`.
                if !sponsorblock_query.is_empty() {
                    set_pp_info(&info);
                    postprocess_progress("SponsorBlock", "started");
                    match downloader.fetch_sponsorblock(
                        &mut info,
                        &sponsorblock_query,
                        &args.sponsorblock_api,
                        u32::from(args.extractor_retries),
                    ) {
                        Ok(()) => postprocess_progress("SponsorBlock", "finished"),
                        Err(error) => {
                            let text = match error {
                                yt_dlp_rs::Error::Postprocess(text) => text,
                                other => other.to_string(),
                            };
                            defer(text);
                        }
                    }
                }
                // Upstream lists thumbnails, then subtitles, then formats.
                if args.list_thumbnails {
                    emit_listing(&mut stdout, &yt_dlp_rs::output::render_thumbnails(&info))?;
                }
                if args.list_subs {
                    if !info.automatic_captions.is_empty() {
                        emit_listing(&mut stdout, &render_automatic_captions(&info))?;
                    }
                    emit_listing(&mut stdout, &render_subtitles(&info))?;
                }
                // `-f -`: interactive format selection lists the formats too.
                let interactive = !list_only && format == Some("-");
                if args.list_formats || interactive {
                    emit_listing(
                        &mut stdout,
                        &yt_dlp_rs::output::render_formats_listing(&info, !args.list_formats_old),
                    )?;
                }
                // Upstream `process_video_result` format selection; the info
                // dict is updated with the last selected format.
                let mut selections = Vec::new();
                if interactive {
                    // `input('\nEnter format selector ...')` until a selector
                    // selects something; an empty answer is the default.
                    loop {
                        write!(
                            stdout,
                            "\nEnter format selector (Press ENTER for default, or Ctrl+C to quit): "
                        )?;
                        stdout.flush()?;
                        let mut answer = String::new();
                        if io::stdin().read_line(&mut answer)? == 0 {
                            return Err(
                                EntryExtractionError("EOF when reading a line".into()).into()
                            );
                        }
                        let answer = answer.strip_suffix('\n').unwrap_or(&answer);
                        let requested = (!answer.is_empty()).then_some(answer);
                        match downloader.select_videos(&info, requested) {
                            Ok(selected) if !selected.is_empty() => {
                                selections = selected;
                                break;
                            }
                            // Reported without counting as a failure.
                            Err(yt_dlp_rs::Error::Reported(message)) => {
                                report_error!("ERROR: {message}");
                            }
                            Ok(_) | Err(yt_dlp_rs::Error::FormatSelection(_)) => {
                                report_error!("ERROR: Requested format is not available");
                            }
                            Err(error) => return Err(error.into()),
                        }
                    }
                    info = selections.last().cloned().expect("a nonempty selection");
                } else if !list_only {
                    selections = downloader.select_videos(&info, format).map_err(
                        |error| -> Box<dyn std::error::Error> {
                            match error {
                                yt_dlp_rs::Error::FormatSelection(_) => {
                                    Box::new(EntryExtractionError(format!(
                                        "[{}] {}: Requested format is not available. \
                                         Use --list-formats for a list of available formats",
                                        info.extractor, info.id
                                    )))
                                }
                                other => Box::new(other),
                            }
                        },
                    )?;
                    info = selections.last().cloned().expect("a nonempty selection");
                }
                if selections.is_empty() {
                    selections.push(info.clone());
                }
                if let Some(entry) = json_entries
                    .as_mut()
                    .and_then(|entries| entries.get_mut(entry_index))
                {
                    *entry = serde_json::to_value(&info)?;
                    if let (Some(index), Some(entry)) =
                        (info.last_playlist_index, entry.as_object_mut())
                    {
                        entry.insert("__last_playlist_index".into(), index.into());
                    }
                } else if let Some(value @ Some(_)) =
                    (playlist_title.is_none() && args.dump_single_json).then_some(&mut single_json)
                {
                    *value = Some(serde_json::to_value(&info)?);
                }
                // `simulate == 'list_only'` ends after listing; an explicit
                // `--simulate` still selects (and announces) formats.
                if list_only {
                    return Ok(());
                }
                // Upstream `process_video_result` after format selection.
                let format_ids: Vec<String> = selections
                    .iter()
                    .map(|info| {
                        if info.requested_formats.is_empty() {
                            info.format_id.clone().unwrap_or_else(|| info.ext.clone())
                        } else {
                            info.requested_formats
                                .iter()
                                .map(|format| format.format_id.as_str())
                                .collect::<Vec<_>>()
                                .join("+")
                        }
                    })
                    .collect();
                // Upstream `download_ranges`: every requested section is
                // processed as its own info dict (none when no chapter matches).
                let requested = download_sections.requested(&info);
                if requested.is_empty() {
                    return Ok(());
                }
                screen!(
                    "[info] {}: Downloading {} format(s): {}",
                    info.id,
                    format_ids.len(),
                    format_ids.join(", ")
                );
                if requested != [None] {
                    screen!(
                        "[info] {}: Downloading {} time ranges: {}",
                        info.id,
                        requested.len(),
                        yt_dlp_rs::sections::describe(&requested)
                    );
                }
                // `itertools.product(formats_to_download, requested_ranges)`.
                let runs: Vec<_> = selections
                    .iter()
                    .flat_map(|selected| requested.iter().map(move |section| (selected, section)))
                    .collect();
                for (selected, section) in runs {
                    let mut info = yt_dlp_rs::sections::apply(selected, section.as_ref());
                    (|| -> Result<(), Box<dyn std::error::Error>> {
                        // Upstream's YouTube `_get_comments` uses its comment API.
                        if args.write_comments && info.extractor_key == "Youtube" {
                            return Err(yt_dlp_rs::Error::Reported(
                                "YouTube comment extraction (--write-comments) is not supported natively"
                                    .into(),
                            )
                            .into());
                        }
                        // Upstream `process_info` rechecks complete metadata before counting.
                        if !match_filters.is_empty() {
                            match match_filters.check(&info, false)? {
                                MatchOutcome::Pass => {}
                                // `_match_entry`'s `Download "FILE"? (Y/n): `.
                                MatchOutcome::Ask => {
                                    let filename =
                                        prepare_media(&templates, &info, &template_options)
                                            .map(|path| path.display().to_string())
                                            .unwrap_or_default();
                                    loop {
                                        let mut stdout = io::stdout().lock();
                                        write!(stdout, "Download \"{filename}\"? (Y/n): ")?;
                                        stdout.flush()?;
                                        drop(stdout);
                                        let mut reply = String::new();
                                        if io::stdin().read_line(&mut reply)? == 0 {
                                            return Err(yt_dlp_rs::Error::Reported(
                                                "EOF when reading a line".into(),
                                            )
                                            .into());
                                        }
                                        match yt_dlp_rs::batch::py_strip(&reply.to_lowercase()) {
                                            "y" | "" => break,
                                            "n" => {
                                                let title = Some(info.title.as_str())
                                                    .filter(|title| !title.is_empty())
                                                    .unwrap_or(&info.id);
                                                screen!("[download] Skipping {title}");
                                                return Ok(());
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                                MatchOutcome::Reject(reason) => {
                                    screen!("[download] {reason}");
                                    return Ok(());
                                }
                                MatchOutcome::Break(reason) => {
                                    screen!("[download] {reason}");
                                    entry_cancel = Some(Cancel::Rejected);
                                    return Ok(());
                                }
                            }
                        }
                        // Upstream `pre_process(info, 'video')` before counting.
                        run_metadata_parser(
                            &metadata_actions,
                            "video",
                            &mut info,
                            &template_options,
                        )?;
                        // Upstream `process_info` counts every processed video.
                        counted = true;
                        template_options.autonumber.set(Some(
                            (args.autonumber_start - 1 + processed + 1).max(0) as u64,
                        ));
                        // The info postprocessor progress templates describe.
                        set_pp_info(&info);
                        if let Err(error) =
                            run_exec(&exec, Stage::Video, &info, None, None, &template_options)
                        {
                            defer(error);
                        }
                        // `_forceprint`: `filename` is the prepared media filename.
                        let print_filename = prepare_media(&templates, &info, &template_options)
                            .map(|path| path.to_string_lossy().into_owned());
                        let stage_print =
                            |stage: Stage, info: &yt_dlp_rs::VideoInfo, paths: StagePaths<'_>| {
                                render_stage(
                                    stage,
                                    &print_templates,
                                    &print_files,
                                    info,
                                    print_filename.as_deref(),
                                    paths,
                                    &template_options,
                                )
                            };
                        stage_print(
                            Stage::Video,
                            &info,
                            StagePaths {
                                filepath: None,
                                prepared: print_filename.as_deref().ok(),
                            },
                        )?
                        .emit()?;
                        // Upstream `-J`: each processed download's dict joins
                        // the result's `requested_downloads`.
                        if args.dump_single_json {
                            let mut download = serde_json::to_value(&info)?;
                            if let Some(object) = download.as_object_mut() {
                                if let Some(index) = info.last_playlist_index {
                                    object.insert("__last_playlist_index".into(), index.into());
                                }
                                if let Ok(filename) = &print_filename {
                                    object.insert("_filename".into(), filename.clone().into());
                                    object.insert("filename".into(), filename.clone().into());
                                }
                                object.insert(
                                    "__write_download_archive".into(),
                                    (!simulate || args.force_write_archive).into(),
                                );
                            }
                            let target = match json_entries
                                .as_mut()
                                .and_then(|entries| entries.get_mut(entry_index))
                            {
                                Some(entry) => Some(entry),
                                None if playlist_title.is_none() => single_json.as_mut(),
                                None => None,
                            };
                            if let Some(serde_json::Value::Object(target)) = target
                                && let serde_json::Value::Array(downloads) = target
                                    .entry("requested_downloads")
                                    .or_insert_with(|| serde_json::Value::Array(Vec::new()))
                            {
                                downloads.push(download);
                            }
                        }
                        // Upstream `forcejson`: `--dump-json` or `--print-json`.
                        if args.dump_json || args.print_json {
                            // `sanitize_info` of the dict `process_info` has given
                            // `_filename`/`filename`; `epoch` and `_type` default.
                            let mut value = serde_json::to_value(&info)?;
                            if let Some(object) = value.as_object_mut() {
                                if let Some(index) = info.last_playlist_index {
                                    object.insert("__last_playlist_index".into(), index.into());
                                }
                                if let Ok(filename) = &print_filename {
                                    object.insert("_filename".into(), filename.clone().into());
                                    object.insert("filename".into(), filename.clone().into());
                                }
                                object.entry("epoch").or_insert(epoch().into());
                                object.entry("_type").or_insert("video".into());
                            }
                            yt_dlp_rs::upstream_shape(&mut value);
                            writeln!(stdout, "{}", yt_dlp_rs::output::python_json_dumps(&value))?;
                            // One `write_string` call per message.
                            stdout.flush()?;
                        }
                        // --force-write-archive records simulated or skipped videos.
                        let mut force_record = |info: &yt_dlp_rs::VideoInfo| -> Result<(), String> {
                            match &mut archive {
                                Some(archive) if args.force_write_archive => archive
                                    .record(info)
                                    .map_err(|error| format!("archive update failed: {error}")),
                                _ => Ok(()),
                            }
                        };
                        if simulate {
                            force_record(&info)?;
                        } else {
                            if args.skip_download
                                && !writes_info_json
                                && !writes_description
                                && !writes_thumbnails
                                && !exec.has(Stage::BeforeDownload)
                                && !writes_links
                                && !prints_stage(Stage::BeforeDownload)
                                && !(write_subs || args.write_auto_subs)
                            {
                                // `MoveFilesAfterDownloadPP(downloaded=False)` has
                                // nothing to move.
                                postprocess_progress("MoveFiles", "started");
                                postprocess_progress("MoveFiles", "finished");
                                force_record(&info)?;
                                return Ok(());
                            }
                            let path = prepare_media(&templates, &info, &template_options)?;
                            if !template_options.paths.is_empty()
                                && path_is_absolute_template(&templates)
                            {
                                warning!(
                                    "WARNING: --paths is ignored since an absolute path is given in output template"
                                );
                            }
                            // Upstream `prepare_filename(info, 'temp')`: downloads and
                            // processors work on the temporary name, then
                            // `MoveFiles` publishes beside `final_path`.
                            let final_path = path;
                            let path = if final_path.as_os_str() != "-"
                                && template_options.paths.contains_key("temp")
                            {
                                templates
                                    .prepare(&info, Some("temp"), &template_options)?
                                    .expect("default output template is non-empty")
                            } else {
                                final_path.clone()
                            };
                            // `_ensure_dir_exists` for both names.
                            if final_path.as_os_str() != "-" {
                                for name in [&final_path, &path] {
                                    if let Some(parent) =
                                        name.parent().filter(|p| !p.as_os_str().is_empty())
                                    {
                                        std::fs::create_dir_all(parent)?;
                                    }
                                }
                            }
                            let mut move_files = MoveFiles::new(&final_path, overwrites);
                            let processing = processors.audio.is_some()
                                || processors.video.is_some()
                                || args.embed_subs
                                || processors.metadata
                                || processors.chapters
                                || embed_info_json.is_some()
                                || args.split_chapters
                                || processors.modify_chapters.is_some()
                                || args.embed_thumbnail;
                            // Upstream `existing_video_file`: the `final_ext` name
                            // first, then the prepared one.
                            let mut existing_files = Vec::new();
                            if !args.skip_download {
                                let mut candidates = Vec::new();
                                for name in [&final_path, &path] {
                                    candidates.extend(
                                        final_ext
                                            .as_deref()
                                            .map(|ext| {
                                                yt_dlp_rs::audio_output_path(name, &info.ext, ext)
                                            })
                                            .transpose()?,
                                    );
                                    candidates.push(name.clone());
                                }
                                for candidate in candidates {
                                    if !existing_files.contains(&candidate)
                                        && path_exists(&candidate)?
                                    {
                                        existing_files.push(candidate);
                                    }
                                }
                            }
                            let existing_media = !existing_files.is_empty();
                            // Upstream writes the description before subtitles and info JSON.
                            if writes_description
                                && let Some(path) = templates.prepare(
                                    &info,
                                    Some("description"),
                                    &template_options,
                                )?
                            {
                                write_description(&path, &info, overwrites)?;
                            }
                            // Plan source, intermediate and final names before subtitle I/O.
                            let has_subtitles = info
                                .requested_subtitles
                                .as_ref()
                                .is_some_and(|subtitles| !subtitles.is_empty());
                            if (write_subs || args.write_auto_subs) && !has_subtitles {
                                screen!(
                                    "[info] There are no subtitles for the requested languages"
                                );
                            }
                            // Upstream writes subtitles beside the (temporary) media
                            // name; `MoveFiles` gives them the subtitle template's.
                            let mut subtitle_finals = Vec::new();
                            let sidecars = match templates.prepare(
                                &info,
                                Some("subtitle"),
                                &template_options,
                            )? {
                                Some(base) => {
                                    subtitle_finals = subtitle_paths(&info, &base)?
                                        .into_iter()
                                        .map(|(_, _, path)| path)
                                        .collect();
                                    subtitle_paths(&info, &path)?
                                }
                                None => {
                                    if has_subtitles {
                                        screen!("[info] Skipping writing video subtitles");
                                    }
                                    Vec::new()
                                }
                            };
                            let mut seen = std::collections::HashSet::new();
                            let info_json = if writes_info_json {
                                templates.prepare(&info, Some("infojson"), &template_options)?
                            } else {
                                None
                            };
                            if let Some(info_json) = &info_json {
                                seen.insert(info_json.as_os_str().to_ascii_lowercase());
                                if paths_alias(info_json, &path)? {
                                    return Err(yt_dlp_rs::Error::InfoJson(
                                        "info JSON path collides with media output".into(),
                                    )
                                    .into());
                                }
                                reject_reserved(info_json, [&archive_path, &args.cookies])?;
                            }
                            for (_, subtitle, source) in &sidecars {
                                let mut paths = vec![source.clone()];
                                if let Some(conversion) = &subtitle_conversion {
                                    let target = conversion.output_path(source, &subtitle.ext)?;
                                    if target != *source {
                                        paths.push(target);
                                    }
                                    if let Some(intermediate) =
                                        conversion.intermediate_path(source, &subtitle.ext)
                                    {
                                        paths.push(intermediate);
                                    }
                                }
                                for sidecar in &paths {
                                    if !seen.insert(sidecar.as_os_str().to_ascii_lowercase()) {
                                        return Err(yt_dlp_rs::Error::Subtitle(
                                            "subtitle conversion or info JSON filenames collide"
                                                .into(),
                                        )
                                        .into());
                                    }
                                    for reserved in
                                        [&archive_path, &args.cookies].into_iter().flatten()
                                    {
                                        if paths_alias(sidecar, reserved)? {
                                            return Err(yt_dlp_rs::Error::Subtitle("subtitle path collides with cookie file or download archive".into()).into());
                                        }
                                    }
                                }
                            }
                            let mut embedding_files = Vec::new();
                            let planned_sidecars: Vec<_> = sidecars
                                .iter()
                                .map(|(_, subtitle, source)| (subtitle.ext.clone(), source.clone()))
                                .collect();
                            for ((language, subtitle, written_path), final_subtitle) in
                                sidecars.into_iter().zip(subtitle_finals)
                            {
                                // Upstream `existing_file((final, written))`: related
                                // files are overwritten unless --no-overwrites.
                                let mut subtitle_path = written_path.clone();
                                for candidate in [&final_subtitle, &written_path] {
                                    if std::fs::symlink_metadata(candidate).is_ok()
                                        && std::fs::metadata(candidate)?.is_file()
                                    {
                                        if overwrites != Some(false) {
                                            screen!(
                                                "Deleting existing file {}",
                                                candidate.display()
                                            );
                                            std::fs::remove_file(candidate)?;
                                        } else if subtitle_path == written_path {
                                            subtitle_path = candidate.clone();
                                        }
                                    }
                                    if final_subtitle == written_path {
                                        break;
                                    }
                                }
                                move_files
                                    .insert(subtitle_path.clone(), Some(final_subtitle.clone()));
                                match std::fs::symlink_metadata(&subtitle_path) {
                                    Ok(_) if std::fs::metadata(&subtitle_path)?.is_file() => {
                                        screen!(
                                            "[info] Video subtitle {}.{} is already present",
                                            language,
                                            subtitle.ext
                                        );
                                    }
                                    Ok(_) => {
                                        return Err(yt_dlp_rs::Error::Subtitle(format!(
                                            "subtitle destination is not a regular file: {}",
                                            subtitle_path.display()
                                        ))
                                        .into());
                                    }
                                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                                        screen!(
                                            "[info] Writing video subtitles to: {}",
                                            subtitle_path.display()
                                        );
                                        sleep.subtitle();
                                        // Upstream `dl(..., subtitle=True)`: the file
                                        // downloader announces the sidecar once
                                        // connected and reports it.
                                        let announce = std::cell::Cell::new(true);
                                        let destination = || {
                                            if announce.replace(false) {
                                                screen!(
                                                    "[download] Destination: {}",
                                                    subtitle_path.display()
                                                );
                                            }
                                        };
                                        let mut reporter = ProgressReporter::new(&progress_options);
                                        let downloaded = downloader
                                            .download_subtitle_with_progress(
                                                &info,
                                                &subtitle,
                                                &subtitle_path,
                                                DownloadOptions {
                                                    retries,
                                                    update_time: args.mtime,
                                                    ..Default::default()
                                                },
                                                |progress| {
                                                    if !matches!(
                                                        progress.phase,
                                                        DownloadPhase::Requesting
                                                            | DownloadPhase::WaitingToRetry
                                                    ) {
                                                        destination();
                                                    }
                                                    reporter.event(&progress);
                                                    ControlFlow::Continue(())
                                                },
                                            );
                                        if let Err(error) = downloaded {
                                            // Upstream `_write_subtitles`: a warning
                                            // with --ignore-errors, else the video fails.
                                            let message = format!(
                                                "Unable to download video subtitles for {}: {}",
                                                yt_dlp_rs::batch::py_repr(&language),
                                                yt_dlp_rs::upstream_error_text(&error)
                                            );
                                            if error_mode == ErrorMode::Ignore {
                                                warning!("WARNING: {message}");
                                                continue;
                                            }
                                            // Reported first only with --abort-on-error.
                                            return Err(yt_dlp_rs::Error::Raised(message).into());
                                        }
                                        destination();
                                        reporter.finish();
                                    }
                                    Err(error) => return Err(error.into()),
                                }
                                let embedded_file = SubtitleFile {
                                    path: subtitle_path.clone(),
                                    ext: subtitle.ext.clone(),
                                    language,
                                    name: subtitle.name.clone(),
                                };
                                embedding_files.push(embedded_file);
                            }
                            // Upstream writes thumbnails after subtitles and before info JSON.
                            let mut thumbnail_files = Vec::new();
                            if writes_thumbnails {
                                thumbnail_files = write_thumbnails(
                                    &downloader,
                                    &info,
                                    &path,
                                    &mut move_files,
                                    templates.prepare(
                                        &info,
                                        Some("thumbnail"),
                                        &template_options,
                                    )?,
                                    thumbnail_mode == 2,
                                    overwrites,
                                    retries,
                                )?;
                            }
                            // Upstream writes info JSON after subtitles and before media.
                            if let Some(info_json) = &info_json {
                                let value = yt_dlp_rs::video_info_json_with(
                                    &info,
                                    epoch(),
                                    !args.no_clean_info_json,
                                )?;
                                let written = write_info_json_with(
                                    info_json,
                                    &value,
                                    overwrites != Some(false),
                                )?;
                                report_info_json("video", info_json, written);
                            }
                            // Upstream writes internet shortcuts after info JSON.
                            if writes_links {
                                write_links(
                                    &link_kinds,
                                    &info,
                                    &templates,
                                    &template_options,
                                    overwrites,
                                )?;
                            }
                            let prepared = final_path.to_string_lossy().into_owned();
                            let downloaded = path.to_string_lossy().into_owned();
                            stage_print(
                                Stage::BeforeDownload,
                                &info,
                                stage_paths(None, &prepared),
                            )?
                            .emit()?;
                            // Upstream's `before_dl` processors: MetadataParser,
                            // SubtitlesConvertor, ThumbnailsConvertor, then
                            // `--exec before_dl:`; a failure skips the rest.
                            run_metadata_parser(
                                &metadata_actions,
                                "before_dl",
                                &mut info,
                                &template_options,
                            )?;
                            let before_dl = (|| -> Result<(), String> {
                                if let Some(conversion) = &subtitle_conversion {
                                    postprocess_progress("SubtitlesConvertor", "started");
                                    convert_subtitles(
                                        &downloader,
                                        &info,
                                        &mut embedding_files,
                                        conversion,
                                        args.keep_video,
                                        &mut move_files,
                                    )?;
                                }
                                if let Some(conversion) = &thumbnail_conversion {
                                    postprocess_progress("ThumbnailsConvertor", "started");
                                    convert_thumbnails(
                                        &downloader,
                                        &mut thumbnail_files,
                                        &mut move_files,
                                        conversion,
                                        args.keep_video,
                                    )
                                    .map_err(|error| error.to_string())?;
                                }
                                run_exec(
                                    &exec,
                                    Stage::BeforeDownload,
                                    &info,
                                    None,
                                    Some(&final_path),
                                    &template_options,
                                )
                            })();
                            if let Err(error) = before_dl {
                                defer(error);
                            }
                            // Upstream prints `post_process` before merging/processing
                            // and `after_move` with the absolute moved path.
                            let post_print = stage_print(
                                Stage::PostProcess,
                                &info,
                                stage_paths(Some(&downloaded), &prepared),
                            )?;
                            let move_print = |filepath: &std::path::Path,
                                              info: &yt_dlp_rs::VideoInfo|
                             -> Result<(), String> {
                                let filepath = py_abspath(filepath).to_string_lossy().into_owned();
                                stage_print(
                                    Stage::AfterMove,
                                    info,
                                    stage_paths(Some(&filepath), &prepared),
                                )
                                .map_err(|error| error.to_string())?
                                .emit()
                                .map_err(|error| error.to_string())
                            };

                            if args.skip_download {
                                // `MoveFilesAfterDownloadPP(downloaded=False)` publishes
                                // the sidecars.
                                if let Err(error) = move_files.run(&path, false) {
                                    report_error!("ERROR: Postprocessing: {error}");
                                    return Ok(());
                                }
                                force_record(&info)?;
                                return Ok(());
                            }
                            // Upstream `FileDownloader.download` reuses a single file
                            // found under its own name only with --no-overwrites
                            // or when continuing with part files; otherwise it
                            // downloads again over it.
                            let redownload = existing_media
                                && overwrites.is_none()
                                && existing_files[0] == path
                                && info.requested_formats.is_empty()
                                && (args.no_continue || args.no_part);
                            let existing_media = existing_media && !redownload;
                            let mut process_existing = None;
                            let found = existing_files.first().cloned();
                            if existing_media && overwrites == Some(true) {
                                for existing in &existing_files {
                                    screen!("Deleting existing file {}", existing.display());
                                    std::fs::remove_file(existing)?;
                                }
                            } else if existing_media {
                                let existing = existing_files.swap_remove(0);
                                screen!(
                                    "[download] {} has already been downloaded",
                                    existing.display()
                                );
                                // A single file found under its own name goes
                                // through `FileDownloader.download`, which also
                                // reports it finished.
                                if existing == path && info.requested_formats.is_empty() {
                                    ProgressReporter::new(&progress_options)
                                        .for_download(&info, &path)
                                        .already_downloaded(std::fs::metadata(&path)?.len());
                                }
                                if processing {
                                    // The processors run again on the existing file.
                                    process_existing = Some(existing);
                                }
                            }
                            if existing_media && overwrites != Some(true) && !processing {
                                {
                                    post_print.emit()?;
                                    let found = found.as_ref().unwrap_or(&path);
                                    entry_files.borrow_mut()[entry_index] =
                                        Some(move_files.final_path(found));
                                    if args.dump_single_json {
                                        let target = match json_entries
                                            .as_mut()
                                            .and_then(|entries| entries.get_mut(entry_index))
                                        {
                                            Some(entry) => Some(entry),
                                            None if playlist_title.is_none() => {
                                                single_json.as_mut()
                                            }
                                            None => None,
                                        };
                                        annotate_download(
                                            target,
                                            &info,
                                            &path,
                                            false,
                                            &move_files.final_path(found),
                                        );
                                    }
                                    // The `post_process` `MetadataParserPP`.
                                    let mut parsed_info = None;
                                    if metadata_actions.has("post_process") {
                                        let mut parsed = info.clone();
                                        run_metadata_parser(
                                            &metadata_actions,
                                            "post_process",
                                            &mut parsed,
                                            &template_options,
                                        )?;
                                        parsed_info = Some(parsed);
                                    }
                                    if !after_download(
                                        &exec,
                                        parsed_info.as_ref().unwrap_or(&info),
                                        found,
                                        &final_path,
                                        &move_files,
                                        &template_options,
                                        error_mode,
                                        &pending_error,
                                        &postprocess_failed,
                                        &move_print,
                                        &metadata_actions,
                                    ) {
                                        return Ok(());
                                    }
                                    if let Some(archive) = &mut archive {
                                        archive.record(&info).map_err(|error| {
                                            format!("archive update failed: {error}")
                                        })?;
                                    }
                                    return Ok(());
                                }
                            }
                            // Upstream `_catch_unsafe_extension_error` around the
                            // download's `replace_extension` calls.
                            if let Some(ext) = std::iter::once(&info.ext)
                                .chain(info.requested_formats.iter().map(|format| &format.ext))
                                .find(|ext| {
                                    // allow-unsafe-ext disables the check, but path
                                    // separators stay rejected natively.
                                    if ALLOW_UNSAFE_EXT.load(Ordering::Relaxed) {
                                        ext.contains(['/', '\\'])
                                    } else {
                                        !yt_dlp_rs::output::allowed_extension(ext)
                                    }
                                })
                            {
                                return Err(format!(
                            "The extracted extension ({}) is unusual and will be skipped for safety reasons. If you believe this is an error, please report it",
                            yt_dlp_rs::batch::py_repr(ext)
                        )
                        .into());
                            }
                            let deferred_destination = std::cell::Cell::new(false);
                            // An existing file is processed without downloading.
                            let refreshed = if process_existing.is_none() {
                                let partial = yt_dlp_rs::sections::is_partial(&info);
                                // `get_suitable_downloader(...) == FFmpegFD`.
                                let ffmpeg_fd = downloader.uses_ffmpeg_fd(&info);
                                // Upstream `process_info`: a partial download needs FFmpegFD.
                                let no_direct_merge =
                                    compat.iter().any(|name| name == "no-direct-merge");
                                if no_direct_merge
                                    && ffmpeg_fd
                                    && !info.requested_formats.is_empty()
                                {
                                    // Upstream downloads each format with FFmpegFD
                                    // and merges them afterwards.
                                    return Err(yt_dlp_rs::Error::Reported(
                                        "--compat-options no-direct-merge with formats FFmpegFD would merge is not supported natively"
                                            .into(),
                                    )
                                    .into());
                                }
                                if partial && !downloader.has_ffmpeg() && !no_direct_merge {
                                    return Err(yt_dlp_rs::Error::Reported(
                                "You have requested downloading the video partially, but ffmpeg is not installed. Aborting"
                                    .into(),
                            )
                            .into());
                                }
                                if !partial
                                    && !info.requested_formats.is_empty()
                                    && !downloader.has_ffmpeg()
                                {
                                    let message = "You have requested merging of multiple formats but ffmpeg is not installed";
                                    if error_mode == ErrorMode::Abort {
                                        return Err(yt_dlp_rs::Error::Reported(format!(
                                            "{message}. Aborting due to --abort-on-error"
                                        ))
                                        .into());
                                    }
                                    warning!("WARNING: {message}. The formats won't be merged");
                                }
                                downloader.announce_download(&info);
                                // Upstream `HlsFD` refetches the media playlist first.
                                let refreshed = if !ffmpeg_fd && info.requested_formats.is_empty() {
                                    downloader.refresh_hls(&info)?
                                } else {
                                    None
                                };
                                let info = refreshed.as_ref().unwrap_or(&info);
                                if !ffmpeg_fd
                                    && info.requested_formats.is_empty()
                                    && !info.fragments.is_empty()
                                {
                                    // Upstream `_prepare_frag_download`.
                                    let downloader = match info.protocol.as_deref() {
                                        Some("http_dash_segments") => "dashsegments",
                                        Some("ism") => "ism",
                                        _ => "hlsnative",
                                    };
                                    screen!(
                                        "[{downloader}] Total fragments: {}",
                                        yt_dlp_rs::total_fragments(info)
                                    );
                                }
                                // Merges announce each format's file instead, except
                                // FFmpegFD merging the formats into one file.
                                if info.requested_formats.is_empty() || ffmpeg_fd {
                                    // `HttpFD` names the destination once data
                                    // arrives, after any resume messages.
                                    let resumed = match part_files {
                                        yt_dlp_rs::PartFiles::Part { resume: true } => {
                                            let mut name = path.as_os_str().to_owned();
                                            name.push(".part");
                                            Some(PathBuf::from(name))
                                        }
                                        yt_dlp_rs::PartFiles::NoPart { resume: true } => {
                                            Some(path.clone())
                                        }
                                        _ => None,
                                    };
                                    let resumes = !processing
                                        && !ffmpeg_fd
                                        && info.fragments.is_empty()
                                        && resumed.is_some_and(|file| {
                                            std::fs::metadata(file).is_ok_and(|metadata| {
                                                metadata.is_file() && metadata.len() > 0
                                            })
                                        });
                                    // HttpFD also waits for the connection.
                                    let http_fd = !ffmpeg_fd
                                        && info.fragments.is_empty()
                                        && info.requested_formats.is_empty();
                                    if resumes || http_fd {
                                        deferred_destination.set(true);
                                    } else {
                                        screen!("[download] Destination: {}", path.display());
                                    }
                                }
                                // Upstream sleeps before each stream's FileDownloader.download.
                                for _ in 0..info.requested_formats.len().max(1) {
                                    sleep.media();
                                }
                                refreshed
                            } else {
                                None
                            };
                            let info = refreshed.as_ref().unwrap_or(&info);
                            let post_printed = std::cell::Cell::new(false);
                            let mut reporter =
                                ProgressReporter::new(&progress_options).for_download(info, &path);
                            let mut progress = |progress: yt_dlp_rs::DownloadProgress| {
                                if !matches!(
                                    progress.phase,
                                    DownloadPhase::Requesting | DownloadPhase::WaitingToRetry
                                ) && deferred_destination.replace(false)
                                {
                                    screen!("[download] Destination: {}", path.display());
                                }
                                reporter.event(&progress);
                                if matches!(
                                    progress.phase,
                                    DownloadPhase::Merging | DownloadPhase::Postprocessing
                                ) && !post_printed.replace(true)
                                    && post_print.emit().is_err()
                                {
                                    return ControlFlow::Break(());
                                }
                                ControlFlow::Continue(())
                            };
                            let mut processors = processors.clone();
                            if let Some(explicit) = embed_info_json {
                                processors.infojson = Some(yt_dlp_rs::InfoJsonEmbed {
                                    written: info_json.clone(),
                                    explicit,
                                    json: yt_dlp_rs::to_python_json(
                                        &yt_dlp_rs::video_info_json_with(
                                            info,
                                            epoch(),
                                            !args.no_clean_info_json,
                                        )?,
                                    )?,
                                });
                            }
                            // Upstream embeds the best thumbnail on disk.
                            let mut embedded_thumbnail = None;
                            // Printed when the processor runs, after the download.
                            let mut embed_note = None;
                            if args.embed_thumbnail {
                                if info.metadata.thumbnails.is_empty() {
                                    embed_note = Some(
                                        "[EmbedThumbnail] There aren't any thumbnails to embed",
                                    );
                                } else if let Some((_, written)) = thumbnail_files.last() {
                                    processors.thumbnail = Some(written.clone());
                                    embedded_thumbnail = Some(written.clone());
                                } else {
                                    embed_note =
                                        Some("[EmbedThumbnail] There are no thumbnails on disk");
                                }
                            }
                            if let Some(modify) = &mut processors.modify_chapters {
                                modify.subtitles = embedding_files
                                    .iter()
                                    .map(|file| (file.path.clone(), file.ext.clone()))
                                    .collect();
                            }
                            if args.embed_subs {
                                processors.embed = Some(SubtitleEmbedding {
                                    subtitles: embedding_files,
                                    keep_subtitles: (args.write_subs && keep_subs)
                                        || args.keep_video,
                                });
                            }
                            let processing = processors.audio.is_some()
                                || processors.video.is_some()
                                || processors.embed.is_some()
                                || processors.metadata
                                || processors.chapters
                                || processors.infojson.is_some()
                                || processors.modify_chapters.is_some()
                                || processors.thumbnail.is_some();
                            if path.as_os_str() == "-" {
                                // Upstream streams to stdout and skips postprocessing,
                                // fixups, the archive record and after-move hooks.
                                let streams: Vec<yt_dlp_rs::VideoInfo> = if info
                                    .requested_formats
                                    .is_empty()
                                {
                                    vec![info.clone()]
                                } else {
                                    let reason = if downloader.has_ffmpeg() {
                                        return Err(
                                    "merging multiple formats to stdout through ffmpeg is unsupported"
                                        .into(),
                                );
                                    } else {
                                        "but ffmpeg is not installed"
                                    };
                                    warning!(
                                        "WARNING: You have requested downloading multiple formats to stdout {reason}. The formats will be streamed one after the other"
                                    );
                                    info.requested_formats
                                        .iter()
                                        .map(|format| info.with_selected_format(format))
                                        .collect()
                                };
                                for stream in &streams {
                                    let refreshed = if info.requested_formats.is_empty() {
                                        None
                                    } else {
                                        downloader.announce_download(stream);
                                        downloader.refresh_hls(stream)?
                                    };
                                    let stream = refreshed.as_ref().unwrap_or(stream);
                                    // A single result already announced itself above.
                                    if !info.requested_formats.is_empty() {
                                        if !stream.fragments.is_empty() {
                                            let downloader = match stream.protocol.as_deref() {
                                                Some("http_dash_segments") => "dashsegments",
                                                Some("ism") => "ism",
                                                _ => "hlsnative",
                                            };
                                            screen!(
                                                "[{downloader}] Total fragments: {}",
                                                yt_dlp_rs::total_fragments(stream)
                                            );
                                        }
                                        screen!("[download] Destination: -");
                                    }
                                    let temporary = tempfile::tempdir()?;
                                    let target = temporary.path().join("media");
                                    downloader.download_with_progress(
                                        stream,
                                        &target,
                                        DownloadOptions {
                                            min_filesize,
                                            max_filesize,
                                            rate_limit,
                                            update_time: false,
                                            part_files: yt_dlp_rs::PartFiles::Private,
                                            retries,
                                            fragment_retries,
                                            retry_sleep,
                                            skip_unavailable_fragments,
                                            merge_files: yt_dlp_rs::MergeFiles::Private,
                                            overwrite: false,
                                        },
                                        &mut progress,
                                    )?;
                                    let mut media = std::fs::File::open(&target)?;
                                    let mut out = io::stdout().lock();
                                    io::copy(&mut media, &mut out)?;
                                    out.flush()?;
                                }
                                return Ok(());
                            }
                            // `run_pp` with -k keeps the merged formats for `MoveFiles`.
                            if args.keep_video {
                                for format in &info.requested_formats {
                                    move_files
                                        .keep(yt_dlp_rs::stream_path(&path, &info.ext, format));
                                }
                            }
                            let mut changed_chapters = None;
                            // The info dict as `post_process` processors changed it.
                            let mut processed_info = None;
                            let (bytes, published_paths) = if processing {
                                // `existing_video_file` sets `ext` from the found file.
                                let existing_info;
                                let (info, path) = match &process_existing {
                                    Some(existing) => {
                                        let mut found = info.clone();
                                        if let Some(ext) = existing.extension() {
                                            found.ext = ext.to_string_lossy().into_owned();
                                        }
                                        processors.existing = Some(existing.clone());
                                        existing_info = found;
                                        (&existing_info, existing)
                                    }
                                    None => (info, &path),
                                };
                                processors.metadata_parser = metadata_actions
                                    .has("post_process")
                                    .then(|| (metadata_actions.clone(), template_options.clone()));
                                let result = downloader.download_processed_with_progress(
                                    info,
                                    path,
                                    DownloadOptions {
                                        min_filesize,
                                        max_filesize,
                                        rate_limit,
                                        update_time: args.mtime,
                                        part_files,
                                        retries,
                                        fragment_retries,
                                        retry_sleep,
                                        skip_unavailable_fragments,
                                        merge_files: yt_dlp_rs::MergeFiles::Upstream {
                                            keep: args.keep_video,
                                        },
                                        overwrite: redownload,
                                    },
                                    &processors,
                                    progress,
                                )?;
                                for warning in result.warnings {
                                    warning!("WARNING: {warning}");
                                }
                                if let Some(note) = embed_note {
                                    postprocess_progress("EmbedThumbnail", "started");
                                    screen!("{note}");
                                    postprocess_progress("EmbedThumbnail", "finished");
                                }
                                // Upstream `run_pp` under --ignore-errors.
                                for error in result.errors {
                                    report_error!("ERROR: {error}");
                                    postprocess_failed.set(true);
                                }
                                // The processor may have corrected a WebP extension.
                                if let (Some(written), Some(fixed)) =
                                    (&embedded_thumbnail, &result.thumbnail)
                                    && written != fixed
                                {
                                    move_files.derive(written, fixed, true);
                                    embedded_thumbnail = Some(fixed.clone());
                                }
                                if let Some(thumbnail) = &embedded_thumbnail
                                    && !keeps_thumbnails
                                    && std::fs::remove_file(thumbnail).is_err()
                                {
                                    warning!(
                                        "WARNING: Unable to delete file {}",
                                        thumbnail.display()
                                    );
                                }
                                // Kept sources move to the final directory.
                                for kept in &result.kept_files {
                                    move_files.keep(kept.clone());
                                }
                                let mut paths = vec![result.path];
                                paths.extend(result.kept_files);
                                changed_chapters = result.chapters;
                                processed_info = result.info;
                                (result.bytes, paths)
                            } else {
                                let bytes = downloader.download_with_progress(
                                    info,
                                    &path,
                                    DownloadOptions {
                                        min_filesize,
                                        max_filesize,
                                        rate_limit,
                                        update_time: args.mtime,
                                        part_files,
                                        retries,
                                        fragment_retries,
                                        retry_sleep,
                                        skip_unavailable_fragments,
                                        merge_files: yt_dlp_rs::MergeFiles::Upstream {
                                            keep: args.keep_video,
                                        },
                                        overwrite: redownload,
                                    },
                                    progress,
                                )?;
                                // Upstream `fixup()` for an unmerged, unprocessed download.
                                if let Some(fixup) = &processors.fixup {
                                    downloader.fixup(info, &path, &path, fixup)?;
                                }
                                // The `post_process` `MetadataParserPP` follows the fixups.
                                if metadata_actions.has("post_process") {
                                    let mut parsed = info.clone();
                                    run_metadata_parser(
                                        &metadata_actions,
                                        "post_process",
                                        &mut parsed,
                                        &template_options,
                                    )?;
                                    processed_info = Some(parsed);
                                }
                                if let Some(note) = embed_note {
                                    postprocess_progress("EmbedThumbnail", "started");
                                    screen!("{note}");
                                    postprocess_progress("EmbedThumbnail", "finished");
                                }
                                (bytes, vec![path.clone()])
                            };
                            // `ModifyChaptersPP` and `post_process` metadata parsing
                            // change the info dict for later stages.
                            let modified_info;
                            let info = match processed_info {
                                Some(processed) => {
                                    modified_info = processed;
                                    &modified_info
                                }
                                None => match changed_chapters {
                                    Some((chapters, duration)) => {
                                        let mut modified = info.clone();
                                        modified.metadata.chapters = chapters;
                                        modified.metadata.duration = duration;
                                        modified_info = modified;
                                        &modified_info
                                    }
                                    None => info,
                                },
                            };
                            let mut published_paths = published_paths;
                            published_paths.extend(info_json);
                            for (ext, source) in planned_sidecars {
                                if let Some(conversion) = &subtitle_conversion {
                                    published_paths.push(conversion.output_path(&source, &ext)?);
                                    published_paths
                                        .extend(conversion.intermediate_path(&source, &ext));
                                }
                                published_paths.push(source);
                            }
                            if let Some(archive_path) = &archive_path {
                                for output in &published_paths {
                                    let identical = std::path::absolute(output)?
                                        == std::path::absolute(archive_path)?;
                                    let canonical_alias = std::fs::canonicalize(output)
                                        .ok()
                                        .zip(std::fs::canonicalize(archive_path).ok())
                                        .is_some_and(|(left, right)| left == right);
                                    if identical || canonical_alias {
                                        return Err(yt_dlp_rs::Error::Archive(
                                            "download output collides with archive path".into(),
                                        )
                                        .into());
                                    }
                                }
                            }
                            if !post_printed.get() {
                                post_print.emit()?;
                            }
                            // Upstream `FFmpegSplitChaptersPP`, the last
                            // `post_process` processor before `--exec`.
                            if args.split_chapters {
                                postprocess_progress("SplitChapters", "started");
                                let split = (|| -> yt_dlp_rs::Result<()> {
                                    if info.metadata.chapters.is_empty() {
                                        screen!(
                                            "[SplitChapters] Chapter information is unavailable"
                                        );
                                        return Ok(());
                                    }
                                    let mut chapters = Vec::new();
                                    for (index, chapter) in
                                        info.metadata.chapters.iter().enumerate()
                                    {
                                        let mut section = info.clone();
                                        section.section_number = Some(index + 1);
                                        section.section_title = chapter.title.clone();
                                        section.section_start = chapter.start_time;
                                        section.section_end = chapter.end_time;
                                        let Some(destination) = templates.prepare(
                                            &section,
                                            Some("chapter"),
                                            &template_options,
                                        )?
                                        else {
                                            continue;
                                        };
                                        chapters.push((chapter.clone(), destination));
                                    }
                                    downloader.split_chapters(
                                        &published_paths[0],
                                        &chapters,
                                        args.force_keyframes_at_cuts,
                                    )
                                })();
                                match split {
                                    Ok(()) => postprocess_progress("SplitChapters", "finished"),
                                    Err(error) => {
                                        postprocess_failed.set(true);
                                        if error_mode == ErrorMode::Ignore {
                                            report_error!("ERROR: {error}");
                                        } else {
                                            report_error!("ERROR: Postprocessing: {error}");
                                            return Ok(());
                                        }
                                    }
                                }
                            }
                            // Upstream `XAttrMetadataPP`, after SplitChapters.
                            if args.xattrs {
                                postprocess_progress("XAttrMetadata", "started");
                                match yt_dlp_rs::xattr::write_metadata(&published_paths[0], info) {
                                    Ok(warnings) => {
                                        for warning in warnings {
                                            warning!("WARNING: {warning}");
                                        }
                                        postprocess_progress("XAttrMetadata", "finished");
                                    }
                                    Err(error) => {
                                        let error = match error {
                                            yt_dlp_rs::Error::Postprocess(message) => message,
                                            other => other.to_string(),
                                        };
                                        postprocess_failed.set(true);
                                        if error_mode == ErrorMode::Ignore {
                                            report_error!("ERROR: {error}");
                                        } else {
                                            report_error!("ERROR: Postprocessing: {error}");
                                            return Ok(());
                                        }
                                    }
                                }
                            }
                            entry_files.borrow_mut()[entry_index] =
                                Some(move_files.final_path(&published_paths[0]));
                            // `-J`: `process_info`'s keys on the download's dict.
                            if args.dump_single_json {
                                let target = match json_entries
                                    .as_mut()
                                    .and_then(|entries| entries.get_mut(entry_index))
                                {
                                    Some(entry) => Some(entry),
                                    None if playlist_title.is_none() => single_json.as_mut(),
                                    None => None,
                                };
                                annotate_download(
                                    target,
                                    info,
                                    &path,
                                    process_existing.is_none(),
                                    &move_files.final_path(&published_paths[0]),
                                );
                            }
                            if !after_download(
                                &exec,
                                info,
                                &published_paths[0],
                                &final_path,
                                &move_files,
                                &template_options,
                                error_mode,
                                &pending_error,
                                &postprocess_failed,
                                &move_print,
                                &metadata_actions,
                            ) {
                                return Ok(());
                            }
                            if let Some(archive) = &mut archive {
                                archive.record(info).map_err(|error| {
                                    format!(
                                        "download completed, but archive update failed: {error}"
                                    )
                                })?;
                            }
                            let _ = bytes;
                        }
                        Ok(())
                    })()?;
                    // A break filter or download limit ends the remaining sections.
                    if entry_cancel.is_some() {
                        break;
                    }
                }
                Ok(())
            })();
            let mut extraction_failed = false;
            let succeeded = match result {
                Ok(()) => true,
                Err(error) if error.is::<EntryExtractionError>() => {
                    report_error!("ERROR: {error}");
                    failed = true;
                    extraction_failed = true;
                    if let Some(entry) = json_entries
                        .as_mut()
                        .and_then(|entries| entries.get_mut(entry_index))
                    {
                        *entry = serde_json::Value::Null;
                    } else if playlist_title.is_none() && single_json.is_some() {
                        single_json = Some(serde_json::Value::Null);
                    }
                    false
                }
                Err(error) => match error.downcast_ref::<yt_dlp_rs::Error>() {
                    Some(yt_dlp_rs::Error::CookieLoad(message)) => {
                        report_error!("ERROR: {message}");
                        failed = true;
                        aborted = true;
                        false
                    }
                    // Upstream skips the transfer without an error or archive
                    // record: HttpFD's `\r`-prefixed message, then (as
                    // `real_download` returns) the status printer's `end()`.
                    Some(yt_dlp_rs::Error::FileSizeLimit(message)) => {
                        screen!("\r[download] {message}");
                        if progress_options.enabled && !progress_options.newline {
                            ProgressReporter::write(if progress_options.quiet {
                                "\r\r"
                            } else {
                                "\n"
                            });
                        }
                        true
                    }
                    // A raised DownloadError: reported here unless the loaded
                    // info falls back to its URL (only --abort-on-error
                    // reports it first).
                    Some(yt_dlp_rs::Error::Raised(message)) => {
                        if from_info_json && !abort_on_error {
                            fallback = Some((message.clone(), info.webpage_url.to_string()));
                        } else {
                            report_error!("ERROR: {message}");
                            failed = true;
                        }
                        false
                    }
                    // Upstream `report_error` text without a URL/ID prefix.
                    Some(yt_dlp_rs::Error::Reported(message)) => {
                        report_error!("ERROR: {message}");
                        failed = true;
                        false
                    }
                    // `report_error(f'Postprocessing: {err}')`.
                    Some(yt_dlp_rs::Error::Postprocess(message)) => {
                        report_error!("ERROR: Postprocessing: {message}");
                        failed = true;
                        false
                    }
                    // A fatal fragment: `report_retry`, then `append_fragment`.
                    Some(yt_dlp_rs::Error::FragmentUnavailable { index, source }) => {
                        report_error!("ERROR: \r{source}");
                        report_error!("ERROR: fragment {index} not found, unable to continue");
                        failed = true;
                        false
                    }
                    // `RetryManager` exhaustion through `report_retry`.
                    Some(error @ yt_dlp_rs::Error::RetriesExhausted { .. }) => {
                        report_error!("ERROR: \r{error}");
                        failed = true;
                        false
                    }
                    // `process_info`: `unable to download video data: ERR` for
                    // network errors of the media download.
                    Some(
                        error @ (yt_dlp_rs::Error::Http(_)
                        | yt_dlp_rs::Error::HttpRetryAfter { .. }),
                    ) => {
                        report_error!(
                            "ERROR: unable to download video data: {}",
                            yt_dlp_rs::upstream_error_text(error)
                        );
                        failed = true;
                        false
                    }
                    _ => {
                        report_error!("ERROR: {url}: {}: {error}", info.id);
                        failed = true;
                        false
                    }
                },
            };
            failed |= postprocess_failed.get();
            // Upstream `_raise_pending_errors` after `process_info`, then the
            // `after_video` commands (without file paths).
            if let Some(error) = pending_error.take() {
                report_error!("ERROR: {error}");
                failed = true;
            }
            if counted
                && !(abort_on_error && failed)
                && let Err(error) = render_stage(
                    Stage::AfterVideo,
                    &print_templates,
                    &print_files,
                    &info,
                    prepare_media(&templates, &info, &template_options)
                        .map(|path| path.to_string_lossy().into_owned())
                        .as_deref(),
                    StagePaths::default(),
                    &template_options,
                )
                .map_err(|error| error.to_string())
                .and_then(|rendered| rendered.emit().map_err(|error| error.to_string()))
                .and_then(|()| {
                    // `MetadataParserPP` precedes `ExecPP`.
                    let mut info = info.clone();
                    if metadata_actions.has("after_video") {
                        set_pp_info(&info);
                        metadata_actions
                            .run_reported("after_video", &mut info, &template_options)
                            .map_err(|error| error.to_string())?;
                    }
                    run_exec(
                        &exec,
                        Stage::AfterVideo,
                        &info,
                        None,
                        None,
                        &template_options,
                    )
                })
            {
                report_error!("ERROR: {error}");
                failed = true;
            }
            if aborted {
                break;
            }
            // Upstream `--abort-on-error`: the first reported error raises and
            // ends the queue (a loaded info retries its URL instead).
            if abort_on_error && failed {
                if from_info_json && let Some(line) = LAST_ERROR.lock().unwrap().clone() {
                    fallback = Some((line, info.webpage_url.to_string()));
                    failed = failed_before;
                } else {
                    aborted = true;
                }
                break;
            }
            if fallback.is_some() {
                break;
            }
            extraction_failures += usize::from(extraction_failed);
            if let (Some(limit), Some(title)) = (
                args.skip_playlist_after_errors.filter(|limit| *limit != 0),
                &playlist_title,
            ) && extraction_failures as i64 >= limit
            {
                report_error!(
                    "ERROR: Skipping the remaining entries in playlist \"{title}\" since {extraction_failures} items failed extraction"
                );
                failed = true;
                break;
            }
            if let Some(reason) = entry_cancel {
                cancelled = cancel(reason, args.break_per_input);
                if cancelled.is_none() {
                    processed = 0;
                }
                url_cancelled = true;
                break;
            }
            if counted {
                processed += 1;
            }
            if succeeded && counted && max_downloads.is_some_and(|limit| processed >= limit) {
                cancelled = cancel(Cancel::MaxDownloads, args.break_per_input);
                if cancelled.is_none() {
                    processed = 0;
                }
                // The rest of this input is skipped either way.
                url_cancelled = true;
                break;
            }
        }
        if let Some((message, webpage_url)) = fallback {
            eprintln!("\r");
            warning!(
                "WARNING: The info failed to download: {message}; trying with URL {webpage_url}"
            );
            inputs.push_front((webpage_url, None));
            continue;
        }
        if cancelled.is_some() || aborted {
            break;
        }
        if let Some(reason) = pending.take().filter(|_| !url_cancelled) {
            // Entries before the archived/rejected one were processed; stop now.
            cancelled = cancel(reason, args.break_per_input);
            if cancelled.is_some() {
                break;
            }
            processed = 0;
            continue;
        }
        if url_cancelled {
            continue;
        }
        // "Write the updated info to json": the cleaned dict loses its
        // entries, so the content is the playlist's again.
        if let Some((path, value)) = &playlist_info_json {
            match write_info_json_with(path, value, true) {
                Ok(written) => report_info_json("updated playlist", path, written),
                // Upstream reports the failure without the OS error.
                Err(_) => {
                    report_error!(
                        "ERROR: Cannot write updated playlist metadata to JSON file {}",
                        path.display()
                    );
                    failed = true;
                    continue;
                }
            }
        }
        if let Some(dict) = &playlist_dict
            && !run_playlist_stage(
                dict,
                &entry_files.borrow(),
                &print_templates,
                &print_files,
                &exec,
                &templates,
                &template_options,
                &downloader,
                PlaylistStage {
                    simulate,
                    concat: args.concat_playlist == "always",
                    concat_pp: args.concat_playlist != "never",
                    keep: args.keep_video,
                },
                &metadata_actions,
            )?
        {
            failed = true;
        }
        if let Some(title) = &playlist_title {
            screen!("[download] Finished downloading playlist: {title}");
        }
        if let (Some(object), Some(entries)) = (
            single_json.as_mut().and_then(|value| value.as_object_mut()),
            json_entries,
        ) {
            object.insert("entries".into(), entries);
        }
        if let Some(mut value) = single_json {
            // `sanitize_info` defaults on the printed result.
            if let Some(object) = value.as_object_mut() {
                object.entry("epoch").or_insert(epoch().into());
                object.entry("_type").or_insert("video".into());
            }
            yt_dlp_rs::upstream_shape(&mut value);
            writeln!(stdout, "{}", yt_dlp_rs::output::python_json_dumps(&value))?;
            // One `write_string` call per message.
            stdout.flush()?;
        }
    }
    for warning in downloader.cookie_jar().take_warnings() {
        warning!("WARNING: {warning}");
    }
    if let Some(path) = &args.cookies {
        let cookies = downloader.cookie_jar();
        let save = if cookie_file_was_missing {
            cookies.save_new(path)
        } else {
            cookies.save(path)
        };
        if let Err(error) = save {
            report_error!("ERROR: {error}");
            failed = true;
        }
    }
    // Cookie loading's `report_error` lines count like other errors.
    failed |= yt_dlp_rs::screen::cookies_reported_error();
    if let Some(cancel) = cancelled {
        return Err(Box::new(cancel));
    }
    Ok(!failed)
}

/// Run `--parse-metadata`/`--replace-in-metadata` actions for `when`.
fn run_metadata_parser(
    actions: &yt_dlp_rs::metadata_parser::MetadataActions,
    when: &str,
    info: &mut yt_dlp_rs::VideoInfo,
    options: &TemplateOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    if actions.has(when) {
        set_pp_info(info);
    }
    Ok(actions.run_reported(when, info, options)?)
}

/// Upstream `__list_table`: the caption (or `ID has no NAME`) is a screen
/// message and only the table is written to stdout.
fn emit_listing(stdout: &mut impl Write, text: &str) -> io::Result<()> {
    let (caption, table) = text.split_once('\n').unwrap_or((text, ""));
    screen!("{caption}");
    stdout.write_all(table.as_bytes())?;
    stdout.flush()
}

/// An entry `ExtractorError` from upstream `process_video_result`, reported
/// as `ERROR: [EXTRACTOR] ID: MESSAGE`.
#[derive(Debug)]
struct EntryExtractionError(String);

impl std::fmt::Display for EntryExtractionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for EntryExtractionError {}

/// An upstream `parser.error`: usage, `PROG: error: MESSAGE`, exit status 2.
#[derive(Debug)]
struct UsageError(String);

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for UsageError {}

/// optparse's `BadOptionError` text for an unknown option.
fn unknown_option(error: &clap::Error) -> String {
    let argument = error
        .get(clap::error::ContextKind::InvalidArg)
        .map(ToString::to_string)
        .unwrap_or_default();
    format!(
        "no such option: {}",
        argument.split('=').next().unwrap_or_default()
    )
}

/// Every long option (with its aliases and the deprecated ones stripped
/// before parsing) and whether it takes a separate value.
fn long_options() -> Vec<(String, bool)> {
    let mut options: Vec<(String, bool)> = command()
        .get_arguments()
        .filter(|argument| !argument.is_positional())
        .flat_map(|argument| {
            let takes_value = argument.get_action().takes_values();
            argument
                .get_long_and_visible_aliases()
                .into_iter()
                .flatten()
                .chain(argument.get_all_aliases().into_iter().flatten())
                .map(move |name| (format!("--{name}"), takes_value))
                .collect::<Vec<_>>()
        })
        .collect();
    options.extend(
        DEPRECATED_SWITCHES
            .iter()
            .map(|name| ((*name).to_owned(), false)),
    );
    options.extend(
        DEPRECATED_ARGUMENTS
            .iter()
            .map(|name| ((*name).to_owned(), true)),
    );
    // Expanded before the parser runs.
    options.extend(["--alias", "--preset-alias"].map(|name| (name.to_owned(), true)));
    options.sort();
    options.dedup();
    options
}

/// optparse `_match_long_opt` for `--NAME[=VALUE]`: an unambiguous prefix
/// of a long option (or a defined `--alias`) stands for it, an ambiguous one
/// is an error listing the sorted candidates, and an unknown one is left for
/// the parser. A switch given a value is optparse's error.
fn resolve_long(
    token: &str,
    options: &[(String, bool)],
    aliases: &[(String, usize, String, usize)],
) -> Result<String, String> {
    let (name, value) = match token.split_once('=') {
        Some((name, value)) => (name, Some(value)),
        None => (token, None),
    };
    let mut candidates: Vec<(&str, bool)> = options
        .iter()
        .map(|(option, takes_value)| (option.as_str(), *takes_value))
        .chain(
            aliases
                .iter()
                .map(|(alias, nargs, ..)| (alias.as_str(), *nargs > 0)),
        )
        .collect();
    candidates.sort();
    candidates.dedup();
    let matched: Vec<(&str, bool)> = match candidates.iter().find(|(option, _)| *option == name) {
        Some(exact) => vec![*exact],
        None => candidates
            .iter()
            .filter(|(option, _)| option.starts_with(name))
            .copied()
            .collect(),
    };
    match matched.as_slice() {
        [] => Ok(token.to_owned()),
        [(option, false)] if value.is_some() => {
            Err(format!("{option} option does not take a value"))
        }
        [(option, _)] => Ok(match value {
            Some(value) => format!("{option}={value}"),
            None => (*option).to_owned(),
        }),
        many => {
            let names: Vec<&str> = many.iter().map(|(option, _)| *option).collect();
            Err(format!("ambiguous option: {name} ({}?)", names.join(", ")))
        }
    }
}

/// The parser, with optparse's value handling: an option that takes a
/// value consumes the next argument even when it starts with `-`.
fn command() -> clap::Command {
    Args::command().mut_args(|argument| {
        let takes_value = argument.get_action().takes_values() && !argument.is_positional();
        argument.allow_hyphen_values(takes_value)
    })
}

/// Upstream validation for options this port accepts without acting on
/// them, and warnings where upstream would behave differently.
/// Upstream `--compat-options` values.
const COMPAT: [&str; 29] = [
    "abort-on-error",
    "allow-unsafe-exec-expansion",
    "allow-unsafe-ext",
    "embed-metadata",
    "embed-thumbnail-atomicparsley",
    "filename",
    "filename-sanitization",
    "format-sort",
    "format-spec",
    "list-formats",
    "manifest-filesize-approx",
    "mtime-by-default",
    "multistreams",
    "no-attach-info-json",
    "no-certifi",
    "no-clean-infojson",
    "no-direct-merge",
    "no-external-downloader-progress",
    "no-keep-subs",
    "no-live-chat",
    "no-playlist-metafiles",
    "no-youtube-channel-redirect",
    "no-youtube-prefer-utc-upload-date",
    "no-youtube-unavailable-videos",
    "playlist-index",
    "playlist-match-filter",
    "prefer-legacy-http-handler",
    "prefer-vp9-sort",
    "seperate-video-versions",
];

/// Upstream `_set_from_options_callback` with `orderedSet_from_options`:
/// aliases expand, `-NAME` removes, and `all` is every option.
fn compat_options(values: &[String]) -> Result<Vec<String>, UsageError> {
    fn alias(name: &str) -> Option<Vec<&'static str>> {
        Some(match name {
            "all" => COMPAT.to_vec(),
            "youtube-dl" => vec![
                "all",
                "-multistreams",
                "-playlist-match-filter",
                "-manifest-filesize-approx",
                "-allow-unsafe-ext",
                "-prefer-vp9-sort",
                "-allow-unsafe-exec-expansion",
            ],
            "youtube-dlc" => vec![
                "all",
                "-no-youtube-channel-redirect",
                "-no-live-chat",
                "-playlist-match-filter",
                "-manifest-filesize-approx",
                "-allow-unsafe-ext",
                "-prefer-vp9-sort",
                "-allow-unsafe-exec-expansion",
            ],
            "2021" => vec!["2022", "no-certifi", "filename-sanitization"],
            "2022" => vec![
                "2023",
                "no-external-downloader-progress",
                "playlist-match-filter",
                "prefer-legacy-http-handler",
                "manifest-filesize-approx",
            ],
            "2023" => vec!["2024", "prefer-vp9-sort"],
            "2024" => vec!["2025", "mtime-by-default"],
            "2025" => vec![],
            _ => return None,
        })
    }
    fn apply(values: &[String], requested: &mut Vec<String>) -> Result<(), String> {
        for value in values {
            let (discard, name) = match value.strip_prefix('-') {
                Some(name) => (true, name),
                None => (false, value.as_str()),
            };
            if let Some(expansion) = alias(name) {
                let expansion: Vec<String> = expansion
                    .iter()
                    .map(|item| match (discard, item.strip_prefix('-')) {
                        (false, _) => (*item).to_owned(),
                        (true, Some(rest)) => rest.to_owned(),
                        (true, None) => format!("-{item}"),
                    })
                    .collect();
                apply(&expansion, requested)?;
                continue;
            }
            if !COMPAT.contains(&name) {
                return Err(name.to_owned());
            }
            if discard {
                requested.retain(|item| item != name);
            } else {
                requested.push(name.to_owned());
            }
        }
        Ok(())
    }
    let mut requested = Vec::new();
    for value in values {
        let parts: Vec<String> = value
            .split(',')
            .map(|part| part.trim().to_lowercase())
            .collect();
        apply(&parts, &mut requested)
            .map_err(|name| usage(format!("wrong OPTS for --compat-options: {name}")))?;
    }
    let mut seen = std::collections::HashSet::new();
    requested.retain(|item| seen.insert(item.clone()));
    Ok(requested)
}

/// `--hls-use-mpegts`/`--no-hls-use-mpegts` (the last given), else `None`.
fn hls_use_mpegts(args: &Args) -> Option<bool> {
    if args.hls_use_mpegts {
        Some(true)
    } else if args.no_hls_use_mpegts {
        Some(false)
    } else {
        None
    }
}

fn validate_accepted_options(args: &Args) -> Result<(), UsageError> {
    // `--encoding`: messages are encoded with `errors='ignore'`.
    if let Some(name) = &args.encoding {
        match yt_dlp_rs::output::encoding::Encoding::lookup(name) {
            Some(encoding) => yt_dlp_rs::output::encoding::set_output(encoding),
            None => {
                warning!("WARNING: --encoding {name} is not supported natively; output is UTF-8")
            }
        }
    }
    for (name, value, strict) in [
        ("buffer size", &args.buffer_size, true),
        ("throttled rate limit", &args.throttled_rate, false),
    ] {
        if let Some(value) = value {
            match parse_bytes(value) {
                None => return Err(usage(format!("invalid {name} \"{value}\" given"))),
                Some(0) if strict => {
                    return Err(usage(format!("{name} \"{value}\" must be positive")));
                }
                _ => {}
            }
        }
    }
    let file_access = yt_dlp_rs::batch::py_strip(&args.file_access_retries);
    if !matches!(file_access, "inf" | "infinite")
        && file_access.trim_start_matches('+').parse::<u64>().is_err()
    {
        return Err(usage(format!(
            "invalid file access retry count \"{}\" given",
            args.file_access_retries
        )));
    }
    if let Some(range) = &args.wait_for_video {
        let parse = |text: &str| yt_dlp_rs::parse_duration(text);
        let py_str = |value: yt_dlp_rs::PyNumber| match value {
            yt_dlp_rs::PyNumber::Int(value) => value.to_string(),
            yt_dlp_rs::PyNumber::Float(value) => py_float(value),
        };
        let (min, max) = match range.split_once('-') {
            Some((min, max)) => (parse(min), Some(parse(max))),
            None => (parse(range), None),
        };
        let invalid = || {
            usage(format!(
                "invalid time range to wait for video \"{range}\" given"
            ))
        };
        let min = min.ok_or_else(invalid)?;
        if let Some(max) = max {
            let max = max.ok_or_else(invalid)?;
            if max.as_f64() < min.as_f64() {
                return Err(usage(format!(
                    "max time range to wait for video \"{}\" must be must be greater than or equal to min time range to wait for video \"{}\"",
                    py_str(max),
                    py_str(min)
                )));
            }
        }
    }
    compat_options(&args.compat_options)?;
    for value in &args.js_runtimes {
        let name = value.split(':').next().unwrap_or(value).to_lowercase();
        if !matches!(name.as_str(), "deno" | "node" | "bun" | "quickjs") {
            warning!(
                "WARNING: Ignoring unsupported JavaScript runtime(s): {name}. Supported runtimes: deno, node, bun, quickjs."
            );
        } else if !matches!(name.as_str(), "deno" | "node") {
            warning!(
                "WARNING: JavaScript runtime {name} is not supported natively; use deno or node"
            );
        }
    }
    for (given, what) in [
        (args.legacy_server_connect, "--legacy-server-connect"),
        (args.bidi_workaround, "--bidi-workaround"),
        (!args.plugin_dirs.is_empty(), "--plugin-dirs"),
    ] {
        if given {
            warning!("WARNING: {what} is not supported natively and is ignored");
        }
    }
    if args.allow_unplayable_formats {
        warning!(
            "WARNING: You have asked for UNPLAYABLE formats to be listed/downloaded. This is a developer option intended for debugging. \n         If you experience any issues while using this option, DO NOT open a bug report"
        );
    }
    Ok(())
}

/// Upstream `_PRESET_ALIASES`.
const PRESET_ALIASES: [(&str, &[&str]); 5] = [
    (
        "mp3",
        &["-f", "ba[acodec^=mp3]/ba/b", "-x", "--audio-format", "mp3"],
    ),
    (
        "aac",
        &[
            "-f",
            "ba[acodec^=aac]/ba[acodec^=mp4a.40.]/ba/b",
            "-x",
            "--audio-format",
            "aac",
        ],
    ),
    (
        "mp4",
        &[
            "--merge-output-format",
            "mp4",
            "--remux-video",
            "mp4",
            "-S",
            "vcodec:h264,lang,quality,res,fps,hdr:12,acodec:aac",
        ],
    ),
    (
        "mkv",
        &["--merge-output-format", "mkv", "--remux-video", "mkv"],
    ),
    (
        "sleep",
        &[
            "--sleep-subtitles",
            "5",
            "--sleep-requests",
            "0.75",
            "--sleep-interval",
            "10",
            "--max-sleep-interval",
            "20",
        ],
    ),
];

/// Upstream `-t`/`--preset-alias NAME`: the preset's options replace the
/// alias in place (`parser.rargs[:0] = ...`). Option values (including
/// another option's `-t` value) and arguments after `--` are left alone.
fn expand_preset_aliases(arguments: Vec<String>) -> Result<Vec<String>, String> {
    // Value arities are only known once the command is built.
    let mut command = Args::command();
    command.build();
    let takes_value = |token: &str| -> bool {
        if DEPRECATED_ARGUMENTS.contains(&token) {
            return true;
        }
        let found = if let Some(long) = token.strip_prefix("--") {
            command.get_arguments().find(|argument| {
                argument.get_long() == Some(long)
                    || argument
                        .get_all_aliases()
                        .is_some_and(|aliases| aliases.contains(&long))
            })
        } else {
            let short = token.chars().nth(1);
            command
                .get_arguments()
                .find(|argument| argument.get_short() == short)
        };
        found.is_some_and(|argument| {
            argument
                .get_num_args()
                .is_some_and(|range| range.takes_values())
        })
    };
    let arity = |token: &str| -> usize {
        let long = token.strip_prefix("--").unwrap_or(token);
        let found = command.get_arguments().find(|argument| {
            argument.get_long() == Some(long)
                || argument
                    .get_all_aliases()
                    .is_some_and(|aliases| aliases.contains(&long))
        });
        found
            .and_then(|argument| argument.get_num_args())
            .map_or(1, |range| range.min_values().max(1))
    };
    let mut output = Vec::with_capacity(arguments.len());
    let mut queue: std::collections::VecDeque<String> = arguments.into();
    // Upstream `--alias ALIASES OPTIONS`: option strings with their argument
    // count, template and trigger count.
    let mut aliases: Vec<(String, usize, String, usize)> = Vec::new();
    let options = long_options();
    let shorts: Vec<(char, bool)> = command
        .get_arguments()
        .filter_map(|argument| {
            argument.get_short().map(|short| {
                (
                    short,
                    argument
                        .get_num_args()
                        .is_some_and(|range| range.takes_values()),
                )
            })
        })
        .collect();
    while let Some(token) = queue.pop_front() {
        if token == "--" {
            output.push(token);
            output.extend(queue);
            break;
        }
        let token = if token.starts_with("--") {
            resolve_long(&token, &options, &aliases)?
        } else {
            token
        };
        if token == "--alias" {
            let (Some(names), Some(options)) = (queue.pop_front(), queue.pop_front()) else {
                return Err("--alias option requires 2 arguments".into());
            };
            let nargs = alias_arguments(&options)
                .map_err(|error| format!("wrong --alias OPTIONS formatting; {error}"))?;
            for name in names.split(',').map(str::trim) {
                let name = if name.starts_with('-') {
                    name.to_owned()
                } else {
                    format!("--{name}")
                };
                aliases.retain(|(existing, ..)| *existing != name);
                aliases.push((name, nargs, options.clone(), 0));
            }
            continue;
        }
        if let Some((name, nargs, options, triggers)) =
            aliases.iter_mut().find(|(name, ..)| *name == token)
        {
            *triggers += 1;
            if *triggers > 100 {
                return Err(format!("Alias {name} exceeded invocation limit"));
            }
            let mut values = Vec::new();
            for _ in 0..*nargs {
                values.push(queue.pop_front().ok_or_else(|| {
                    format!(
                        "{name} option requires {nargs} argument{}",
                        if *nargs == 1 { "" } else { "s" }
                    )
                })?);
            }
            let quoted: Vec<String> = values
                .iter()
                .map(|value| yt_dlp_rs::output::shell_quote(value))
                .collect();
            // Upstream only formats templates that take arguments.
            let text = if *nargs == 0 {
                options.clone()
            } else {
                format_alias(options, &quoted)?
            };
            let expanded = yt_dlp_rs::config::shlex_split(&text)?;
            for (index, value) in expanded.into_iter().enumerate() {
                queue.insert(index, value);
            }
            continue;
        }
        let name = if token == "-t" || token == "--preset-alias" {
            Some(
                queue
                    .pop_front()
                    .ok_or_else(|| format!("{token} option requires 1 argument"))?,
            )
        } else {
            token
                .strip_prefix("--preset-alias=")
                .or_else(|| {
                    token
                        .strip_prefix("-t")
                        .filter(|_| !token.starts_with("--"))
                })
                .map(str::to_owned)
        };
        if let Some(name) = name {
            let preset = PRESET_ALIASES
                .iter()
                .find(|(preset, _)| *preset == name)
                .ok_or_else(|| format!("Unknown preset alias: {name}"))?;
            output.extend(preset.1.iter().map(|value| (*value).to_owned()));
            continue;
        }
        // A short option's attached value is the rest of the argument,
        // verbatim (`-f=best` is `=best`).
        if !token.starts_with("--") && token.starts_with('-') && token.len() > 2 {
            let cluster = &token[1..];
            let attached = cluster.char_indices().find_map(|(index, short)| {
                shorts
                    .iter()
                    .find(|(name, takes_value)| *name == short && *takes_value)
                    .map(|_| index + short.len_utf8())
            });
            if let Some(end) = attached.filter(|end| *end < cluster.len()) {
                output.push(format!("-{}", &cluster[..end]));
                output.push(cluster[end..].to_owned());
                continue;
            }
        }
        // Keep option values with their option.
        let inline = token.starts_with("--") && token.contains('=');
        let short_inline = !token.starts_with("--") && token.starts_with('-') && token.len() > 2;
        if token.starts_with('-')
            && token.len() > 1
            && !inline
            && !short_inline
            && takes_value(&token)
        {
            let count = if token.starts_with("--") {
                arity(&token)
            } else {
                1
            };
            output.push(token);
            for _ in 0..count {
                output.extend(queue.pop_front());
            }
            continue;
        }
        output.push(token);
    }
    Ok(output)
}

/// The number of distinct `str.format` fields in an alias template
/// (`{}` fields count by position, like `Formatter.parse`).
fn alias_arguments(template: &str) -> Result<usize, String> {
    let mut fields = Vec::new();
    let mut auto = 0;
    for field in alias_fields(template)? {
        let key = if field.is_empty() {
            let key = auto.to_string();
            auto += 1;
            key
        } else {
            field
        };
        if !fields.contains(&key) {
            fields.push(key);
        }
    }
    // Validate like `opts.format(*range(nargs))`.
    format_alias(
        template,
        &(0..fields.len()).map(|i| i.to_string()).collect::<Vec<_>>(),
    )?;
    Ok(fields.len())
}

/// `str.format` field names in order (`{{`/`}}` are literal braces).
fn alias_fields(template: &str) -> Result<Vec<String>, String> {
    let mut fields = Vec::new();
    let mut chars = template.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
            }
            '{' => {
                let mut field = String::new();
                loop {
                    match chars.next() {
                        Some('}') => break,
                        Some(c) => field.push(c),
                        None => return Err("expected '}' before end of string".into()),
                    }
                }
                fields.push(field);
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
            }
            '}' => return Err("Single '}' encountered in format string".into()),
            _ => {}
        }
    }
    Ok(fields)
}

/// Python `template.format(*values)` for positional fields.
fn format_alias(template: &str, values: &[String]) -> Result<String, String> {
    let mut output = String::new();
    let mut chars = template.chars().peekable();
    let mut auto = 0;
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                output.push('{');
            }
            '{' => {
                let field: String = chars.by_ref().take_while(|c| *c != '}').collect();
                let index = if field.is_empty() {
                    auto += 1;
                    auto - 1
                } else {
                    field
                        .parse::<usize>()
                        .map_err(|_| format!("unsupported format field {{{field}}}"))?
                };
                output.push_str(values.get(index).ok_or_else(|| {
                    format!("Replacement index {index} out of range for positional args tuple")
                })?);
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                output.push('}');
            }
            '}' => return Err("Single '}' encountered in format string".into()),
            other => output.push(other),
        }
    }
    Ok(output)
}

/// Upstream's deprecated switches, which take no effect.
const DEPRECATED_SWITCHES: &[&str] = &[
    "--xattr-set-filesize",
    "--dump-user-agent",
    "--youtube-include-dash-manifest",
    "--no-youtube-skip-dash-manifest",
    "--youtube-skip-dash-manifest",
    "--no-youtube-include-dash-manifest",
    "--youtube-include-hls-manifest",
    "--no-youtube-skip-hls-manifest",
    "--youtube-skip-hls-manifest",
    "--no-youtube-include-hls-manifest",
    "--youtube-print-sig-code",
    "--sponskrub",
    "--no-sponskrub",
    "--sponskrub-cut",
    "--no-sponskrub-cut",
    "--sponskrub-force",
    "--no-sponskrub-force",
    "--prefer-avconv",
    "--no-prefer-ffmpeg",
    "--prefer-ffmpeg",
    "--no-prefer-avconv",
    "-C",
    "--call-home",
    "--no-call-home",
    "--include-ads",
    "--no-include-ads",
    "--write-annotations",
    "--no-write-annotations",
];
/// Upstream's deprecated options with a value.
const DEPRECATED_ARGUMENTS: &[&str] = &[
    "--sponskrub-location",
    "--sponskrub-args",
    "--cn-verification-proxy",
];

/// Upstream `deprecated_switches`/`deprecated_arguments`: accepted no-ops.
fn strip_deprecated_options(arguments: Vec<String>) -> (Vec<String>, Vec<String>) {
    let mut kept = Vec::with_capacity(arguments.len());
    let mut deprecated = Vec::new();
    let mut iter = arguments.into_iter();
    while let Some(argument) = iter.next() {
        if argument == "--" {
            kept.push(argument);
            kept.extend(iter);
            break;
        }
        let name = argument
            .split_once('=')
            .map_or(argument.as_str(), |(name, _)| name);
        if DEPRECATED_SWITCHES.contains(&argument.as_str()) {
            deprecated.push(argument);
        } else if DEPRECATED_ARGUMENTS.contains(&name) {
            if !argument.contains('=') {
                iter.next();
            }
            deprecated.push(name.to_owned());
        } else {
            kept.push(argument);
        }
    }
    (kept, deprecated)
}

/// What one configuration's own arguments say about further configuration
/// (upstream `parse_known_args` of those arguments).
#[derive(Default)]
struct ConfigScan {
    ignore_config: bool,
    locations: Vec<String>,
    home: Option<String>,
}

/// Find `--ignore-config`, `--config-locations` and `-P` home values in an
/// argument list, using clap's option table to skip option values.
fn scan_config_options(command: &clap::Command, args: &[String]) -> ConfigScan {
    let mut scan = ConfigScan::default();
    let find_long = |name: &str| {
        command.get_arguments().find(|arg| {
            arg.get_long() == Some(name)
                || arg
                    .get_all_aliases()
                    .is_some_and(|aliases| aliases.contains(&name))
        })
    };
    let takes_value = |arg: &clap::Arg| arg.get_action().takes_values();
    let mut record = |id: &str, value: Option<&str>| match (id, value) {
        ("ignore_config", _) => scan.ignore_config = true,
        ("no_config_locations", _) => scan.locations.clear(),
        ("config_locations", Some(value)) => scan.locations.push(value.to_owned()),
        ("paths", Some(value)) => {
            // `-P [TYPES:]PATH` with the shared typed-key rule.
            if let Ok(paths) = yt_dlp_rs::output::parse_paths(&[value.to_owned()])
                && let Some(home) = paths.get("home")
            {
                scan.home = Some(home.clone());
            }
        }
        _ => {}
    };
    let mut index = 0;
    while index < args.len() {
        let token = &args[index];
        index += 1;
        if token == "--" {
            break;
        }
        if let Some(long) = token.strip_prefix("--") {
            let (name, inline) = match long.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (long, None),
            };
            let Some(arg) = find_long(name) else {
                continue;
            };
            let value = if takes_value(arg) {
                inline.or_else(|| {
                    let value = args.get(index).map(String::as_str);
                    index += 1;
                    value
                })
            } else {
                None
            };
            record(arg.get_id().as_str(), value);
        } else if let Some(cluster) = token.strip_prefix('-').filter(|rest| !rest.is_empty()) {
            for (offset, short) in cluster.char_indices() {
                let Some(arg) = command.get_arguments().find(|arg| {
                    arg.get_short() == Some(short)
                        || arg
                            .get_short_and_visible_aliases()
                            .is_some_and(|aliases| aliases.contains(&short))
                }) else {
                    break;
                };
                if takes_value(arg) {
                    let rest = &cluster[offset + short.len_utf8()..];
                    let value = if rest.is_empty() {
                        let value = args.get(index).cloned();
                        index += 1;
                        value
                    } else {
                        Some(rest.to_owned())
                    };
                    record(arg.get_id().as_str(), value.as_deref());
                    break;
                }
                record(arg.get_id().as_str(), None);
            }
        }
    }
    scan
}

/// One loaded configuration and the configurations its locations name.
struct ConfigNode {
    args: Vec<String>,
    children: Vec<ConfigNode>,
    /// Upstream `Config.label` (`None` for `--config-locations` files).
    label: Option<&'static str>,
    filename: Option<PathBuf>,
}

/// Python `repr(list_of_str)`.
fn py_list(items: &[String]) -> String {
    let items: Vec<String> = items
        .iter()
        .map(|item| yt_dlp_rs::batch::py_repr(item))
        .collect();
    format!("[{}]", items.join(", "))
}

/// Upstream `Config.hide_login_info`.
fn hide_login_info(args: &[String]) -> Vec<String> {
    const PRIVATE: [&str; 7] = [
        "-p",
        "--password",
        "-u",
        "--username",
        "--video-password",
        "--ap-password",
        "--ap-username",
    ];
    let mut args: Vec<String> = args
        .iter()
        .map(|arg| match arg.split_once('=') {
            Some((key, value)) if PRIVATE.contains(&key) && !value.is_empty() => {
                format!("{key}=PRIVATE")
            }
            _ => arg.clone(),
        })
        .collect();
    for index in 0..args.len() {
        if PRIVATE.contains(&args[index].as_str()) && index + 1 < args.len() {
            args[index + 1] = "PRIVATE".into();
        }
    }
    args
}

impl ConfigNode {
    /// Upstream `Config.__str__`, as lines; nested configurations are
    /// prefixed with `| `.
    fn debug_lines(&self, out: &mut Vec<String>) {
        let mut label = [
            self.label.unwrap_or(""),
            "config",
            &self
                .filename
                .as_ref()
                .map(|path| format!("\"{}\"", path.display()))
                .unwrap_or_default(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
        if let Some(first) = label.get(..1) {
            label = format!("{}{}", first.to_uppercase(), &label[1..]);
        }
        out.push(format!(
            "{label}: {}",
            py_list(&hide_login_info(&self.args))
        ));
        for child in &self.children {
            let mut lines = Vec::new();
            child.debug_lines(&mut lines);
            out.extend(lines.into_iter().map(|line| format!("| {line}")));
        }
    }

    /// Upstream `Config.all_args`: later locations first, then own arguments.
    fn all_args(&self, out: &mut Vec<String>) {
        for child in self.children.iter().rev() {
            child.all_args(out);
        }
        out.extend(self.args.iter().cloned());
    }
}

/// Upstream `Config.init`/`load_configs` for one configuration; `None` when
/// the file was already loaded.
fn load_config(
    command: &clap::Command,
    args: Vec<String>,
    filename: Option<&Path>,
    loaded: &mut std::collections::HashSet<PathBuf>,
) -> Result<Option<ConfigNode>, UsageError> {
    let mut directory = PathBuf::new();
    if let Some(filename) = filename {
        let location = std::fs::canonicalize(filename).unwrap_or_else(|_| filename.into());
        directory = location.parent().map(Path::to_path_buf).unwrap_or_default();
        if !loaded.insert(location) {
            return Ok(None);
        }
    }
    let mut node = ConfigNode {
        args: Vec::new(),
        children: Vec::new(),
        label: None,
        filename: filename.map(Path::to_path_buf),
    };
    for location in scan_config_options(command, &args).locations {
        if location == "-" {
            if !loaded.insert(PathBuf::from("-")) {
                continue;
            }
            eprintln!("Reading options from STDIN - EOF (Ctrl+D) to end:");
            let mut text = String::new();
            std::io::Read::read_to_string(&mut io::stdin(), &mut text)
                .map_err(|error| usage(error.to_string()))?;
            let stdin_args = yt_dlp_rs::config::shlex_split(&text).map_err(usage)?;
            if let Some(mut child) = load_config(command, stdin_args, None, loaded)? {
                child.label = Some("stdin");
                node.children.push(child);
            }
            continue;
        }
        let mut path = directory.join(yt_dlp_rs::batch::expand_path_text(&location));
        if path.is_dir() {
            path = path.join(format!("{}.conf", yt_dlp_rs::config::PACKAGE_NAME));
        }
        if !path.exists() {
            return Err(usage(format!(
                "config location {} does not exist",
                path.display()
            )));
        }
        let file_args = yt_dlp_rs::config::read_file(&path)
            .map_err(usage)?
            .unwrap_or_default();
        if let Some(child) = load_config(command, file_args, Some(&path), loaded)? {
            node.children.push(child);
        }
    }
    node.args = args;
    Ok(Some(node))
}

/// Upstream `parseOpts` configuration assembly: command line, then
/// portable, home, user and system configurations until `--ignore-config`,
/// flattened so the command line takes precedence, with the `--verbose`
/// configuration debug lines.
fn config_arguments(cli: Vec<String>) -> Result<(Vec<String>, Vec<String>), UsageError> {
    use yt_dlp_rs::config::{
        PACKAGE_NAME, first_config, read_file, system_config_dirs, user_config_dirs,
    };
    let command = Args::command();
    let mut loaded = std::collections::HashSet::new();
    // (label, node) in append order; `all_args` reverses it.
    let mut configs: Vec<(&'static str, ConfigNode)> = Vec::new();
    let flatten = |configs: &[(&str, ConfigNode)]| {
        let mut out = Vec::new();
        for (_, node) in configs.iter().rev() {
            node.all_args(&mut out);
        }
        out
    };
    if let Some(mut node) = load_config(&command, cli, None, &mut loaded)? {
        node.label = Some("Command-line");
        configs.push(("Command-line", node));
    }
    let mut loaded_all = true;
    for label in ["Portable", "Home", "User", "System"] {
        let current = scan_config_options(&command, &flatten(&configs));
        if current.ignore_config {
            loaded_all = false;
            break;
        }
        let found = match label {
            "Portable" => std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(Path::to_path_buf))
                .map(|dir| dir.join(format!("{PACKAGE_NAME}.conf")))
                .map(|path| read_file(&path).map(|args| args.map(|args| (args, path))))
                .transpose()
                .map_err(usage)?
                .flatten(),
            "Home" => {
                let home = current.home.as_deref().unwrap_or("");
                let home = yt_dlp_rs::batch::expand_path_text(home);
                let path = Path::new(home.trim()).join(format!("{PACKAGE_NAME}.conf"));
                read_file(&path).map_err(usage)?.map(|args| (args, path))
            }
            "User" => {
                let home = std::env::var_os("HOME").map(PathBuf::from);
                first_config(&user_config_dirs(home.as_deref())).map_err(usage)?
            }
            _ => first_config(&system_config_dirs()).map_err(usage)?,
        };
        if let Some((args, path)) = found
            && let Some(mut node) = load_config(&command, args, Some(&path), &mut loaded)?
        {
            node.label = Some(label);
            configs.push((label, node));
        }
    }
    // `--ignore-config` inside the system configuration drops the user one.
    if loaded_all && scan_config_options(&command, &flatten(&configs)).ignore_config {
        configs.retain(|(label, _)| *label != "User");
    }
    let mut debug = Vec::new();
    for (_, node) in &configs {
        node.debug_lines(&mut debug);
    }
    Ok((flatten(&configs), debug))
}

fn usage(message: impl std::fmt::Display) -> UsageError {
    UsageError(message.to_string())
}

/// Upstream `_write_link_file` for each requested type. Like upstream, an
/// existing shortcut is kept unless `-w`/`--no-overwrites` is given.
fn write_links(
    kinds: &[&str],
    info: &yt_dlp_rs::VideoInfo,
    templates: &OutputTemplates,
    options: &TemplateOptions,
    overwrites: Option<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(uri) = yt_dlp_rs::output::iri_to_uri(&info.webpage_url) else {
        warning!(
            "WARNING: Cannot write internet shortcut file because the actual URL of \"{}\" is unknown or disallowed",
            info.webpage_url
        );
        return Ok(());
    };
    let Some(base) = templates.prepare(info, Some("link"), options)? else {
        return Ok(());
    };
    for kind in kinds {
        let path = PathBuf::from(yt_dlp_rs::output::replace_extension(
            &base.to_string_lossy(),
            kind,
            Some(&info.ext),
        ));
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        if overwrites != Some(false) && path.exists() {
            screen!("[info] Internet shortcut (.{kind}) is already present");
            continue;
        }
        screen!(
            "[info] Writing internet shortcut (.{kind}) to: {}",
            path.display()
        );
        let text = yt_dlp_rs::output::link_file(kind, &uri, &path.to_string_lossy());
        std::fs::write(&path, text)
            .map_err(|_| format!("Cannot write internet shortcut {}", path.display()))?;
    }
    Ok(())
}

fn stage_paths<'a>(filepath: Option<&'a str>, prepared: &'a str) -> StagePaths<'a> {
    StagePaths {
        filepath,
        prepared: Some(prepared),
    }
}

/// Rendered `--print`/`--print-to-file` output for one stage.
struct StagePrint {
    lines: Vec<String>,
    files: Vec<(String, PathBuf, String)>,
}

impl StagePrint {
    /// Print lines, then append file lines with upstream's `[info] Writing`.
    fn emit(&self) -> io::Result<()> {
        let mut stdout = out_stream();
        for line in &self.lines {
            writeln!(stdout, "{line}")?;
            // One `write_string` call per message.
            stdout.flush()?;
        }
        stdout.flush()?;
        for (template, path, line) in &self.files {
            screen!(
                "[info] Writing {} to: {}",
                yt_dlp_rs::batch::py_repr(template),
                path.display()
            );
            append_line(path, line)?;
        }
        Ok(())
    }
}

/// Render a stage's templates (all lines before any output, so a failure
/// prints nothing partial).
fn render_stage(
    stage: Stage,
    templates: &[PrintTemplate],
    files: &[PrintToFile],
    info: &yt_dlp_rs::VideoInfo,
    filename: Result<&str, &yt_dlp_rs::Error>,
    paths: StagePaths<'_>,
    options: &TemplateOptions,
) -> yt_dlp_rs::Result<StagePrint> {
    let lines = templates
        .iter()
        .filter(|template| template.stage() == stage && template.applies(info))
        .map(|template| template.render_at(info, filename, paths, options))
        .collect::<yt_dlp_rs::Result<_>>()?;
    let files = files
        .iter()
        .filter(|pair| pair.stage() == stage)
        .map(|pair| {
            let (path, line) = pair.render_at(info, filename, paths, options)?;
            Ok((pair.template().to_owned(), path, line))
        })
        .collect::<yt_dlp_rs::Result<_>>()?;
    Ok(StagePrint { lines, files })
}

/// Python `os.path.abspath`: join with the working directory and normalize
/// `.`/`..` components lexically.
fn py_abspath(path: &std::path::Path) -> PathBuf {
    use std::path::Component;
    let joined = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    let mut normal = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !matches!(
                    normal.components().next_back(),
                    Some(Component::RootDir) | None
                ) {
                    normal.pop();
                }
            }
            other => normal.push(other),
        }
    }
    normal
}

/// Upstream `ExecPP.run` for one stage: announce and run each command,
/// stopping at the first failure.
fn run_exec(
    exec: &ExecCommands,
    stage: Stage,
    info: &yt_dlp_rs::VideoInfo,
    filepath: Option<&std::path::Path>,
    filename: Option<&std::path::Path>,
    options: &TemplateOptions,
) -> Result<(), String> {
    let text =
        |path: Option<&std::path::Path>| path.map(|path| path.to_string_lossy().into_owned());
    let (filepath, filename) = (text(filepath), text(filename));
    // One `ExecPP` runs the stage's commands.
    if exec.stage(stage).next().is_none() {
        return Ok(());
    }
    postprocess_progress("Exec", "started");
    for command in exec.stage(stage) {
        let command = command
            .render(info, filepath.as_deref(), filename.as_deref(), options)
            .map_err(|error| error.to_string())?;
        screen!("[Exec] Executing command: {command}");
        ExecCommand::run(&command)?;
    }
    postprocess_progress("Exec", "finished");
    Ok(())
}

/// Upstream `__files_to_move` with `MoveFilesAfterDownloadPP`: sidecars
/// written beside the (temporary) media move to their final names, other
/// retained files and the media to the final directory.
struct MoveFiles {
    /// `__finaldir`: the directory of the absolute final media filename.
    finaldir: PathBuf,
    /// Source and final name (`None`: the final directory), in insertion order.
    files: Vec<(PathBuf, Option<PathBuf>)>,
    overwrites: bool,
}

impl MoveFiles {
    fn new(final_media: &std::path::Path, overwrites: Option<bool>) -> Self {
        let finaldir = py_abspath(final_media)
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        Self {
            finaldir,
            files: Vec::new(),
            overwrites: overwrites != Some(false),
        }
    }

    /// Python `dict.__setitem__`: an existing key keeps its position.
    fn insert(&mut self, source: PathBuf, target: Option<PathBuf>) {
        match self.files.iter_mut().find(|(old, _)| *old == source) {
            Some(entry) => entry.1 = target,
            None => self.files.push((source, target)),
        }
    }

    /// Python `dict.setdefault(source, '')`.
    fn keep(&mut self, source: PathBuf) {
        if !self.files.iter().any(|(old, _)| *old == source) {
            self.files.push((source, None));
        }
    }

    /// A processor derived `to` from `from` (upstream maps it to `from`'s
    /// final name with `to`'s extension); `rename` drops `from`'s entry.
    fn derive(&mut self, from: &std::path::Path, to: &std::path::Path, rename: bool) {
        let Some(index) = self.files.iter().position(|(old, _)| old == from) else {
            return;
        };
        let target = self.files[index].1.as_ref().map(|target| {
            let ext = to
                .extension()
                .map(|ext| ext.to_string_lossy().into_owned())
                .unwrap_or_default();
            PathBuf::from(yt_dlp_rs::output::replace_extension(
                &target.to_string_lossy(),
                &ext,
                None,
            ))
        });
        if rename {
            self.files.remove(index);
        }
        self.insert(to.to_owned(), target);
    }

    fn final_path(&self, filepath: &std::path::Path) -> PathBuf {
        self.finaldir
            .join(filepath.file_name().unwrap_or(filepath.as_os_str()))
    }

    /// Run the processor; `filepath` is moved when `downloaded`. Returns the
    /// final media path. Files deleted by earlier processors (which upstream
    /// removes from the map) are skipped.
    fn run(&self, filepath: &std::path::Path, downloaded: bool) -> Result<PathBuf, String> {
        postprocess_progress("MoveFiles", "started");
        let moved = self.move_files(filepath, downloaded)?;
        postprocess_progress("MoveFiles", "finished");
        Ok(moved)
    }

    fn move_files(&self, filepath: &std::path::Path, downloaded: bool) -> Result<PathBuf, String> {
        let final_path = self.final_path(filepath);
        let mut files = self.files.clone();
        if downloaded {
            match files.iter_mut().find(|(old, _)| old == filepath) {
                Some(entry) => entry.1 = Some(final_path.clone()),
                None => files.push((filepath.to_owned(), Some(final_path.clone()))),
            }
        }
        for (old, new) in files {
            if std::fs::symlink_metadata(&old).is_err() {
                continue;
            }
            let new = new.unwrap_or_else(|| self.final_path(&old));
            if py_abspath(&old) == py_abspath(&new) {
                continue;
            }
            if std::fs::metadata(&new).is_ok() {
                if self.overwrites {
                    warning!("WARNING: Replacing existing file \"{}\"", new.display());
                    std::fs::remove_file(&new).map_err(|error| error.to_string())?;
                } else {
                    warning!(
                        "WARNING: Cannot move file \"{}\" out of temporary directory since \"{}\" already exists. ",
                        old.display(),
                        new.display()
                    );
                    continue;
                }
            }
            if let Some(parent) = new.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("Unable to create directory: {error}"))?;
            }
            screen!(
                "[MoveFiles] Moving file \"{}\" to \"{}\"",
                old.display(),
                new.display()
            );
            // `shutil.move`: rename, or copy and delete across filesystems.
            if std::fs::rename(&old, &new).is_err() {
                std::fs::copy(&old, &new)
                    .and_then(|_| std::fs::remove_file(&old))
                    .map_err(|error| error.to_string())?;
            }
        }
        Ok(final_path)
    }
}

/// After a download or an existing file: report a pending preprocessing
/// error, then run `post_process` commands, `MoveFiles` and `after_move`
/// commands with `run_pp` error handling. Returns whether the video may be
/// archived.
#[allow(clippy::too_many_arguments)]
fn after_download(
    exec: &ExecCommands,
    info: &yt_dlp_rs::VideoInfo,
    filepath: &std::path::Path,
    filename: &std::path::Path,
    move_files: &MoveFiles,
    options: &TemplateOptions,
    mode: ErrorMode,
    pending: &std::cell::Cell<Option<String>>,
    failed: &std::cell::Cell<bool>,
    print_after_move: &dyn Fn(&std::path::Path, &yt_dlp_rs::VideoInfo) -> Result<(), String>,
    metadata_actions: &yt_dlp_rs::metadata_parser::MetadataActions,
) -> bool {
    if let Some(error) = pending.take() {
        report_error!("ERROR: {error}");
        failed.set(true);
        if mode == ErrorMode::Abort {
            return false;
        }
    }
    // Upstream `run_pp` error handling.
    let handle = |result: Result<(), String>| {
        if let Err(error) = result {
            failed.set(true);
            if mode == ErrorMode::Ignore {
                report_error!("ERROR: {error}");
            } else {
                report_error!("ERROR: Postprocessing: {error}");
                return false;
            }
        }
        true
    };
    if !handle(run_exec(
        exec,
        Stage::PostProcess,
        info,
        Some(filepath),
        Some(filename),
        options,
    )) {
        return false;
    }
    let mut moved = move_files.final_path(filepath);
    if !handle(move_files.run(filepath, true).map(|path| moved = path)) {
        return false;
    }
    // `run_all_pps('after_move')`: the prints, then `MetadataParserPP`,
    // then `ExecPP`.
    let after_move = print_after_move(&moved, info).and_then(|()| {
        let mut info = info.clone();
        if metadata_actions.has("after_move") {
            set_pp_info(&info);
            metadata_actions
                .run_reported("after_move", &mut info, options)
                .map_err(|error| error.to_string())?;
        }
        run_exec(
            exec,
            Stage::AfterMove,
            &info,
            Some(&moved),
            Some(filename),
            options,
        )
    });
    if !handle(after_move) {
        return false;
    }
    true
}

/// `--console-title` and the `--progress-template` title and postprocessor
/// templates (`download-title`, `postprocess`, `postprocess-title`).
#[derive(Debug, Default)]
struct ConsoleOptions {
    title: bool,
    download_title: Option<String>,
    postprocess: Option<String>,
    postprocess_title: Option<String>,
}

static CONSOLE: std::sync::OnceLock<ConsoleOptions> = std::sync::OnceLock::new();

/// The info dict of the video being processed, for postprocessor templates.
static PP_INFO: std::sync::Mutex<Option<serde_json::Value>> = std::sync::Mutex::new(None);

/// The default console title template.
const DEFAULT_TITLE: &str = "yt-dlp %(progress._default_template)s";

/// `_ProgressState` values.
const PROGRESS_HIDDEN: u8 = 0;
const PROGRESS_VISIBLE: u8 = 1;
const PROGRESS_INDETERMINATE: u8 = 3;

/// `--console-title`.
fn console_title() -> bool {
    CONSOLE.get().is_some_and(|console| console.title)
}

/// Whether postprocessor templates need the current info dict.
fn wants_pp_info() -> bool {
    CONSOLE
        .get()
        .is_some_and(|console| console.postprocess.is_some() || console.title)
}

/// Remember `info` for postprocessor progress templates.
fn set_pp_info(info: &yt_dlp_rs::VideoInfo) {
    if wants_pp_info() {
        *PP_INFO.lock().unwrap() = yt_dlp_rs::info_value(info).ok();
    }
}

/// Windows console calls: upstream `windows_enable_vt_mode` and the
/// `SetConsoleTitleW` fallback.
#[cfg(windows)]
mod windows_console {
    use std::os::windows::io::AsRawHandle;

    type Handle = *mut std::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetConsoleMode(handle: Handle, mode: *mut u32) -> i32;
        fn SetConsoleMode(handle: Handle, mode: u32) -> i32;
        fn GetConsoleWindow() -> Handle;
        fn SetConsoleTitleW(title: *const u16) -> i32;
    }

    /// `ENABLE_VIRTUAL_TERMINAL_PROCESSING` on `CONOUT$` (fails before
    /// Windows 10 1511, like upstream's version check).
    pub fn vt_mode() -> bool {
        static VT_MODE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *VT_MODE.get_or_init(|| {
            let Ok(console) = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("CONOUT$")
            else {
                return false;
            };
            let handle = console.as_raw_handle() as Handle;
            let mut mode = 0u32;
            // SAFETY: `handle` is the open console buffer and `mode` a valid
            // out-pointer for the duration of the calls.
            unsafe {
                GetConsoleMode(handle, &mut mode) != 0 && SetConsoleMode(handle, mode | 0x0004) != 0
            }
        })
    }

    /// `ctypes.windll.kernel32.SetConsoleTitleW(message)` when a console
    /// window exists.
    pub fn set_title(title: &str) {
        // SAFETY: no arguments; returns null without a console window.
        if unsafe { GetConsoleWindow() }.is_null() {
            return;
        }
        let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is a NUL-terminated UTF-16 string alive for the call.
        unsafe {
            SetConsoleTitleW(wide.as_ptr());
        }
    }
}

/// Upstream `supports_terminal_sequences` before the stream check: VT mode
/// on Windows, a nonempty `TERM` elsewhere.
fn terminal_sequences() -> bool {
    #[cfg(windows)]
    {
        windows_console::vt_mode()
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("TERM").is_some_and(|term| !term.is_empty())
    }
}

/// Upstream `_send_console_code`: written to `_out_files.console`, the
/// first of stderr and stdout that is a terminal supporting sequences.
fn send_console_code(code: &str) -> bool {
    use std::io::IsTerminal;
    if !terminal_sequences() {
        return false;
    }
    if io::stderr().is_terminal() {
        let mut stderr = io::stderr().lock();
        let _ = stderr.write_all(code.as_bytes());
        let _ = stderr.flush();
    } else if io::stdout().is_terminal() {
        let mut stdout = io::stdout().lock();
        let _ = stdout.write_all(code.as_bytes());
        let _ = stdout.flush();
    } else {
        return false;
    }
    true
}

/// Upstream `remove_terminal_sequences` (`\033[...m`).
fn remove_terminal_sequences(text: &str) -> String {
    static SEQUENCE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new("\x1b\\[[^m]+m").unwrap());
    SEQUENCE.replace_all(text, "").into_owned()
}

/// Upstream `to_console_title(message, progress_state, percent)`.
fn to_console_title(message: Option<&str>, state: Option<u8>, percent: Option<f64>) {
    if !CONSOLE.get().is_some_and(|console| console.title) {
        return;
    }
    if let Some(message) = message.filter(|message| !message.is_empty()) {
        let sent = send_console_code(&format!(
            "\x1b]0;{}\x07",
            remove_terminal_sequences(message)
        ));
        #[cfg(windows)]
        if !sent {
            windows_console::set_title(message);
        }
        #[cfg(not(windows))]
        let _ = sent;
    }
    if let Some(state) = state {
        let percent = percent.map_or(0, |percent| percent as i64);
        send_console_code(&format!("\x1b]9;4;{state};{percent}\x07"));
    }
}

/// Upstream `with YoutubeDL(...)`: `save_console_title` (unless
/// simulating) and an indeterminate progress state; dropping restores the
/// title and hides the progress.
struct ConsoleTitle {
    simulate: bool,
}

impl ConsoleTitle {
    fn enter(simulate: bool) -> Self {
        if CONSOLE.get().is_some_and(|console| console.title) {
            if !simulate {
                send_console_code("\x1b[22;0t");
            }
            to_console_title(None, Some(PROGRESS_INDETERMINATE), None);
        }
        Self { simulate }
    }
}

impl Drop for ConsoleTitle {
    fn drop(&mut self) {
        if CONSOLE.get().is_some_and(|console| console.title) {
            if !self.simulate {
                send_console_code("\x1b[23;0t");
            }
            to_console_title(None, Some(PROGRESS_HIDDEN), None);
        }
    }
}

/// Upstream `PostProcessor.report_progress`: the `postprocess` template
/// (printed even when quiet) and the `postprocess-title`.
fn postprocess_progress(postprocessor: &str, status: &str) {
    let Some(console) = CONSOLE.get() else {
        return;
    };
    if console.postprocess.is_none() && !console.title {
        return;
    }
    let info = PP_INFO
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| serde_json::json!({}));
    let dict = serde_json::json!({
        "info": info,
        "progress": {
            "status": status,
            "postprocessor": postprocessor,
            "_default_template": format!("{postprocessor} {status}"),
        },
    });
    let render = |template: &str| {
        yt_dlp_rs::output::evaluate_dict_template(template, &dict, &TemplateOptions::default())
            .unwrap_or_default()
    };
    if let Some(template) = &console.postprocess {
        let line = render(template);
        if QUIET.load(Ordering::Relaxed) || LOG_TO_STDERR.load(Ordering::Relaxed) {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
    let title = render(
        console
            .postprocess_title
            .as_deref()
            .unwrap_or(DEFAULT_TITLE),
    );
    to_console_title(Some(&title), Some(PROGRESS_INDETERMINATE), None);
}

/// Upstream progress parameters: `noprogress` (quiet unless `--progress`),
/// `progress_with_newline` and `progress_delta`.
struct ProgressOptions {
    enabled: bool,
    newline: bool,
    quiet: bool,
    delta: Option<Duration>,
    /// `--progress-template download:TEMPLATE`.
    template: Option<String>,
}

/// `FileDownloader.report_progress` through one status printer per
/// downloaded stream, written to upstream's `_out_files.out` (stdout, or
/// stderr with `logtostderr`).
struct ProgressReporter<'a> {
    options: &'a ProgressOptions,
    active: Option<ActiveDownload>,
    /// `info` and `progress.filename` for `--progress-template`.
    info: serde_json::Value,
    filename: Option<String>,
}

struct ActiveDownload {
    stream: Option<u8>,
    start: Instant,
    downloaded: u64,
    printer: yt_dlp_rs::output::progress::StatusPrinter,
    next: Instant,
    /// FragmentFD's progress state for fragmented downloads.
    fragments: Option<FragmentState>,
}

/// FragmentFD `frag_progress_hook` state: completed fragments, the running
/// `complete_frags_downloaded_bytes` and the `ProgressCalculator`.
struct FragmentState {
    index: u32,
    complete: u64,
    calculator: yt_dlp_rs::output::progress::ProgressCalculator,
}

impl<'a> ProgressReporter<'a> {
    fn new(options: &'a ProgressOptions) -> Self {
        Self {
            options,
            active: None,
            info: serde_json::Value::Object(Default::default()),
            filename: None,
        }
    }

    /// The video and file a `--progress-template` describes.
    fn for_download(mut self, info: &yt_dlp_rs::VideoInfo, filename: &std::path::Path) -> Self {
        if self.options.template.is_some() || console_title() {
            self.info = yt_dlp_rs::info_value(info).unwrap_or_default();
            self.filename = Some(filename.to_string_lossy().into_owned());
        }
        self
    }

    /// Upstream `_report_progress_status`: the template over
    /// `{'info': ..., 'progress': ...}`, else `[download] DEFAULT`.
    fn render(&self, progress: serde_json::Value) -> String {
        match &self.options.template {
            Some(template) => {
                let dict = serde_json::json!({"info": self.info, "progress": progress});
                yt_dlp_rs::output::evaluate_dict_template(
                    template,
                    &dict,
                    &TemplateOptions::default(),
                )
                .unwrap_or_default()
            }
            None => format!(
                "[download] {}",
                progress["_default_template"].as_str().unwrap_or_default()
            ),
        }
    }

    /// `_report_progress_status`'s `to_console_title` (the `download-title`
    /// template), visible with a percentage while downloading.
    fn title(&self, progress: serde_json::Value) {
        let Some(console) = CONSOLE.get().filter(|console| console.title) else {
            return;
        };
        let finished = progress["status"] == "finished";
        let percent = progress["_percent"].as_f64();
        let dict = serde_json::json!({"info": self.info, "progress": progress});
        let message = yt_dlp_rs::output::evaluate_dict_template(
            console.download_title.as_deref().unwrap_or(DEFAULT_TITLE),
            &dict,
            &TemplateOptions::default(),
        )
        .unwrap_or_default();
        let state = if finished || percent.is_none() {
            PROGRESS_INDETERMINATE
        } else {
            PROGRESS_VISIBLE
        };
        to_console_title(Some(&message), Some(state), percent);
    }

    fn write(text: &str) {
        if LOG_TO_STDERR.load(Ordering::Relaxed) {
            let mut stderr = io::stderr().lock();
            let _ = stderr.write_all(text.as_bytes());
            let _ = stderr.flush();
        } else {
            let mut stdout = io::stdout().lock();
            let _ = stdout.write_all(text.as_bytes());
            let _ = stdout.flush();
        }
    }

    fn event(&mut self, progress: &yt_dlp_rs::DownloadProgress) {
        use yt_dlp_rs::output::progress::ProgressStatus;
        match progress.phase {
            DownloadPhase::Downloading => {}
            // A reused merge input: upstream reports it finished without timing.
            DownloadPhase::Publishing
                if self.active.is_none() && progress.stream_index.is_some() =>
            {
                return self.already_downloaded(progress.downloaded_bytes);
            }
            DownloadPhase::Merging | DownloadPhase::Postprocessing | DownloadPhase::Publishing => {
                return self.finish();
            }
            // `_finish_multiline_status` after a `False` real_download.
            DownloadPhase::Failed => {
                self.active = None;
                if self.options.enabled && !self.options.newline {
                    Self::write(if self.options.quiet { "\r\r" } else { "\n" });
                }
                return;
            }
            // An external downloader's single `finished` status.
            DownloadPhase::Completed => {
                if let Some(active) = self.active.as_mut() {
                    active.downloaded = progress.downloaded_bytes;
                }
                return self.finish();
            }
            _ => return,
        }
        if self
            .active
            .as_ref()
            .is_some_and(|active| active.stream != progress.stream_index)
        {
            self.finish();
        }
        let now = Instant::now();
        let active = self.active.get_or_insert_with(|| ActiveDownload {
            stream: progress.stream_index,
            start: now,
            downloaded: 0,
            printer: yt_dlp_rs::output::progress::StatusPrinter::new(
                self.options.newline,
                !self.options.quiet,
            ),
            next: now,
            fragments: None,
        });
        active.downloaded = progress.downloaded_bytes;
        // FragmentFD reports each fragment's HttpFD progress (only).
        if progress.fragment_count.is_some() && progress.fragment.is_none() {
            return;
        }
        // Upstream's HTTP downloader reports after each received block.
        // Nothing is reported before the first data (`ProgressCalculator`
        // ignores empty updates, and HttpFD hooks after each block).
        // `noprogress` hides the lines; console titles are still updated.
        if (!self.options.enabled && !console_title()) || progress.downloaded_bytes == 0 {
            return;
        }
        if let Some(delta) = self.options.delta {
            if now < active.next {
                return;
            }
            active.next += delta;
        }
        if let (Some(fragment), Some(count)) = (progress.fragment, progress.fragment_count) {
            let state = active.fragments.get_or_insert_with(|| FragmentState {
                index: 0,
                complete: 0,
                calculator: yt_dlp_rs::output::progress::ProgressCalculator::new(0),
            });
            // Upstream's estimate counts the running total (not only the
            // completed fragments) plus this fragment's length.
            let estimate = (state.complete as f64 + fragment.total.unwrap_or(0) as f64)
                / (f64::from(state.index) + 1.0)
                * f64::from(count);
            state.calculator.set_total(Some(estimate));
            state.calculator.update(fragment.downloaded);
            if fragment.finished {
                state.index += 1;
                state.calculator.thread_reset();
            }
            state.complete = state.calculator.downloaded;
            let status = ProgressStatus {
                downloaded_bytes: state.complete,
                total_bytes: None,
                total_bytes_estimate: state.calculator.total(),
                elapsed: Some(state.calculator.elapsed),
                speed: state.calculator.speed(),
                eta: state.calculator.eta(),
                fragment_index: Some(state.index),
                fragment_count: Some(count),
            };
            return self.downloading(&status);
        }
        let elapsed = now.duration_since(active.start).as_secs_f64();
        let downloaded = progress.downloaded_bytes as f64;
        let speed =
            (progress.downloaded_bytes != 0 && elapsed >= 0.001).then(|| downloaded / elapsed);
        // Fragments: completed count and a size estimate from the average
        // fragment so far (upstream also counts the current one's length).
        let (fragment_index, estimate) = match (progress.fragment_index, progress.fragment_count) {
            (Some(index), Some(count)) if progress.total_bytes.is_none() => {
                let estimate = (progress.downloaded_bytes > 0)
                    .then(|| downloaded / f64::from(index) * f64::from(count));
                (Some(index.saturating_sub(1)), estimate)
            }
            (Some(index), _) => (Some(index.saturating_sub(1)), None),
            _ => (None, None),
        };
        let remaining = progress
            .total_bytes
            .map(|total| total as f64)
            .or(estimate)
            .map(|total| total - downloaded);
        let eta = speed
            .zip(remaining)
            .map(|(speed, remaining)| (remaining / speed).trunc());
        let status = ProgressStatus {
            downloaded_bytes: progress.downloaded_bytes,
            total_bytes: progress.total_bytes,
            total_bytes_estimate: estimate,
            elapsed: Some(elapsed),
            speed,
            eta,
            fragment_index,
            fragment_count: progress.fragment_count,
        };
        self.downloading(&status);
    }

    /// One `downloading` status: the line (unless `noprogress`) and title.
    fn downloading(&mut self, status: &yt_dlp_rs::output::progress::ProgressStatus) {
        let dict =
            || yt_dlp_rs::output::progress::downloading_dict(status, self.filename.as_deref());
        if self.options.enabled {
            let line = if self.options.template.is_some() {
                self.render(dict())
            } else {
                format!(
                    "[download] {}",
                    yt_dlp_rs::output::progress::downloading_line(status)
                )
            };
            let active = self.active.as_mut().expect("active download");
            Self::write(&active.printer.print(&line));
        }
        if console_title() {
            self.title(dict());
        }
    }

    /// `FileDownloader.download`'s `finished` status for an existing file
    /// (no timing).
    fn already_downloaded(&self, bytes: u64) {
        if console_title() {
            self.title(yt_dlp_rs::output::progress::finished_dict(
                bytes,
                None,
                self.filename.as_deref(),
            ));
        }
        if !self.options.enabled {
            screen!("[download] Download completed");
        } else {
            let line = yt_dlp_rs::output::progress::finished_line(bytes, None);
            let mut printer = yt_dlp_rs::output::progress::StatusPrinter::new(
                self.options.newline,
                !self.options.quiet,
            );
            Self::write(&printer.print(&format!("[download] {line}")));
            Self::write(&printer.end());
        }
    }

    /// The `finished` status of the current stream.
    fn finish(&mut self) {
        let Some(mut active) = self.active.take() else {
            return;
        };
        let elapsed = active.start.elapsed().as_secs_f64();
        let dict = yt_dlp_rs::output::progress::finished_dict(
            active.downloaded,
            Some(elapsed),
            self.filename.as_deref(),
        );
        if !self.options.enabled {
            screen!("[download] Download completed");
        } else {
            let line = self.render(dict.clone());
            Self::write(&active.printer.print(&line));
            Self::write(&active.printer.end());
        }
        self.title(dict);
    }
}

impl Drop for ProgressReporter<'_> {
    /// End an interrupted status line before an error is reported.
    fn drop(&mut self) {
        if let Some(mut active) = self.active.take()
            && self.options.enabled
        {
            Self::write(&active.printer.end());
        }
    }
}

/// Upstream `ignoreerrors` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorMode {
    Ignore,
    OnlyDownload,
    Abort,
}

/// Upstream `DownloadCancelled` reasons; uncaught ones exit with 101.
#[derive(Debug, Clone)]
enum Cancel {
    MaxDownloads,
    Existing,
    /// A `--break-match-filters` (or `--break-on-reject`) rejection.
    Rejected,
}

impl std::fmt::Display for Cancel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Cancel::MaxDownloads => {
                "Maximum number of downloads reached, stopping due to --max-downloads"
            }
            Cancel::Existing => {
                "Encountered a video that is already in the archive, stopping due to --break-on-existing"
            }
            Cancel::Rejected => {
                "Encountered a video that did not match filter, stopping due to --break-match-filter"
            }
        })
    }
}

impl std::error::Error for Cancel {}

/// Report a cancellation like `__download_wrapper`; with `--break-per-input`
/// it only ends the current input (returns `None`).
fn cancel(reason: Cancel, per_input: bool) -> Option<Cancel> {
    screen!("[info] {reason}");
    if per_input {
        None
    } else {
        screen!("Aborting remaining downloads");
        Some(reason)
    }
}

/// Upstream `FFmpegPostProcessor._determine_executables`: without a location
/// the programs come from `PATH`; a directory holds both, and a file's name
/// selects its program, with the other found by substituting the name.
/// Upstream `FFmpegPostProcessor._get_ffmpeg_version`: the version from
/// `PROG -bsfs` and, for ffmpeg, the enabled features.
fn ffmpeg_version(path: &Path, ffmpeg: bool) -> Option<String> {
    let output = std::process::Command::new(path)
        .arg("-bsfs")
        .stdin(std::process::Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    static VERSION: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"version\s+([-0-9._a-zA-Z]+)").unwrap());
    static DISTRIBUTION: std::sync::LazyLock<[regex::Regex; 2]> = std::sync::LazyLock::new(|| {
        [
            regex::Regex::new(r"^(?:\d+:)?([0-9.]+)-[0-9]+ubuntu[0-9.]+$").unwrap(),
            regex::Regex::new(r"^n([0-9.]+)$").unwrap(),
        ]
    });
    let mut version = VERSION
        .captures(&text)
        .map_or("present".to_owned(), |found| found[1].to_owned());
    for regex in DISTRIBUTION.iter() {
        if let Some(found) = regex.captures(&version) {
            version = found[1].to_owned();
        }
    }
    if !ffmpeg {
        return Some(version);
    }
    static LAVF: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?m)^\s+libavformat\s+(?:[0-9. ]+)\s+/\s+(?P<runtime>[0-9. ]+)")
            .unwrap()
    });
    // `is_outdated_version(runtime, '57.56.100', assume_new=False)`.
    let outdated = LAVF
        .captures(&text)
        .map(|found| found["runtime"].replace(' ', ""))
        .and_then(|runtime| {
            let parse = |text: &str| {
                text.split(['-', '.'])
                    .map(|part| part.parse::<u64>().ok())
                    .collect::<Option<Vec<_>>>()
            };
            Some(parse(&runtime)? < parse("57.56.100")?)
        })
        .unwrap_or(true);
    let mut features = Vec::new();
    if text.contains("--enable-libfdk-aac") {
        features.push("fdk");
    }
    if outdated {
        features.push("needs_adtstoasc");
    }
    if text.lines().any(|line| line == "setts") {
        features.push("setts");
    }
    if !features.is_empty() {
        version = format!("{version} ({})", features.join(","));
    }
    Some(version)
}

/// Upstream `YoutubeDL.proxies` as a Python dict: `--proxy`, or the
/// `*_proxy` environment variables (`urllib.request.getproxies`).
fn proxy_map(proxy: Option<&str>) -> String {
    let mut proxies: Vec<(String, String)> = Vec::new();
    let set = |proxies: &mut Vec<(String, String)>, key: String, value: Option<String>| {
        let position = proxies.iter().position(|(name, _)| *name == key);
        match (position, value) {
            (Some(index), Some(value)) => proxies[index].1 = value,
            (None, Some(value)) => proxies.push((key, value)),
            (Some(index), None) => {
                proxies.remove(index);
            }
            (None, None) => {}
        }
    };
    if let Some(proxy) = proxy {
        let proxy = if proxy.is_empty() {
            "__noproxy__"
        } else {
            proxy
        };
        set(&mut proxies, "all".into(), Some(proxy.into()));
    } else {
        let environment: Vec<(String, String)> = std::env::vars_os()
            .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
            .collect();
        for (name, value) in &environment {
            let name = name.to_lowercase();
            if !value.is_empty() && name.ends_with("_proxy") {
                set(
                    &mut proxies,
                    name[..name.len() - 6].into(),
                    Some(value.clone()),
                );
            }
        }
        if std::env::var_os("REQUEST_METHOD").is_some() {
            set(&mut proxies, "http".into(), None);
        }
        for (name, value) in &environment {
            if name.ends_with("_proxy") {
                let name = name.to_lowercase();
                let value = (!value.is_empty()).then(|| value.clone());
                set(&mut proxies, name[..name.len() - 6].into(), value);
            }
        }
        if let Some((_, http)) = proxies.iter().find(|(name, _)| name == "http").cloned()
            && !proxies.iter().any(|(name, _)| name == "https")
        {
            proxies.push(("https".into(), http));
        }
    }
    let items: Vec<String> = proxies
        .iter()
        .map(|(name, value)| {
            format!(
                "{}: {}",
                yt_dlp_rs::batch::py_repr(name),
                yt_dlp_rs::batch::py_repr(value)
            )
        })
        .collect();
    format!("{{{}}}", items.join(", "))
}

/// Upstream `print_debug_header`, for the parts that describe this port.
fn print_debug_header(
    quiet: bool,
    ffmpeg: Option<&Path>,
    ffprobe: Option<&Path>,
    proxy: Option<&str>,
    compat: &[String],
    args_encoding: Option<&str>,
) {
    use std::io::IsTerminal;
    let term = std::env::var("TERM").unwrap_or_default();
    let encoding = |tty: bool| {
        let mut info = Vec::new();
        if term.eq_ignore_ascii_case("dumb") {
            info.push("dumb");
        }
        if term.is_empty() || !tty {
            info.push("No ANSI");
        }
        if info.is_empty() {
            "utf-8".to_owned()
        } else {
            format!("utf-8 ({})", info.join(","))
        }
    };
    let (stdout, stderr) = (io::stdout().is_terminal(), io::stderr().is_terminal());
    let out = if LOG_TO_STDERR.load(Ordering::Relaxed) {
        stderr
    } else {
        stdout
    };
    let screen = if quiet { stderr } else { out };
    let debug = |text: &str| eprintln!("[debug] {text}");
    // Written without `--encoding`; `pref` is its name.
    std::eprintln!(
        "[debug] Encodings: locale utf-8, fs utf-8, pref {}, out {}, error {}, screen {}",
        args_encoding.unwrap_or("utf-8"),
        encoding(out),
        encoding(stderr),
        encoding(screen)
    );
    debug(&format!("yt-dlp-rs version {}", env!("CARGO_PKG_VERSION")));
    if !compat.is_empty() {
        debug(&format!("Compatibility options: {}", compat.join(", ")));
    }
    debug(&format!(
        "Rust native build ({}) - {}",
        std::env::consts::ARCH,
        std::env::consts::OS
    ));
    let mut versions = Vec::new();
    if let Some(version) = ffmpeg.and_then(|path| ffmpeg_version(path, true)) {
        versions.push(format!("ffmpeg {version}"));
    }
    if let Some(version) = ffprobe.and_then(|path| ffmpeg_version(path, false)) {
        versions.push(format!("ffprobe {version}"));
    }
    debug(&format!(
        "exe versions: {}",
        if versions.is_empty() {
            "none".to_owned()
        } else {
            versions.join(", ")
        }
    ));
    debug(&format!("Proxy map: {}", proxy_map(proxy)));
}

fn ffmpeg_executables(location: Option<&Path>) -> (Option<PathBuf>, Option<PathBuf>) {
    const PROGRAMS: [&str; 2] = ["ffmpeg", "ffprobe"];
    let Some(location) = location else {
        let [ffmpeg, ffprobe] = PROGRAMS.map(find_in_path);
        return (ffmpeg, ffprobe);
    };
    if !location.exists() {
        warning!(
            "WARNING: ffmpeg-location {} does not exist! Continuing without ffmpeg",
            location.display()
        );
        return (None, None);
    }
    let (directory, named) = if location.is_dir() {
        (location.to_path_buf(), None)
    } else {
        let filename = location
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let basename = PROGRAMS
            .into_iter()
            .find(|program| filename.contains(program))
            .unwrap_or("ffmpeg");
        let absolute = std::path::absolute(location).unwrap_or_else(|_| location.into());
        let directory = absolute.parent().map(Path::to_path_buf).unwrap_or_default();
        (directory, Some((basename, filename)))
    };
    let [ffmpeg, ffprobe] = PROGRAMS.map(|program| {
        let Some((basename, filename)) = &named else {
            return directory.join(program);
        };
        if *basename == program {
            return location.to_path_buf();
        }
        if filename.contains(basename) {
            let sibling = directory.join(filename.replace(basename, program));
            if sibling.exists() {
                return sibling;
            }
        }
        directory.join(program)
    });
    (Some(ffmpeg), Some(ffprobe))
}

/// The first executable `PATH` entry for `program` (with `.exe` on Windows).
fn find_in_path(program: &str) -> Option<PathBuf> {
    let name = format!("{program}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|directory| directory.join(&name))
        .find(|candidate| is_executable(candidate))
}

fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

fn epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// Upstream `determine_ext(url, 'jpg')`.
fn determine_ext(url: &str) -> String {
    const KNOWN: &[&str] = &[
        "3g2", "3gp", "f4v", "mk3d", "divx", "mpg", "ogv", "m4v", "wmv", "avi", "flv", "mkv",
        "mov", "mp4", "webm", "aac", "ape", "asf", "f4a", "f4b", "m4b", "m4r", "oga", "ogx", "spx",
        "vorbis", "wma", "weba", "aiff", "alac", "flac", "m4a", "mka", "mp3", "ogg", "opus", "wav",
        "f4f", "f4m", "m3u8", "smil", "mpd",
    ];
    if !url.contains('.') {
        return "jpg".into();
    }
    let path = url.split('?').next().unwrap_or("");
    let guess = path.rsplit_once('.').map_or(path, |(_, guess)| guess);
    if !guess.is_empty() && guess.bytes().all(|b| b.is_ascii_alphanumeric()) {
        guess.into()
    } else if KNOWN.contains(&guess.trim_end_matches('/')) {
        guess.trim_end_matches('/').into()
    } else {
        "jpg".into()
    }
}

/// Upstream `_write_thumbnails` for a video: try thumbnails from best to worst
/// until one is written (or all of them with `write_all`).
#[allow(clippy::too_many_arguments)]
fn write_thumbnails(
    downloader: &YoutubeDl,
    info: &yt_dlp_rs::VideoInfo,
    filename: &std::path::Path,
    move_files: &mut MoveFiles,
    base: Option<PathBuf>,
    write_all: bool,
    overwrites: Option<bool>,
    retries: u32,
) -> Result<Vec<(usize, PathBuf)>, Box<dyn std::error::Error>> {
    let thumbnails = &info.metadata.thumbnails;
    let mut on_disk = Vec::new();
    if thumbnails.is_empty() {
        screen!("[info] There are no video thumbnails to download");
        return Ok(on_disk);
    }
    let Some(base) = base else {
        return Ok(on_disk);
    };
    let multiple = write_all && thumbnails.len() > 1;
    let base = base.to_string_lossy();
    let filename = filename.to_string_lossy();
    let mut written = false;
    for (index, thumbnail) in thumbnails.iter().enumerate().rev() {
        let id = thumbnail.id.as_deref().unwrap_or("None");
        let mut ext = determine_ext(&thumbnail.url);
        if multiple {
            ext = format!("{id}.{ext}");
        }
        let display = format!("video thumbnail {id}");
        // Written beside the (temporary) media name, moved to the template's.
        let path = PathBuf::from(yt_dlp_rs::output::replace_extension(
            &filename,
            &ext,
            Some(&info.ext),
        ));
        let final_path = PathBuf::from(yt_dlp_rs::output::replace_extension(
            &base,
            &ext,
            Some(&info.ext),
        ));
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        // Upstream `existing_file((final, written))`.
        let mut existing = Vec::new();
        for candidate in [&final_path, &path] {
            if !existing.contains(candidate) && path_exists(candidate)? {
                existing.push(candidate.clone());
            }
        }
        if !existing.is_empty() {
            if overwrites == Some(false) {
                let label = if multiple {
                    display.clone()
                } else {
                    "video thumbnail".into()
                };
                let mut chars = label.chars();
                let capitalized: String = chars
                    .next()
                    .map(|c| {
                        c.to_uppercase()
                            .chain(chars.flat_map(char::to_lowercase))
                            .collect()
                    })
                    .unwrap_or_default();
                screen!("[info] {capitalized} is already present");
                move_files.insert(existing[0].clone(), Some(final_path));
                on_disk.push((index, existing.swap_remove(0)));
                written = true;
                if !write_all {
                    break;
                }
                continue;
            }
            for existing in existing {
                screen!("Deleting existing file {}", existing.display());
                std::fs::remove_file(&existing)?;
            }
        }
        screen!("[info] Downloading {display} ...");
        let options = DownloadOptions {
            retries,
            ..Default::default()
        };
        match downloader.download_thumbnail(&thumbnail.url, &path, options) {
            Ok(_) => {
                screen!("[info] Writing {display} to: {}", path.display());
                move_files.insert(path.clone(), Some(final_path));
                on_disk.push((index, path));
                written = true;
            }
            Err(yt_dlp_rs::Error::Http(error))
                if error.status() == Some(reqwest::StatusCode::NOT_FOUND) =>
            {
                let title: Vec<String> = display
                    .split(' ')
                    .map(|word| {
                        let mut chars = word.chars();
                        chars
                            .next()
                            .map(|c| c.to_uppercase().chain(chars).collect())
                            .unwrap_or_default()
                    })
                    .collect();
                screen!("[info] {} does not exist", title.join(" "));
            }
            Err(error) => warning!("WARNING: Unable to download {display}: {error}"),
        }
        if written && !write_all {
            break;
        }
    }
    on_disk.sort();
    Ok(on_disk)
}

/// Upstream `FFmpegSubtitlesConvertorPP.run` over the written subtitles.
/// Originals (and TTML's SRT intermediate) are deleted unless -k.
fn convert_subtitles(
    downloader: &YoutubeDl,
    info: &yt_dlp_rs::VideoInfo,
    files: &mut [SubtitleFile],
    conversion: &SubtitleConversion,
    keep: bool,
    move_files: &mut MoveFiles,
) -> Result<(), String> {
    let Some(requested) = &info.requested_subtitles else {
        screen!("[SubtitlesConvertor] There aren't any subtitles to convert");
        postprocess_progress("SubtitlesConvertor", "finished");
        return Ok(());
    };
    let target = conversion.target().unwrap_or_default();
    screen!("[SubtitlesConvertor] Converting subtitles");
    let mut originals = Vec::new();
    for (language, _) in &requested.entries {
        let language = language.as_deref().unwrap_or("None");
        let Some(file) = files
            .iter_mut()
            .find(|file| file.language == language && file.path.exists())
        else {
            warning!("WARNING: Skipping embedding {language} subtitle because the file is missing");
            continue;
        };
        if file.ext == target {
            screen!(
                "[SubtitlesConvertor] Subtitle file for {target} is already in the requested format"
            );
            continue;
        } else if file.ext == "json" {
            screen!(
                "[SubtitlesConvertor] You have requested to convert json subtitles into another format, which is currently not possible"
            );
            continue;
        }
        let ttml = matches!(file.ext.as_str(), "dfxp" | "ttml" | "tt");
        if ttml {
            warning!(
                "WARNING: You have requested to convert dfxp (TTML) subtitles into another format, which results in style information loss"
            );
        }
        let converted = downloader
            .convert_subtitle_with_options(&file.path, &file.ext, conversion)
            .map_err(|error| match error {
                yt_dlp_rs::Error::Postprocess(message) => message,
                other => other.to_string(),
            })?;
        originals.push(file.path.clone());
        originals.extend(converted.intermediates);
        // Upstream maps the result to the original's final name, except a
        // TTML source converted to SRT, which it leaves unmapped.
        if !(ttml && target == "srt") {
            move_files.derive(&file.path, &converted.path, false);
        }
        // The new entry drops the subtitle's original name.
        file.path = converted.path;
        file.ext = converted.ext;
        file.name = None;
    }
    // `run_pp` deletes (or with -k keeps) the processor's originals after
    // it returns.
    postprocess_progress("SubtitlesConvertor", "finished");
    for original in originals {
        if keep {
            move_files.keep(original);
        } else {
            screen!(
                "Deleting original file {} (pass -k to keep)",
                original.display()
            );
            if std::fs::remove_file(&original).is_err() {
                warning!("WARNING: Unable to delete file {}", original.display());
            }
        }
    }
    Ok(())
}

/// Upstream `FFmpegThumbnailsConvertorPP.run` over the written thumbnails.
fn convert_thumbnails(
    downloader: &YoutubeDl,
    thumbnails: &mut [(usize, PathBuf)],
    move_files: &mut MoveFiles,
    conversion: &ThumbnailConversion,
    keep_original: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    if thumbnails.is_empty() {
        screen!("[ThumbnailsConvertor] There aren't any thumbnails to convert");
    }
    let mut originals = Vec::new();
    for (_, path) in thumbnails.iter_mut() {
        let original = yt_dlp_rs::fixup_webp(path)?;
        if original != *path {
            screen!(
                "[ThumbnailsConvertor] Correcting thumbnail \"{}\" extension to webp",
                path.display()
            );
            move_files.derive(path, &original, true);
        }
        let mut ext = original
            .extension()
            .map(|ext| ext.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if ext == "jpeg" {
            ext = "jpg".into();
        }
        match conversion.resolve(&ext) {
            Err(reason) => {
                screen!(
                    "[ThumbnailsConvertor] Not converting thumbnail \"{}\"; {reason}",
                    original.display()
                );
                *path = original;
            }
            Ok(target) => {
                let converted = PathBuf::from(yt_dlp_rs::output::replace_extension(
                    &original.to_string_lossy(),
                    &target,
                    None,
                ));
                screen!(
                    "[ThumbnailsConvertor] Converting thumbnail \"{}\" to {target}",
                    original.display()
                );
                downloader.convert_thumbnail(&original, &converted, &target)?;
                move_files.derive(&original, &converted, false);
                originals.push(original);
                *path = converted;
            }
        }
    }
    // `run_pp` deletes the originals after the processor returns.
    postprocess_progress("ThumbnailsConvertor", "finished");
    if !keep_original {
        for original in originals {
            screen!(
                "Deleting original file {} (pass -k to keep)",
                original.display()
            );
            if std::fs::remove_file(&original).is_err() {
                warning!("WARNING: Unable to delete file {}", original.display());
            }
        }
    }
    Ok(())
}

/// Upstream `_write_description` for a video.
fn write_description(
    path: &std::path::Path,
    info: &yt_dlp_rs::VideoInfo,
    overwrites: Option<bool>,
) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    if overwrites == Some(false) && path.exists() {
        screen!("[info] Video description is already present");
    } else if let Some(description) = &info.metadata.description {
        screen!("[info] Writing video description to: {}", path.display());
        std::fs::write(path, description)?;
    } else {
        screen!("[info] There's no video description to write");
    }
    Ok(())
}

/// `process_info`'s keys on a `-J` result's latest `requested_downloads`
/// dict: its postprocessors (object reprs without their addresses), whether
/// it was downloaded, the merged `.fID` files, `__finaldir` and `filepath`.
fn annotate_download(
    target: Option<&mut serde_json::Value>,
    info: &yt_dlp_rs::VideoInfo,
    path: &std::path::Path,
    real: bool,
    final_path: &std::path::Path,
) {
    let Some(download) = target
        .and_then(|target| target.get_mut("requested_downloads"))
        .and_then(serde_json::Value::as_array_mut)
        .and_then(|downloads| downloads.last_mut())
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    let merging = real && !info.requested_formats.is_empty();
    let processors: Vec<serde_json::Value> = merging
        .then(|| "<yt_dlp.postprocessor.ffmpeg.FFmpegMergerPP object>".into())
        .into_iter()
        .collect();
    download.insert("__postprocessors".into(), processors.into());
    download.insert("__real_download".into(), real.into());
    if merging {
        let files: Vec<serde_json::Value> = info
            .requested_formats
            .iter()
            .map(|format| {
                yt_dlp_rs::output::replace_extension(
                    &path.to_string_lossy(),
                    &format!("f{}.{}", format.format_id, format.ext),
                    Some(&info.ext),
                )
                .into()
            })
            .collect();
        download.insert("__files_to_merge".into(), files.into());
    }
    let final_file = py_abspath(final_path);
    download.insert(
        "__finaldir".into(),
        final_file
            .parent()
            .map(|dir| dir.to_string_lossy().into_owned())
            .unwrap_or_default()
            .into(),
    );
    download.insert(
        "filepath".into(),
        final_file.to_string_lossy().into_owned().into(),
    );
}

fn report_info_json(label: &str, path: &std::path::Path, written: InfoJsonWrite) {
    match written {
        InfoJsonWrite::Written => {
            screen!(
                "[info] Writing {label} metadata as JSON to: {}",
                path.display()
            )
        }
        InfoJsonWrite::AlreadyPresent => {
            let mut title = label.to_owned();
            title[..1].make_ascii_uppercase();
            screen!("[info] {title} metadata is already present")
        }
    }
}

fn reject_reserved<const N: usize>(
    path: &std::path::Path,
    reserved: [&Option<PathBuf>; N],
) -> Result<(), Box<dyn std::error::Error>> {
    for reserved in reserved.into_iter().flatten() {
        if paths_alias(path, reserved)? {
            return Err(yt_dlp_rs::Error::InfoJson(
                "info JSON path collides with cookie file or download archive".into(),
            )
            .into());
        }
    }
    Ok(())
}

fn paths_alias(left: &std::path::Path, right: &std::path::Path) -> io::Result<bool> {
    Ok(std::path::absolute(left)? == std::path::absolute(right)?
        || std::fs::canonicalize(left)
            .ok()
            .zip(std::fs::canonicalize(right).ok())
            .is_some_and(|(left, right)| left == right))
}

/// Apply match filters before selection: rejected entries are skipped, and a
/// breaking rejection truncates entries and records the pending cancellation.
fn prefilter(
    result: ExtractResult,
    filters: &MatchFilters,
    pending: &mut Option<Cancel>,
) -> yt_dlp_rs::Result<Option<ExtractResult>> {
    if filters.is_empty() {
        return Ok(Some(result));
    }
    let mut keep = |info: &yt_dlp_rs::VideoInfo| -> yt_dlp_rs::Result<Option<bool>> {
        Ok(match filters.check(info, true)? {
            MatchOutcome::Pass | MatchOutcome::Ask => Some(true),
            MatchOutcome::Reject(reason) => {
                screen!("[download] {reason}");
                Some(false)
            }
            MatchOutcome::Break(reason) => {
                screen!("[download] {reason}");
                *pending = Some(Cancel::Rejected);
                None
            }
        })
    };
    match result {
        ExtractResult::Video(info) => {
            Ok((keep(&info)? == Some(true)).then_some(ExtractResult::Video(info)))
        }
        ExtractResult::Playlist(mut playlist) => {
            let mut kept = Vec::new();
            for info in playlist.entries {
                match keep(&info)? {
                    Some(true) => kept.push(info),
                    Some(false) => {}
                    None => break,
                }
            }
            playlist.entries = kept;
            Ok(Some(ExtractResult::Playlist(playlist)))
        }
    }
}

/// Drop archived entries. With `break_on_existing`, entries stop at the first
/// archived one and the returned flag requests cancellation after the earlier
/// entries are processed (upstream processes entries lazily in order).
fn filter_archived(
    result: ExtractResult,
    archive: &DownloadArchive,
    break_on_existing: bool,
) -> yt_dlp_rs::Result<(Option<ExtractResult>, bool)> {
    let keep = |info: &yt_dlp_rs::VideoInfo| -> yt_dlp_rs::Result<bool> {
        let recorded = archive.contains(info)?;
        if recorded {
            screen!(
                "[download] {}: {} has already been recorded in the archive",
                info.id,
                info.title
            );
        }
        Ok(!recorded)
    };
    match result {
        ExtractResult::Video(info) => {
            let kept = keep(&info)?;
            Ok((
                kept.then_some(ExtractResult::Video(info)),
                !kept && break_on_existing,
            ))
        }
        ExtractResult::Playlist(mut playlist) => {
            let mut pending = Vec::new();
            let mut reached = false;
            for info in playlist.entries {
                if keep(&info)? {
                    pending.push(info);
                } else if break_on_existing {
                    reached = true;
                    break;
                }
            }
            // An all-archived playlist is still processed for playlist-level files.
            playlist.entries = pending;
            Ok((Some(ExtractResult::Playlist(playlist)), reached))
        }
    }
}

/// The parser and the long `run` need more stack than the 1 MiB some
/// platforms give the main thread, so the CLI runs on its own thread.
const STACK_SIZE: usize = 64 * 1024 * 1024;

fn main() -> std::process::ExitCode {
    std::thread::Builder::new()
        .stack_size(STACK_SIZE)
        .spawn(cli)
        .expect("spawn the CLI thread")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

fn cli() -> std::process::ExitCode {
    match run() {
        Ok(true) => std::process::ExitCode::SUCCESS,
        Ok(false) => std::process::ExitCode::FAILURE,
        Err(error) if error.is::<Cancel>() => std::process::ExitCode::from(101),
        Err(error) if error.is::<UsageError>() => {
            // Upstream `_generate_error_message`, written after a newline.
            eprintln!(
                "\nUsage: yt-dlp-rs [OPTIONS] URL [URL...]\n\nyt-dlp-rs: error: {}",
                yt_dlp_rs::batch::py_strip(&error.to_string())
            );
            std::process::ExitCode::from(2)
        }
        Err(error) => {
            report_error!("ERROR: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Building the parser needs more stack than test threads have.
    fn with_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
        std::thread::Builder::new()
            .stack_size(STACK_SIZE)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn option_aliases_expand_with_formatted_quoted_arguments() {
        let expand = |args: &[&str]| {
            let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
            with_stack(move || expand_preset_aliases(args))
        };
        assert_eq!(
            expand(&[
                "--alias",
                "get-audio,-X",
                "-S aext:{0},abr -x --audio-format {0}",
                "-X",
                "m4a",
                "URL"
            ])
            .unwrap(),
            ["-S", "aext:m4a,abr", "-x", "--audio-format", "m4a", "URL"]
        );
        // Values are shell-quoted into the template, aliases can chain, and
        // argument-less templates are not formatted.
        assert_eq!(
            expand(&[
                "--alias",
                "o",
                "-o {}",
                "--alias",
                "both",
                "--o 'a b' -O {{x}}",
                "--both"
            ])
            .unwrap(),
            ["-o", "a b", "-O", "{{x}}"]
        );
        assert_eq!(
            expand(&["--alias", "loop", "--loop", "--loop"]).unwrap_err(),
            "Alias --loop exceeded invocation limit"
        );
        assert_eq!(
            expand(&["--alias", "bad", "-f {0"]).unwrap_err(),
            "wrong --alias OPTIONS formatting; expected '}' before end of string"
        );
    }

    #[test]
    fn long_option_abbreviations_follow_optparse() {
        let expand = |args: &[&str]| {
            let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
            with_stack(move || expand_preset_aliases(args))
        };
        // Unique prefixes (also of deprecated options and defined aliases)
        // expand; option values and `--` are left alone.
        assert_eq!(
            expand(&[
                "--match-filter",
                "--no-c",
                "--call-h",
                "-o",
                "--yo",
                "--",
                "--x"
            ])
            .unwrap(),
            [
                "--match-filters",
                "--no-c",
                "--call-home",
                "-o",
                "--yo",
                "--",
                "--x"
            ]
        );
        assert_eq!(
            expand(&["--ali", "quiet-sim", "-q", "--quiet-s"]).unwrap(),
            ["-q"]
        );
        assert_eq!(
            expand(&["--no-ch"]).unwrap_err(),
            "ambiguous option: --no-ch (--no-check-certificates, --no-check-formats?)"
        );
        assert_eq!(
            expand(&["--restrict=x"]).unwrap_err(),
            "--restrict-filenames option does not take a value"
        );
        // A short option's attached value is kept verbatim.
        assert_eq!(
            expand(&["-f=best", "-sfbest"]).unwrap(),
            ["-f", "=best", "-sf", "best"]
        );
    }

    #[test]
    fn preset_aliases_expand_in_place_like_upstream() {
        let expand = |args: &[&str]| {
            let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
            with_stack(move || expand_preset_aliases(args))
        };
        assert_eq!(
            expand(&["-t", "mp3", "-o", "-t", "URL"]).unwrap(),
            [
                "-f",
                "ba[acodec^=mp3]/ba/b",
                "-x",
                "--audio-format",
                "mp3",
                "-o",
                "-t",
                "URL"
            ]
        );
        assert_eq!(
            expand(&["-tmkv", "--preset-alias=sleep", "--", "-t"]).unwrap()[..4],
            ["--merge-output-format", "mkv", "--remux-video", "mkv"]
        );
        assert_eq!(
            expand(&["-t", "nope"]).unwrap_err(),
            "Unknown preset alias: nope"
        );
    }

    #[test]
    fn descriptions_follow_upstream_overwrite_and_missing_rules() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sub/video.description");
        let mut info: yt_dlp_rs::VideoInfo = serde_json::from_value(serde_json::json!({
            "id": "v", "title": "t", "ext": "mp4", "url": "https://example.invalid/v.mp4",
            "webpage_url": "https://example.invalid/", "extractor": "generic",
            "extractor_key": "Generic", "direct": true, "description": "Line one\nLine two",
        }))
        .unwrap();
        write_description(&path, &info, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "Line one\nLine two"
        );
        info.metadata.description = Some("changed".into());
        write_description(&path, &info, Some(false)).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "Line one\nLine two"
        );
        write_description(&path, &info, None).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "changed");
        info.metadata.description = None;
        let missing = directory.path().join("none.description");
        write_description(&missing, &info, None).unwrap();
        assert!(!missing.exists());
    }

    #[test]
    fn parse_bytes_matches_upstream_vectors() {
        // Pinned upstream `parse_bytes`; values beyond u64 are rejected natively.
        let cases: &[(&str, Option<u64>)] = &[
            ("50k", Some(51200)),
            ("44.6M", Some(46766490)),
            ("1.5", Some(2)),
            ("2.5", Some(2)),
            ("10 k", Some(10240)),
            ("10\tK", Some(10240)),
            ("1e3", None),
            (".5k", None),
            ("5.k", None),
            ("0", Some(0)),
            ("-1", None),
            ("1KiB", None),
            ("1b", None),
            ("1_0", None),
            ("10 ", None),
            ("1g", Some(1073741824)),
            ("3y", None),
            ("1.25K", Some(1280)),
            ("0.5", Some(0)),
            ("k", None),
            ("12345678901234567890", Some(12345678901234567168)),
            ("1KK", None),
        ];
        for (value, expected) in cases {
            assert_eq!(parse_bytes(value), *expected, "{value:?}");
        }
    }

    #[test]
    fn saved_upstream_simulation_resolution() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/simulate/cases.json")).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let mut argv = vec!["yt-dlp-rs".to_owned()];
            argv.extend(
                case["args"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|arg| arg.as_str().unwrap().to_owned()),
            );
            argv.push("https://example.invalid/".into());
            let simulation = Args::try_parse_from(&argv).unwrap().simulation();
            let expected = Simulation {
                simulate: case["simulate"].as_bool().unwrap(),
                list_only: case["list_only"].as_bool().unwrap(),
                quiet: case["quiet"].as_bool().unwrap(),
            };
            assert_eq!(simulation, expected, "{argv:?}");
        }
    }

    #[test]
    fn saved_embedding_flags_preserve_implied_writing_and_retention_precedence() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/subtitle_embedding/cases.json"
        ))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let flags = case["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|value| value.as_str().unwrap());
            let args = Args::try_parse_from(
                std::iter::once("yt-dlp-rs")
                    .chain(flags)
                    .chain(std::iter::once("https://example.test/video.mp4")),
            )
            .unwrap();
            let expected = &case["expected"];
            assert_eq!(
                args.embed_subs,
                expected["enabled"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(
                args.write_subs || (args.embed_subs && !args.write_auto_subs),
                expected["write_subtitles"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(
                args.write_auto_subs,
                expected["write_automatic"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(
                args.keep_video,
                expected["keep_video"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            if args.embed_subs && case.get("already_have_subtitle").is_none() {
                assert_eq!(
                    args.write_subs,
                    expected["already_have_subtitle"].as_bool().unwrap(),
                    "{}",
                    case["name"]
                );
            }
        }
    }

    #[test]
    fn saved_subtitle_conversion_options_match_pinned_targets_and_alias_precedence() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../tests/fixtures/subtitle_conversion/cases.json"
        ))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let flags = case["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap());
            let args = Args::try_parse_from(
                std::iter::once("yt-dlp-rs")
                    .chain(flags)
                    .chain(std::iter::once("https://example.test/video.mp4")),
            )
            .unwrap();
            let target = args
                .convert_subs
                .as_deref()
                .filter(|target| *target != "none");
            assert_eq!(
                target,
                case["expected"]["target"].as_str(),
                "{}",
                case["name"]
            );
            if let Some(format) = args.convert_subs {
                assert!(SubtitleConversion::new(&format).is_ok());
            }
            assert_eq!(
                args.skip_download,
                case["expected"]["skip_download"].as_bool().unwrap()
            );
        }
    }

    #[test]
    fn automatic_caption_flags_override_in_argument_order() {
        let parse = |flags: &[&str]| {
            Args::try_parse_from(
                std::iter::once("yt-dlp-rs")
                    .chain(flags.iter().copied())
                    .chain(std::iter::once("https://example.test/video.mp4")),
            )
            .unwrap()
        };
        assert!(!parse(&["--write-auto-subs", "--no-write-automatic-subs"]).write_auto_subs);
        assert!(parse(&["--no-write-auto-subs", "--write-automatic-subs"]).write_auto_subs);
        let both = parse(&["--write-subs", "--write-auto-subs"]);
        assert!(both.write_subs && both.write_auto_subs);
    }
}

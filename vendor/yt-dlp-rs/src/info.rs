use serde::{Deserialize, Serialize};
use url::Url;

/// An extractor's `title: None` (an empty title is `''`), consumed by
/// `process_result`'s `_fill_common_fields`.
pub(crate) const TITLE_MISSING: &str = "__yt_dlp_rs_title_missing";

/// An extractor result without formats: `process_video_result`'s
/// `raise_no_formats` after `_fill_common_fields`.
pub(crate) const NO_FORMATS: &str = "__yt_dlp_rs_no_formats";

/// `YoutubeDL._deprecated_multivalue_fields`.
pub(crate) const MULTIVALUE_FIELDS: [(&str, &str); 5] = [
    ("album_artist", "album_artists"),
    ("artist", "artists"),
    ("composer", "composers"),
    ("creator", "creators"),
    ("genre", "genres"),
];

/// Fully extracted results; playlists contain videos, never unresolved URLs.
#[derive(Debug, Clone, Serialize, Deserialize)]
// Preserve the public unboxed Video(VideoInfo) constructor while metadata grows.
#[allow(clippy::large_enum_variant)]
#[serde(tag = "_type", rename_all = "lowercase")]
pub enum ExtractResult {
    Video(VideoInfo),
    Playlist(PlaylistInfo),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistInfo {
    pub id: String,
    pub title: String,
    pub webpage_url: Url,
    pub extractor: String,
    pub extractor_key: String,
    pub entries: Vec<VideoInfo>,
    /// Unmodeled info-dict keys (upstream's `add_default_extra_info`).
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Upstream `url_basename`: the URL path's last segment.
pub fn url_basename(url: &Url) -> String {
    url.path()
        .trim_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Upstream `get_domain`: the URL's netloc without a leading `www.`.
pub fn get_domain(url: &Url) -> Option<String> {
    let mut netloc = String::new();
    if !url.username().is_empty() {
        netloc.push_str(url.username());
        if let Some(password) = url.password() {
            netloc.push(':');
            netloc.push_str(password);
        }
        netloc.push('@');
    }
    netloc.push_str(url.host_str().unwrap_or_default());
    if let Some(port) = url.port() {
        netloc.push_str(&format!(":{port}"));
    }
    let domain = netloc.strip_prefix("www.").unwrap_or(&netloc).to_owned();
    (!domain.is_empty()).then_some(domain)
}

/// `dict.setdefault` for a non-null value.
pub(crate) fn set_default(
    extra: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: impl Into<serde_json::Value>,
) {
    let value = value.into();
    if !value.is_null() {
        extra.entry(key).or_insert(value);
    }
}

impl ExtractResult {
    /// Upstream `add_default_extra_info`: `original_url` (the input URL) and
    /// `webpage_url_basename`/`webpage_url_domain`.
    pub(crate) fn add_default_extra_info(&mut self, input: &str) {
        let (extra, webpage_url) = match self {
            Self::Video(info) => (&mut info.extra, &info.webpage_url),
            Self::Playlist(info) => (&mut info.extra, &info.webpage_url),
        };
        set_default(extra, "original_url", input);
        set_default(extra, "webpage_url_basename", url_basename(webpage_url));
        // `get_domain` is `None` for a pseudo-URL (`jwplatform:ID`).
        match get_domain(webpage_url) {
            Some(domain) => set_default(extra, "webpage_url_domain", domain),
            None => {
                extra
                    .entry("webpage_url_domain")
                    .or_insert(serde_json::Value::Null);
            }
        }
    }
}

impl ExtractResult {
    /// Preserve the single-video API's explicit boundary instead of dropping entries.
    pub fn into_video(self) -> crate::Result<VideoInfo> {
        match self {
            Self::Video(info) => Ok(info),
            Self::Playlist(_) => Err(crate::Error::PlaylistRequiresResult),
        }
    }

    /// The number of videos (playlist entries, or 1).
    pub fn entry_count(&self) -> usize {
        match self {
            Self::Video(_) => 1,
            Self::Playlist(info) => info.entries.len(),
        }
    }

    pub fn into_entries(self) -> Vec<VideoInfo> {
        match self {
            Self::Video(info) => vec![info],
            Self::Playlist(info) => info.entries,
        }
    }

    pub(crate) fn select_format_with_merge(
        &mut self,
        requested: Option<&str>,
        prefer_merge: bool,
        merge_formats: Option<&[String]>,
    ) -> crate::Result<()> {
        // Select every entry before publishing any change to a playlist.
        let mut selected = self.clone();
        match &mut selected {
            Self::Video(info) => {
                info.select_format_with_merge(requested, prefer_merge, merge_formats)?
            }
            Self::Playlist(info) => {
                for entry in &mut info.entries {
                    entry
                        .select_format_with_merge(requested, prefer_merge, merge_formats)
                        .map_err(|error| match error {
                            crate::Error::FormatSelection(message) => {
                                crate::Error::FormatSelection(format!("{}: {message}", entry.id))
                            }
                            other => other,
                        })?;
                }
            }
        }
        *self = selected;
        Ok(())
    }
}

/// Whether a source can play independently or requires a complementary stream.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    /// A generic or muxed source; its precise composition may be unknown.
    #[default]
    Standalone,
    AudioOnly,
    VideoOnly,
}

impl StreamKind {
    fn is_standalone(&self) -> bool {
        *self == Self::Standalone
    }
}

/// A finite byte interval of a media resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ByteRange {
    pub offset: u64,
    pub length: u64,
}

/// AES-128 HLS key location and resolved IV; key bytes are never metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HlsAes128 {
    pub key_url: Url,
    pub iv: [u8; 16],
}

/// An ordered whole-resource or byte-range media fragment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fragment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encryption: Option<HlsAes128>,
    pub url: Url,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_range: Option<ByteRange>,
    /// The manifest's relative location, upstream's `path` (resolved
    /// against the format's `fragment_base_url`); `None` when absolute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Upstream's fragment `duration` in seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
}

/// A direct source or finite manifest plan, with an extractor-assigned format ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoFormat {
    /// Upstream `_calc_headers`: the request headers for this format.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_headers: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_note: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub audio_default: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub audio_autoselect: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fragments: Vec<Fragment>,
    /// Upstream protocol name (`m3u8_native`, `http_dash_segments`, or the URL
    /// scheme); missing values are filled like `determine_protocol`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    /// Upstream codec fields (`parse_codecs`; `none` marks a missing stream).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcodec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acodec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_range: Option<String>,
    /// Upstream `container` (`EXT_dash` for DASH and YouTube single streams).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloader_options: Option<DownloaderOptions>,
    /// Upstream `manifest_stream_number`: the DASH representation's stream
    /// index among those sharing its URL (FFmpeg `-map INPUT:N`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_stream_number: Option<usize>,
    /// Upstream `format_index` with `--hls-split-discontinuity`: the HLS
    /// discontinuity section this format downloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_index: Option<u32>,
    /// Filled like `FormatSorter._fill_sorting_fields`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_ext: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_ext: Option<String>,
    /// Video bitrate, filled like `_fill_sorting_fields` (`0` without video).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vbr: Option<f64>,
    /// Upstream `source_preference` (HLS audio renditions, YouTube).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_preference: Option<i64>,
    /// Extractor `quality` rank (YouTube `qualities`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<PyNumber>,
    /// Audio sample rate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asr: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_channels: Option<u32>,
    /// Upstream `format_resolution` fill (`audio only`, `WxH`, `Hp`, `Wx?`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    /// Upstream `'{id} - {res}{note}'` description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// `round(width / height, 2)`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<f64>,
    pub format_id: String,
    #[serde(default, skip_serializing_if = "StreamKind::is_standalone")]
    pub stream_kind: StreamKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abr: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tbr: Option<f64>,
    pub url: Url,
    pub ext: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesize: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Format keys this port does not model (loaded info JSON, `cookies`).
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// One upstream thumbnail dictionary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Thumbnail {
    pub url: String,
    /// An extractor's preference precedes the `id` `_sanitize_thumbnails`
    /// adds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preference: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    /// Keys this port does not model (an extractor's `filesize`).
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Upstream `sanitize_url`: protocol-relative URLs use `http:` and common
/// scheme typos are repaired.
/// Upstream `_sanitize_thumbnails` over a playlist's thumbnail dicts:
/// `_sort_thumbnails` (preference, width, height, id, url ascending), index
/// IDs, resolutions and sanitized URLs.
pub(crate) fn sanitize_thumbnail_values(thumbnails: &mut [serde_json::Value]) {
    use serde_json::Value;
    let number = |t: &Value, key: &str| t.get(key).and_then(Value::as_i64).unwrap_or(-1);
    let text = |t: &Value, key: &str| {
        t.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    thumbnails.sort_by(|a, b| {
        let key = |t: &Value| {
            (
                number(t, "preference"),
                number(t, "width"),
                number(t, "height"),
                text(t, "id"),
                text(t, "url"),
            )
        };
        key(a).cmp(&key(b))
    });
    for (index, thumbnail) in thumbnails.iter_mut().enumerate() {
        let Some(object) = thumbnail.as_object_mut() else {
            continue;
        };
        if object.get("id").is_none_or(Value::is_null) {
            object.insert("id".into(), index.to_string().into());
        }
        let size = |key: &str| object.get(key).and_then(Value::as_i64).filter(|n| *n != 0);
        if let (Some(width), Some(height)) = (size("width"), size("height")) {
            object.insert("resolution".into(), format!("{width}x{height}").into());
        }
        if let Some(url) = object.get("url").and_then(Value::as_str) {
            let url = sanitize_url(url);
            object.insert("url".into(), url.into());
        }
    }
}

pub fn sanitize_url(url: &str) -> String {
    if url.starts_with("//") {
        return format!("http:{url}");
    }
    if let Some(rest) = url.strip_prefix("httpss://") {
        return format!("https://{rest}");
    }
    for (typo, fixed) in [
        ("rmtp://", "rtmp://"),
        ("rmtpe://", "rtmpe://"),
        ("rmtps://", "rtmps://"),
    ] {
        if let Some(rest) = url.strip_prefix(typo) {
            return format!("{fixed}{rest}");
        }
    }
    url.to_owned()
}

/// Upstream `downloader_options` of a format.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DownloaderOptions {
    /// Chunked HTTP downloading (YouTube HTTPS formats use 10 MiB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_chunk_size: Option<u64>,
}

/// One upstream chapter dictionary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<PyNumber>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<PyNumber>,
    /// Other keys, such as SponsorBlock chapters' `category`, `type`,
    /// `categories`, `name` and `category_names`.
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Represented descriptive metadata; every field is optional and omitted when
/// unknown. Field names and types follow the upstream info dictionary.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Metadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Upstream `chapters` (`start_time`, `title`; `end_time` and missing
    /// titles are filled by `process_video_result`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chapters: Vec<Chapter>,
    /// Upstream `start_time`/`end_time` (YouTube `t`/`start`/`end` URL
    /// parameters), used by `--download-sections "*from-url"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_time: Option<PyNumber>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_time: Option<PyNumber>,
    /// An int or float, as the extractor gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<PyNumber>,
    /// `formatSeconds(duration)`, filled by `_fill_common_fields`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_string: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uploader_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uploader_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub average_rating: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub categories: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playable_in_embed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    /// `YYYYMMDD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upload_date: Option<String>,
    /// UNIX seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<PyNumber>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub availability: Option<String>,
    /// The best thumbnail known to exist, as upstream extractors report it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thumbnails: Vec<Thumbnail>,
}

impl Metadata {
    /// Upstream `_fill_common_fields`: derive `upload_date` from `timestamp`.
    pub fn fill_common_fields(&mut self) {
        if let Some(duration) = self.duration {
            self.duration_string = Some(format_seconds(duration.whole_seconds()));
        }
        if self.upload_date.is_none() {
            self.upload_date = self
                .timestamp
                .map(PyNumber::floor_i64)
                .and_then(crate::dates::strftime_date);
        }
    }

    /// Upstream `process_video_result` chapter filling: a leading chapter
    /// from 0, start/end times from the neighbours (the duration ends the
    /// last one) and `<Untitled Chapter N>` titles.
    pub fn fill_chapters(&mut self) {
        let chapters = &mut self.chapters;
        if chapters
            .first()
            .and_then(|chapter| chapter.start_time)
            .is_some_and(|start| start.as_f64() != 0.0)
        {
            chapters.insert(
                0,
                Chapter {
                    start_time: Some(PyNumber::Int(0)),
                    ..Default::default()
                },
            );
        }
        let duration = self.duration;
        let truthy = |value: Option<PyNumber>| value.filter(|value| value.as_f64() != 0.0);
        for index in 0..chapters.len() {
            // The dummy neighbours: `end_time` 0 before, the duration after.
            let previous_end = if index == 0 {
                Some(PyNumber::Int(0))
            } else {
                chapters[index - 1].end_time
            };
            let next_start = chapters
                .get(index + 1)
                .map_or(duration, |chapter| chapter.start_time);
            let current = &mut chapters[index];
            if current.start_time.is_none() {
                current.start_time = previous_end;
            }
            if truthy(current.end_time).is_none() {
                current.end_time = next_start;
            }
            if current.title.as_deref().is_none_or(str::is_empty) {
                current.title = Some(format!("<Untitled Chapter {}>", index + 1));
            }
        }
    }

    /// Upstream `YoutubeDL._sanitize_thumbnails` (without `--check-formats`):
    /// fall back to `thumbnail`, sort ascending by preference/width/height/
    /// id/url, assign index IDs and resolutions and sanitize URLs.
    pub fn sanitize_thumbnails(&mut self) {
        if self.thumbnails.is_empty() {
            match &self.thumbnail {
                Some(url) if !url.is_empty() => self.thumbnails.push(Thumbnail {
                    url: url.clone(),
                    ..Default::default()
                }),
                _ => return self.finish_thumbnail(),
            }
        }
        self.thumbnails.sort_by(|a, b| {
            let key = |t: &Thumbnail| {
                (
                    t.preference.unwrap_or(-1),
                    t.width.map_or(-1, i64::from),
                    t.height.map_or(-1, i64::from),
                )
            };
            key(a)
                .cmp(&key(b))
                .then_with(|| {
                    a.id.as_deref()
                        .unwrap_or("")
                        .cmp(b.id.as_deref().unwrap_or(""))
                })
                .then_with(|| a.url.cmp(&b.url))
        });
        for (index, thumbnail) in self.thumbnails.iter_mut().enumerate() {
            thumbnail.id.get_or_insert_with(|| index.to_string());
            if let (Some(width), Some(height)) = (thumbnail.width, thumbnail.height)
                && width != 0
                && height != 0
            {
                thumbnail.resolution = Some(format!("{width}x{height}"));
            }
            thumbnail.url = sanitize_url(&thumbnail.url);
        }
        self.finish_thumbnail();
    }

    /// `process_video_result`: sanitize `thumbnail`, or use the best of `thumbnails`.
    fn finish_thumbnail(&mut self) {
        match &self.thumbnail {
            Some(url) if !url.is_empty() => self.thumbnail = Some(sanitize_url(url)),
            _ => {
                if let Some(best) = self.thumbnails.last() {
                    self.thumbnail = Some(best.url.clone());
                }
            }
        }
    }
}

/// `--compat-options format-spec`: the merging default format is
/// `bestvideo+bestaudio/best` (upstream `_default_format_spec`).
pub static COMPAT_FORMAT_SPEC: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Small, explicitly supported subset of yt-dlp's info dictionary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoInfo {
    #[serde(flatten)]
    pub metadata: Metadata,
    /// The selected format's `downloader_options`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downloader_options: Option<DownloaderOptions>,
    #[serde(default, skip_serializing_if = "crate::Subtitles::is_empty")]
    pub automatic_captions: crate::Subtitles,
    #[serde(default, skip_serializing_if = "crate::Subtitles::is_empty")]
    pub subtitles: crate::Subtitles,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_subtitles: Option<crate::RequestedSubtitles>,
    /// Selected maximum original index, used for legacy string-template padding.
    /// Internal rendering context is accepted on input but omitted from JSON output.
    #[serde(default, rename = "__last_playlist_index", skip_serializing)]
    pub last_playlist_index: Option<usize>,
    /// Original one-based position, assigned when applying playlist selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playlist_index: Option<usize>,
    /// One-based selected order, before archive filtering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playlist_autonumber: Option<usize>,
    /// Total number of fully extracted entries before item selection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playlist_count: Option<usize>,
    /// Number of selected entries before archive filtering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub n_entries: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fragments: Vec<Fragment>,
    pub id: String,
    pub title: String,
    /// The extractor's title, filled by `_fill_common_fields`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fulltitle: Option<String>,
    pub ext: String,
    pub url: Url,
    pub webpage_url: Url,
    pub extractor: String,
    pub extractor_key: String,
    /// Upstream sets `direct` only for generic direct links; other results
    /// have no such field (`None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direct: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub formats: Vec<VideoFormat>,
    /// The selected format's `resolution`, `format` and `aspect_ratio`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aspect_ratio: Option<f64>,
    /// The selected format's rates and audio properties (merges: video
    /// `fps`/`vbr`, audio `abr`/`asr`/`audio_channels`, summed `tbr`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<PyNumber>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tbr: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vbr: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abr: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asr: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_channels: Option<u32>,
    /// Extractor sort order (`_format_sort_fields`), before the defaults.
    #[serde(
        default,
        rename = "_format_sort_fields",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub format_sort_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "StreamKind::is_standalone")]
    pub stream_kind: StreamKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_formats: Vec<VideoFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format_id: Option<String>,
    /// The selected format's protocol, `+`-joined for merges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    /// The selected format's codecs; merges take the video's `vcodec` and
    /// `dynamic_range` and the audio's `acodec`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vcodec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acodec: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dynamic_range: Option<String>,
    /// Upstream `container` (`EXT_dash` for DASH and YouTube single streams).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
    /// Upstream `--download-sections` fields: the section's start and end
    /// seconds (`section_end` is missing for the media's end), chapter title
    /// and chapter index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_start: Option<PyNumber>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_end: Option<PyNumber>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section_number: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_ext: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_ext: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filesize: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    /// Fields this port does not model: loaded or derived info-dict keys and
    /// `--parse-metadata`/`--replace-in-metadata` results (serialized last).
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl VideoInfo {
    /// This result narrowed to one of its formats (upstream
    /// `new_info.update(fmt)` for a requested format's download).
    pub fn with_selected_format(&self, format: &VideoFormat) -> VideoInfo {
        let mut info = self.clone();
        info.requested_formats.clear();
        info.apply_single(format);
        info
    }

    /// Whether this is a generic direct link (`direct` is true).
    pub fn is_direct(&self) -> bool {
        self.direct == Some(true)
    }

    /// Select a supported source or explicit pair without implicit merger preference.
    pub fn select_format(&mut self, requested: Option<&str>) -> crate::Result<()> {
        self.select_format_with_merge(requested, false, None)
    }

    /// Upstream format selection (`build_format_selector` over the formats
    /// or, without any, the result itself), applying the first selected
    /// format. Selection failures preserve all fields.
    pub(crate) fn select_format_with_merge(
        &mut self,
        requested: Option<&str>,
        prefer_merge: bool,
        merge_formats: Option<&[String]>,
    ) -> crate::Result<()> {
        let options = crate::format_select::Options {
            merge_output_format: merge_formats.map(<[String]>::to_vec),
            ..Default::default()
        };
        let mut selected = self.select_formats(requested, prefer_merge, &options)?;
        *self = selected.remove(0);
        Ok(())
    }

    /// The format dicts selection runs over (`_get_formats`): the formats
    /// without their fragment lists, or the result itself.
    fn format_dicts(&self) -> Vec<serde_json::Value> {
        if self.formats.is_empty() {
            let mut info = serde_json::to_value(self).unwrap_or_default();
            if let Some(object) = info.as_object_mut() {
                object.shift_remove("formats");
                crate::info_order::strip_markers(object);
            }
            return vec![info];
        }
        self.formats
            .iter()
            .map(|format| {
                let mut value = serde_json::to_value(format).unwrap_or_default();
                if let Some(object) = value.as_object_mut() {
                    object.shift_remove("fragments");
                    crate::info_order::strip_markers(object);
                }
                value
            })
            .collect()
    }

    /// Every format upstream's selector picks (`formats_to_download`), each
    /// applied to a copy of this result; an empty selection is an error.
    pub(crate) fn select_formats(
        &self,
        requested: Option<&str>,
        prefer_merge: bool,
        options: &crate::format_select::Options,
    ) -> crate::Result<Vec<VideoInfo>> {
        let dicts = self.format_dicts();
        let run = |spec: &str, check: bool| -> crate::Result<Vec<crate::format_select::Picked>> {
            let selector = crate::format_select::parse(spec)
                .map_err(|error| crate::Error::Reported(error.0))?;
            let mut tester = |index: usize| match self.formats.get(index) {
                Some(format) => format_works(format),
                None => true,
            };
            let check: Option<&mut dyn FnMut(usize) -> bool> =
                (check && FORMAT_CHECK.with(|slot| slot.borrow().is_some())).then_some(&mut tester);
            selector
                .select(&dicts, options, check)
                .map_err(|error| crate::Error::Reported(error.0))
        };
        // Upstream `_default_format_spec`: without a merger, prefer a single
        // file and warn when that may not be the best available.
        if requested.is_none() && !prefer_merge && !self.formats.is_empty() {
            let ids = |spec: &str| -> crate::Result<Vec<Vec<usize>>> {
                Ok(run(spec, false)?
                    .into_iter()
                    .map(|pick| pick.formats)
                    .collect())
            };
            if ids("b/bv+ba")? != ids("bv*+ba/b")? {
                crate::screen::warning(
                    "ffmpeg not found. The downloaded format may not be the best available. \
                     Installing ffmpeg is strongly recommended: https://github.com/yt-dlp/yt-dlp#dependencies",
                );
            }
        }
        let expression = requested.unwrap_or(if prefer_merge {
            // `allow_multiple_audio_streams or 'format-spec' in compat_opts`.
            if options.multiple_audio
                || COMPAT_FORMAT_SPEC.load(std::sync::atomic::Ordering::Relaxed)
            {
                "bestvideo+bestaudio/best"
            } else {
                "bestvideo*+bestaudio/best"
            }
        } else {
            "best/bestvideo+bestaudio"
        });
        if requested.is_none() {
            crate::screen::debug(&format!("Default format spec: {expression}"));
        }
        let picked = run(expression, true)?;
        if picked.is_empty() {
            return Err(selection_error("Requested format is not available"));
        }
        Ok(picked
            .into_iter()
            .map(|pick| {
                let mut info = self.clone();
                if self.formats.is_empty() {
                    info.requested_formats.clear();
                    return info;
                }
                match &pick.merged {
                    None => info.apply_single(&self.formats[pick.formats[0]]),
                    Some(merged) => info.apply_merged(
                        pick.formats
                            .iter()
                            .map(|index| self.formats[*index].clone())
                            .collect(),
                        merged,
                    ),
                }
                info
            })
            .collect())
    }

    /// `info_dict.update(merged)` for a `_merge` dict: the merged fields,
    /// keeping this result's values where the dict has none.
    fn apply_merged(&mut self, formats: Vec<VideoFormat>, merged: &serde_json::Value) {
        use serde_json::Value;
        let text = |key: &str| merged.get(key).and_then(Value::as_str).map(str::to_owned);
        let number = |key: &str| merged.get(key).and_then(Value::as_f64);
        let present = |key: &str| merged.get(key).is_some();
        self.url = formats[0].url.clone();
        self.format_id = text("format_id");
        self.ext = text("ext").unwrap_or_default();
        self.protocol = text("protocol");
        self.format = text("format").filter(|format| !format.is_empty());
        self.filesize = None;
        self.tbr = number("tbr");
        // `_merge`'s joined `language`/`format_note` and summed size.
        for key in ["language", "format_note", "filesize_approx"] {
            match merged.get(key).filter(|value| !value.is_null()) {
                Some(value) => {
                    self.extra.insert(key.into(), value.clone());
                }
                None => {
                    self.extra.shift_remove(key);
                }
            }
        }
        if present("vcodec") {
            self.width = merged
                .get("width")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok());
            self.height = merged
                .get("height")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok());
            self.resolution = text("resolution");
            self.fps = merged
                .get("fps")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            self.dynamic_range = text("dynamic_range");
            self.vcodec = text("vcodec");
            self.vbr = number("vbr");
            self.aspect_ratio = number("aspect_ratio");
            // The only video's value (a format field), even when unset.
            match merged
                .get("stretched_ratio")
                .filter(|value| !value.is_null())
            {
                Some(ratio) => {
                    self.extra.insert("stretched_ratio".into(), ratio.clone());
                }
                None => {
                    self.extra.shift_remove("stretched_ratio");
                }
            }
        }
        if present("acodec") {
            self.acodec = text("acodec");
            self.abr = number("abr");
            self.asr = merged
                .get("asr")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            self.audio_channels = merged
                .get("audio_channels")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
        }
        self.container = None;
        self.video_ext = None;
        self.audio_ext = None;
        self.stream_kind = StreamKind::Standalone;
        self.fragments.clear();
        self.extra.shift_remove("format_index");
        // The merged dict has no `http_headers` or `cookies`: the info's own
        // `http_headers` (an extractor's entry headers) remain.
        self.extra.shift_remove("cookies");
        self.requested_formats = formats;
    }

    fn apply_single(&mut self, selected: &VideoFormat) {
        // `info_dict.update(format)`: unmodeled format keys, then its headers.
        self.extra.shift_remove("cookies");
        for (key, value) in &selected.extra {
            self.extra.insert(key.clone(), value.clone());
        }
        match &selected.http_headers {
            Some(headers) => {
                self.extra.insert(
                    "http_headers".into(),
                    serde_json::Value::Object(headers.clone()),
                );
            }
            None => {
                self.extra.shift_remove("http_headers");
            }
        }
        self.fragments = selected.fragments.clone();
        self.url = selected.url.clone();
        self.ext = selected.ext.clone();
        self.protocol = Some(selected.determine_protocol());
        self.resolution = selected.resolution.clone();
        self.format = selected.format.clone();
        self.aspect_ratio = selected.aspect_ratio;
        self.fps = selected.fps;
        self.tbr = selected.tbr;
        self.vbr = selected.vbr;
        self.abr = selected.abr;
        self.asr = selected.asr;
        self.audio_channels = selected.audio_channels;
        self.vcodec = selected.vcodec.clone();
        self.acodec = selected.acodec.clone();
        self.dynamic_range = selected.dynamic_range.clone();
        self.container = selected.container.clone();
        self.downloader_options = selected.downloader_options.clone();
        self.video_ext = selected.video_ext.clone();
        self.audio_ext = selected.audio_ext.clone();
        self.filesize = selected.filesize;
        self.width = selected.width;
        self.height = selected.height;
        self.stream_kind = selected.stream_kind;
        self.format_id = Some(selected.format_id.clone());
        // `info_dict.update(best_format)` carries `format_index`.
        match selected.format_index {
            Some(index) => {
                self.extra.insert("format_index".into(), index.into());
            }
            None => {
                self.extra.shift_remove("format_index");
            }
        }
        self.requested_formats.clear();
    }
}

/// A format tester (upstream `_check_formats`) for the current selection.
type FormatCheck = Box<dyn FnMut(&VideoFormat) -> bool>;

thread_local! {
    static FORMAT_CHECK: std::cell::RefCell<Option<FormatCheck>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` with `check` testing the candidates of each selector atom.
pub(crate) fn with_format_check<T>(check: FormatCheck, f: impl FnOnce() -> T) -> T {
    let previous = FORMAT_CHECK.with(|slot| slot.borrow_mut().replace(check));
    let result = f();
    FORMAT_CHECK.with(|slot| *slot.borrow_mut() = previous);
    result
}

/// Whether `format` passes the installed check (always, without one).
fn format_works(format: &VideoFormat) -> bool {
    FORMAT_CHECK.with(|slot| {
        let mut slot = slot.borrow_mut();
        match slot.as_mut() {
            Some(check) => check(format),
            None => true,
        }
    })
}

fn is_false(value: &bool) -> bool {
    !value
}

/// Shared selection/download container policy. Group matching belongs to selection;
/// two explicit format IDs may deliberately override rendition-group matching.
/// Upstream `process_video_result` format ID cleanup: empty IDs become the
/// format's index, selector characters become `_`, duplicates get `-N`
/// suffixes, and IDs that are another common extension get an `f` prefix.
impl VideoFormat {
    /// Upstream `determine_protocol` for the represented URL schemes.
    pub fn determine_protocol(&self) -> String {
        if let Some(protocol) = &self.protocol {
            return protocol.clone();
        }
        let url = self.url.as_str();
        for prefix in ["rtmp", "mms", "rtsp"] {
            if url.starts_with(prefix) {
                return prefix.into();
            }
        }
        let path = url.split('?').next().unwrap_or("");
        match path.rsplit_once('.').map(|(_, ext)| ext) {
            Some("m3u8") => "m3u8_native".into(),
            Some("f4m") => "f4m".into(),
            _ => self.url.scheme().into(),
        }
    }
}

/// Upstream `process_video_result` per-format fills: missing protocols and
/// `video_ext`/`audio_ext` (`FormatSorter._fill_sorting_fields`), `acodec`
/// from audio extensions, and `dynamic_range` `SDR` for video.
pub(crate) fn fill_format_fields(formats: &mut [VideoFormat]) {
    for format in formats {
        format.protocol = Some(format.determine_protocol());
        let ext = format.ext.clone();
        if format.vcodec.as_deref() == Some("none") {
            format.audio_ext = Some(if format.acodec.as_deref() == Some("none") {
                "none".into()
            } else {
                ext.clone()
            });
            format.video_ext = Some("none".into());
        } else {
            format.video_ext = Some(ext.clone());
            format.audio_ext = Some("none".into());
        }
        if matches!(ext.as_str(), "aac" | "opus" | "mp3" | "flac" | "vorbis")
            && format.acodec.is_none()
        {
            format.acodec = Some(ext);
        }
        if format.dynamic_range.is_none() && format.vcodec.as_deref() != Some("none") {
            format.dynamic_range = Some("SDR".into());
        }
        // `_fill_sorting_fields` missing bitrates (`0` marks a missing stream;
        // differences need both operands and non-zero results).
        let no_video = format.vcodec.as_deref() == Some("none");
        let no_audio = format.acodec.as_deref() == Some("none");
        if no_video {
            format.vbr = Some(0.0);
        }
        if no_audio {
            format.abr = Some(0.0);
        }
        let truthy = |value: Option<f64>| value.filter(|value| *value != 0.0);
        if truthy(format.vbr).is_none() && !no_video {
            format.vbr = format
                .tbr
                .zip(format.abr)
                .map(|(t, a)| t - a)
                .filter(|v| *v != 0.0);
        }
        if truthy(format.abr).is_none() && !no_audio {
            format.abr = format
                .tbr
                .zip(format.vbr)
                .map(|(t, v)| t - v)
                .filter(|v| *v != 0.0);
        }
        if truthy(format.tbr).is_none() {
            format.tbr = format
                .vbr
                .zip(format.abr)
                .map(|(v, a)| v + a)
                .filter(|v| *v != 0.0);
        }
    }
}

/// A Python `int` or `float` metadata value, serialized as upstream does
/// (`30` versus `30.0`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PyNumber {
    Int(i64),
    Float(f64),
}

impl PyNumber {
    /// Whole seconds (truncated, at least 0), as `timetuple_from_msec`.
    pub fn whole_seconds(self) -> u64 {
        match self {
            Self::Int(value) => value.max(0) as u64,
            Self::Float(value) => value.max(0.0) as u64,
        }
    }

    /// `int(math.floor(value))`, as `datetime.fromtimestamp` dates a float.
    pub fn floor_i64(self) -> i64 {
        match self {
            Self::Int(value) => value,
            Self::Float(value) => value.floor() as i64,
        }
    }

    pub fn as_f64(self) -> f64 {
        match self {
            Self::Int(value) => value as f64,
            Self::Float(value) => value,
        }
    }
}

/// Upstream `formatSeconds` for whole seconds: `H:MM:SS`, `M:SS` or `S`.
pub fn format_seconds(seconds: u64) -> String {
    let (hours, minutes, seconds) = (seconds / 3600, seconds / 60 % 60, seconds % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else if minutes > 0 {
        format!("{minutes}:{seconds:02}")
    } else {
        seconds.to_string()
    }
}

/// Upstream `format_resolution` with a default.
pub(crate) fn format_resolution(format: &VideoFormat, default: Option<&str>) -> Option<String> {
    if format.vcodec.as_deref() == Some("none") && format.acodec.as_deref() != Some("none") {
        return Some("audio only".into());
    }
    if let Some(resolution) = &format.resolution {
        return Some(resolution.clone());
    }
    match (
        format.width.filter(|w| *w != 0),
        format.height.filter(|h| *h != 0),
    ) {
        (Some(width), Some(height)) => Some(format!("{width}x{height}")),
        (None, Some(height)) => Some(format!("{height}p")),
        (Some(width), None) => Some(format!("{width}x?")),
        (None, None) => default.map(str::to_owned),
    }
}

/// `--compat-options manifest-filesize-approx`: manifest formats also get
/// `filesize_approx` from their bitrate.
pub static COMPAT_MANIFEST_FILESIZE_APPROX: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Upstream `process_video_result` description fields, after format IDs:
/// `resolution`, `aspect_ratio`, `format` (`'{id} - {res}{note}'`) and
/// `filesize_approx` (`filesize_from_tbr(tbr, duration)` for formats without
/// a size or a `manifest_url`, since "tbr" is often a maximum for those).
pub(crate) fn fill_format_descriptions(formats: &mut [VideoFormat], duration: Option<PyNumber>) {
    fill_filesize_approx(formats, duration);
    for format in formats {
        fill_description(format);
    }
}

/// `process_video_result`'s `filesize_approx` (before `sort_formats`).
pub(crate) fn fill_filesize_approx(formats: &mut [VideoFormat], duration: Option<PyNumber>) {
    let manifests = COMPAT_MANIFEST_FILESIZE_APPROX.load(std::sync::atomic::Ordering::Relaxed);
    for format in formats.iter_mut() {
        // Python truthiness: a zero or null size is no size.
        let sized = |value: Option<&serde_json::Value>| {
            value
                .and_then(serde_json::Value::as_f64)
                .is_some_and(|size| size != 0.0)
        };
        if (manifests || !format.extra.contains_key("manifest_url"))
            && format.filesize.is_none_or(|size| size == 0)
            && !sized(format.extra.get("filesize_approx"))
            && let (Some(tbr), Some(duration)) = (format.tbr, duration)
        {
            // `int(duration * tbr * (1000 / 8))`.
            let size = (duration.as_f64() * tbr * (1000.0 / 8.0)).trunc();
            if size.is_finite() && size.abs() < 9.0e15 {
                format
                    .extra
                    .insert("filesize_approx".into(), (size as i64).into());
            }
        }
    }
}

/// `resolution`, `aspect_ratio` and the `format` description.
fn fill_description(format: &mut VideoFormat) {
    {
        if format.resolution.is_none() {
            format.resolution = format_resolution(format, None);
        }
        if format.aspect_ratio.is_none()
            && let (Some(width), Some(height)) = (format.width, format.height.filter(|h| *h != 0))
        {
            let ratio = f64::from(width) / f64::from(height);
            format.aspect_ratio = format!("{ratio:.2}").parse().ok();
        }
        if format.format.is_none() {
            let note = format
                .format_note
                .as_deref()
                .map_or_else(String::new, |note| format!(" ({note})"));
            format.format = Some(format!(
                "{} - {}{note}",
                format.format_id,
                format_resolution(format, Some("unknown")).unwrap_or_default()
            ));
        }
    }
}

/// The fields `FormatSorter` reads from a filled format.
fn sort_fields(format: &VideoFormat) -> crate::format_sort::Fields {
    use crate::format_sort::Value;
    let mut fields = crate::format_sort::Fields::new();
    let mut text = |key: &str, value: Option<&str>| {
        if let Some(value) = value {
            fields.insert(key.to_owned(), Value::Str(value.to_owned()));
        }
    };
    text(
        "format_id",
        Some(&format.format_id)
            .filter(|id| !id.is_empty())
            .map(String::as_str),
    );
    text("ext", Some(&format.ext));
    text("protocol", format.protocol.as_deref());
    text("vcodec", format.vcodec.as_deref());
    text("acodec", format.acodec.as_deref());
    text("dynamic_range", format.dynamic_range.as_deref());
    text("video_ext", format.video_ext.as_deref());
    text("audio_ext", format.audio_ext.as_deref());
    let mut number = |key: &str, value: Option<f64>| {
        if let Some(value) = value {
            fields.insert(key.to_owned(), Value::Num(value));
        }
    };
    number("width", format.width.map(f64::from));
    number("height", format.height.map(f64::from));
    number("tbr", format.tbr);
    number("vbr", format.vbr);
    number("abr", format.abr);
    number("filesize", format.filesize.map(|size| size as f64));
    number(
        "filesize_approx",
        format
            .extra
            .get("filesize_approx")
            .and_then(serde_json::Value::as_f64),
    );
    number(
        "source_preference",
        format.source_preference.map(|p| p as f64),
    );
    number(
        "preference",
        format
            .extra
            .get("preference")
            .and_then(serde_json::Value::as_f64),
    );
    number("quality", format.quality);
    number("fps", format.fps.map(PyNumber::as_f64));
    number("asr", format.asr.map(f64::from));
    number("audio_channels", format.audio_channels.map(f64::from));
    fields
}

/// Upstream `sort_formats`: ascending (worst first) `FormatSorter` order.
pub(crate) fn sort_formats(formats: &mut [VideoFormat], sorter: &crate::format_sort::FormatSorter) {
    sorter.clone().sort(formats, sort_fields);
}

pub(crate) fn sanitize_format_ids(formats: &mut [VideoFormat]) {
    const COMMON_EXTS: &[&str] = &[
        "avi", "flv", "mkv", "mov", "mp4", "webm", "3gp", "aiff", "alac", "flac", "m4a", "mka",
        "mp3", "ogg", "opus", "wav", "mhtml",
    ];
    for (index, format) in formats.iter_mut().enumerate() {
        format.format_id = if format.format_id.is_empty() {
            index.to_string()
        } else {
            format
                .format_id
                .chars()
                .map(|c| {
                    if c.is_whitespace() || ",/+[]()".contains(c) {
                        '_'
                    } else {
                        c
                    }
                })
                .collect()
        };
    }
    let mut counts = std::collections::HashMap::<String, usize>::new();
    for format in formats.iter() {
        *counts.entry(format.format_id.clone()).or_default() += 1;
    }
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for format in formats.iter_mut() {
        let id = format.format_id.clone();
        if counts[&id] > 1 {
            let next = seen.entry(id.clone()).or_default();
            format.format_id = format!("{id}-{next}");
            *next += 1;
        }
        if format.format_id != format.ext && COMMON_EXTS.contains(&format.format_id.as_str()) {
            format.format_id = format!("f{}", format.format_id);
        }
    }
}

/// Native merge inputs: one video-only and one audio-only stream in
/// containers with a fixed local demuxer.
pub(crate) fn mergeable(video: &VideoFormat, audio: &VideoFormat) -> bool {
    video.stream_kind == StreamKind::VideoOnly
        && audio.stream_kind == StreamKind::AudioOnly
        && matches!(video.ext.as_str(), "mp4" | "webm" | "ts")
        // HLS audio is labeled `mp4` like upstream; demuxers sniff contents.
        && matches!(audio.ext.as_str(), "m4a" | "mp4" | "webm" | "aac" | "ts")
}

/// Upstream's merged `ext`: `get_compatible_ext` over the pair's codecs and
/// extensions, with `--merge-output-format` preferences.
#[cfg(test)]
pub(crate) fn merge_container(
    video: &VideoFormat,
    audio: &VideoFormat,
    merge_formats: Option<&[String]>,
) -> String {
    let preferences: Option<Vec<&str>> =
        merge_formats.map(|formats| formats.iter().map(String::as_str).collect());
    crate::codecs::compatible_ext(
        &[video.vcodec.as_deref()],
        &[audio.acodec.as_deref()],
        &[video.ext.as_str()],
        &[audio.ext.as_str()],
        preferences.as_deref(),
    )
}

fn selection_error(message: impl Into<String>) -> crate::Error {
    crate::Error::FormatSelection(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn filesize_approx_follows_filesize_from_tbr_for_non_manifest_formats() {
        let mut formats = vec![
            source("a", StreamKind::Standalone, "mp4", None, None, Some(128.5)),
            source("b", StreamKind::Standalone, "mp4", None, None, Some(128.5)),
            source("c", StreamKind::Standalone, "mp4", None, None, Some(128.5)),
            source("d", StreamKind::Standalone, "mp4", None, None, None),
        ];
        for format in &mut formats {
            format.filesize = None;
        }
        formats[1]
            .extra
            .insert("manifest_url".into(), "https://e.invalid/m.mpd".into());
        formats[2].filesize = Some(7);
        fill_format_descriptions(&mut formats, Some(PyNumber::Int(61)));
        let approx: Vec<_> = formats
            .iter()
            .map(|format| format.extra.get("filesize_approx").cloned())
            .collect();
        // `int(61 * 128.5 * 125)`; manifests, sized and bitrate-less formats
        // get none.
        assert_eq!(approx, [Some(json!(979_812)), None, None, None]);
        let mut formats = vec![source(
            "a",
            StreamKind::Standalone,
            "mp4",
            None,
            None,
            Some(1.0),
        )];
        formats[0].filesize = None;
        fill_format_descriptions(&mut formats, None);
        assert!(!formats[0].extra.contains_key("filesize_approx"));
    }

    fn source(
        id: &str,
        kind: StreamKind,
        ext: &str,
        height: Option<u32>,
        abr: Option<f64>,
        tbr: Option<f64>,
    ) -> VideoFormat {
        // Like extracted formats, separate streams name their missing codec.
        VideoFormat {
            http_headers: None,
            extra: Default::default(),
            protocol: None,
            vcodec: (kind == StreamKind::AudioOnly).then(|| "none".into()),
            acodec: (kind == StreamKind::VideoOnly).then(|| "none".into()),
            dynamic_range: None,
            video_ext: None,
            audio_ext: None,
            vbr: None,
            source_preference: None,
            quality: None,
            fps: None,
            asr: None,
            audio_channels: None,
            resolution: None,
            format: None,
            aspect_ratio: None,
            audio_group: None,
            language: None,
            format_note: None,
            audio_default: false,
            audio_autoselect: false,
            downloader_options: None,
            manifest_stream_number: None,
            format_index: None,
            container: None,
            fragments: Vec::new(),
            format_id: id.into(),
            stream_kind: kind,
            abr,
            tbr,
            url: Url::parse(&format!("https://example.com/{id}.{ext}")).unwrap(),
            ext: ext.into(),
            filesize: Some(42),
            width: None,
            height,
        }
    }

    fn info(formats: Vec<VideoFormat>) -> VideoInfo {
        VideoInfo {
            protocol: None,
            format_sort_fields: Vec::new(),
            resolution: None,
            format: None,
            aspect_ratio: None,
            fps: None,
            tbr: None,
            vbr: None,
            abr: None,
            asr: None,
            audio_channels: None,
            vcodec: None,
            acodec: None,
            dynamic_range: None,
            container: None,
            section_start: None,
            section_end: None,
            section_title: None,
            section_number: None,
            extra: Default::default(),
            downloader_options: None,
            video_ext: None,
            audio_ext: None,
            metadata: Default::default(),
            automatic_captions: Default::default(),
            subtitles: Default::default(),
            requested_subtitles: None,
            last_playlist_index: None,
            playlist_index: None,
            playlist_autonumber: None,
            playlist_count: None,
            n_entries: None,
            fragments: Vec::new(),
            id: "test".into(),
            title: "Test".into(),
            fulltitle: None,
            ext: "mp4".into(),
            url: Url::parse("https://example.com/original.mp4").unwrap(),
            webpage_url: Url::parse("https://example.com/watch").unwrap(),
            extractor: "test".into(),
            extractor_key: "Test".into(),
            direct: None,
            stream_kind: StreamKind::Standalone,
            requested_formats: Vec::new(),
            formats,
            format_id: None,
            filesize: None,
            width: None,
            height: None,
        }
    }

    fn adaptive() -> VideoInfo {
        info(vec![
            source(
                "v1",
                StreamKind::VideoOnly,
                "mp4",
                Some(720),
                None,
                Some(1000.0),
            ),
            source(
                "v2",
                StreamKind::VideoOnly,
                "mp4",
                Some(1080),
                None,
                Some(2000.0),
            ),
            source(
                "a1",
                StreamKind::AudioOnly,
                "m4a",
                None,
                Some(128.0),
                Some(130.0),
            ),
            source(
                "a2",
                StreamKind::AudioOnly,
                "m4a",
                None,
                Some(192.0),
                Some(194.0),
            ),
        ])
    }

    fn snapshot(info: &VideoInfo) -> Value {
        serde_json::to_value(info).unwrap()
    }

    #[test]
    fn standalone_serialization_and_legacy_deserialization_are_unchanged() {
        let legacy = json!({
            "id":"test", "title":"Test", "ext":"mp4", "url":"https://example.com/original.mp4",
            "webpage_url":"https://example.com/watch", "extractor":"test", "extractor_key":"Test",
            "direct":false,
            "formats":[{"format_id":"0", "url":"https://example.com/0.mp4", "ext":"mp4", "filesize":42}]
        });
        let decoded: VideoInfo = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(decoded.stream_kind, StreamKind::Standalone);
        assert_eq!(decoded.formats[0].stream_kind, StreamKind::Standalone);
        assert!(decoded.requested_formats.is_empty());
        assert_eq!(snapshot(&decoded), legacy);
        let adaptive = adaptive();
        let encoded = snapshot(&adaptive);
        assert_eq!(encoded["formats"][0]["stream_kind"], "video_only");
        assert_eq!(encoded["formats"][2]["stream_kind"], "audio_only");
        let decoded: VideoInfo = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(snapshot(&decoded), encoded);
    }

    #[test]
    fn best_standalone_preserves_resolution_and_declaration_ties() {
        let mut video = info(vec![
            source(
                "unknown",
                StreamKind::Standalone,
                "mp4",
                None,
                None,
                Some(9000.0),
            ),
            source(
                "first",
                StreamKind::Standalone,
                "mp4",
                Some(720),
                None,
                Some(9000.0),
            ),
            source("last", StreamKind::Standalone, "mp4", Some(720), None, None),
            source(
                "silent",
                StreamKind::VideoOnly,
                "mp4",
                Some(2160),
                None,
                None,
            ),
        ]);
        video.select_format(None).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("last"));
        video.select_format(Some("worst")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("unknown"));
        video.formats.remove(0);
        video.select_format(Some("worst")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("first"));
    }

    #[test]
    fn audio_ranks_abr_then_total_bitrate_with_declaration_ties() {
        let mut video = adaptive();
        video.formats.push(source(
            "a3",
            StreamKind::AudioOnly,
            "webm",
            None,
            Some(192.0),
            Some(200.0),
        ));
        video.formats.push(source(
            "a4",
            StreamKind::AudioOnly,
            "webm",
            None,
            Some(192.0),
            Some(200.0),
        ));
        video.select_format(Some("bestaudio")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("a4"));
        video.select_format(Some("worstaudio")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("a1"));
        video
            .formats
            .retain(|f| f.stream_kind == StreamKind::AudioOnly);
        video.select_format(Some("best")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("a4"));
        assert_eq!(video.stream_kind, StreamKind::AudioOnly);
    }

    #[test]
    fn video_ranks_smaller_dimension_then_bitrate() {
        let mut video = adaptive();
        let mut portrait = source(
            "portrait",
            StreamKind::VideoOnly,
            "webm",
            Some(1920),
            None,
            Some(4000.0),
        );
        portrait.width = Some(720);
        video.formats.push(portrait);
        video.formats.push(source(
            "tie1",
            StreamKind::VideoOnly,
            "mp4",
            Some(1080),
            None,
            Some(3000.0),
        ));
        video.formats.push(source(
            "tie2",
            StreamKind::VideoOnly,
            "mp4",
            Some(1080),
            None,
            Some(3000.0),
        ));
        video.select_format(Some("bestvideo")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("tie2"));
        video.select_format(Some("worstvideo")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v1"));
    }

    #[test]
    fn reversed_merge_preserves_order_and_video_metadata() {
        let mut video = adaptive();
        video.select_format(Some("a1+v2")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("a1+v2"));
        assert_eq!(
            video
                .requested_formats
                .iter()
                .map(|f| f.format_id.as_str())
                .collect::<Vec<_>>(),
            ["a1", "v2"]
        );
        assert_eq!(video.url, video.formats[2].url);
        assert_eq!(video.height, Some(1080));
        assert_eq!(video.filesize, None);
        assert_eq!(video.ext, "mp4");
        assert_eq!(video.stream_kind, StreamKind::Standalone);
        video.select_format(Some("v1")).unwrap();
        assert!(video.requested_formats.is_empty());
        assert_eq!(video.stream_kind, StreamKind::VideoOnly);
        assert_eq!(video.filesize, Some(42));
    }

    #[test]
    fn implicit_defaults_follow_upstream_with_and_without_merger() {
        let mut video = adaptive();
        video.select_format_with_merge(None, true, None).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v2+a2"));
        assert!(
            video
                .select_format_with_merge(Some("best"), true, None)
                .is_err()
        );
        // Without a merger, `best/bestvideo+bestaudio` falls back to the pair.
        video.select_format(None).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v2+a2"));
        video
            .formats
            .retain(|f| f.stream_kind == StreamKind::VideoOnly);
        // Upstream `incomplete_formats`: with only video-only formats,
        // best falls back to them with or without a merger.
        video.select_format(None).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v2"));
        video.select_format_with_merge(None, true, None).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v2"));
        video.formats.push(source(
            "complete",
            StreamKind::Standalone,
            "mp4",
            Some(360),
            None,
            None,
        ));
        video.select_format_with_merge(None, true, None).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("complete"));
        video.formats.push(source(
            "audio",
            StreamKind::AudioOnly,
            "mp3",
            None,
            None,
            None,
        ));
        video.select_format_with_merge(None, true, None).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("complete"));
    }

    #[test]
    fn merge_default_prefers_complete_pair_while_explicit_best_stays_standalone() {
        let mut video = adaptive();
        video.formats.push(source(
            "muxed",
            StreamKind::Standalone,
            "mp4",
            Some(2160),
            None,
            None,
        ));
        video.select_format(None).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("muxed"));
        // Upstream `bestvideo*+bestaudio/best`: the complete 2160p format
        // ranks above the separate video and its audio is not added again.
        video.select_format_with_merge(None, true, None).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("muxed"));
        assert!(video.requested_formats.is_empty());
        video
            .select_format_with_merge(Some("bv+ba"), true, None)
            .unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v2+a2"));
        video
            .select_format_with_merge(Some("best"), true, None)
            .unwrap();
        assert_eq!(video.format_id.as_deref(), Some("muxed"));
        for (expression, expected) in [("v1+v2", "v1"), ("a1+a2", "a1"), ("muxed+a1", "muxed")] {
            video.select_format(Some(expression)).unwrap();
            assert_eq!(video.format_id.as_deref(), Some(expected), "{expression}");
        }
    }

    #[test]
    fn merge_container_policy_and_fallback_are_explicit() {
        let mut video = adaptive();
        video.formats[0].ext = "webm".into();
        video.formats[2].ext = "webm".into();
        video.select_format(Some("v1+a1")).unwrap();
        assert_eq!(video.ext, "webm");
        video.select_format(Some("v2+a1")).unwrap();
        assert_eq!(video.ext, "mkv");
        video
            .select_format(Some("missing+a1/v1+missing/a1+v1"))
            .unwrap();
        assert_eq!(video.format_id.as_deref(), Some("a1+v1"));
    }

    #[test]
    fn errors_are_atomic_and_validate_all_fallback_grammar_first() {
        let mut video = adaptive();
        video.select_format(Some("v2+a2")).unwrap();
        let before = snapshot(&video);
        // Upstream's syntax/filter errors and empty selections.
        for expression in [
            "v1+missing",
            "v1+",
            "/v1",
            "(v1",
            "v1[height>abc]",
            "v1[unsupported_field>10]",
            "",
        ] {
            assert!(
                video.select_format(Some(expression)).is_err(),
                "{expression}"
            );
            assert_eq!(snapshot(&video), before, "{expression}");
        }
    }

    fn grouped(mut format: VideoFormat, group: &str) -> VideoFormat {
        format.audio_group = Some(group.into());
        format
    }

    /// Two HLS rendition groups, already in sorted (worst-first) order as
    /// `process_result` leaves them.
    fn grouped_info() -> VideoInfo {
        info(vec![
            grouped(
                source(
                    "v-b",
                    StreamKind::VideoOnly,
                    "ts",
                    Some(720),
                    None,
                    Some(1000.0),
                ),
                "b",
            ),
            grouped(
                source(
                    "v-a",
                    StreamKind::VideoOnly,
                    "ts",
                    Some(1080),
                    None,
                    Some(2000.0),
                ),
                "a",
            ),
            grouped(
                source("a-a", StreamKind::AudioOnly, "aac", None, Some(64.0), None),
                "a",
            ),
            grouped(
                source("a-b", StreamKind::AudioOnly, "aac", None, Some(192.0), None),
                "b",
            ),
        ])
    }

    #[test]
    fn pairs_combine_first_results_in_expression_order_across_rendition_groups() {
        // Upstream `_merge` pairs each side's first result; rendition groups
        // do not constrain selection (checked with pinned upstream
        // `build_format_selector` on the same worst-first list).
        let mut video = grouped_info();
        video.select_format(Some("bestvideo+bestaudio")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v-a+a-b"));
        // Upstream's compatible extension sets do not include `aac`.
        assert_eq!(video.ext, "mkv");
        video.select_format(Some("bestaudio+bestvideo")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("a-b+v-a"));
        assert_eq!(video.url, video.requested_formats[0].url);
        video.formats.retain(|format| format.format_id != "a-b");
        video.select_format_with_merge(None, true, None).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v-a+a-a"));
    }

    #[test]
    fn literal_ids_pair_with_selectors_and_failed_pairs_fall_back_atomically() {
        let mut video = grouped_info();
        for (expression, expected) in [
            ("v-b+bestaudio", "v-b+a-b"),
            ("bestaudio+v-b", "a-b+v-b"),
            ("a-a+bestvideo", "a-a+v-a"),
            ("bestvideo+a-a", "v-a+a-a"),
            ("v-a+a-b", "v-a+a-b"),
        ] {
            video.select_format(Some(expression)).unwrap();
            assert_eq!(video.format_id.as_deref(), Some(expected), "{expression}");
        }
        let before = snapshot(&video);
        assert!(video.select_format(Some("v-a+missing")).is_err());
        assert_eq!(snapshot(&video), before);
        video
            .select_format(Some("v-a+missing/v-b+bestaudio"))
            .unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v-b+a-b"));
    }

    #[test]
    fn audio_rendition_defaults_then_autoselect_precede_bitrate() {
        let mut video = grouped_info();
        video.formats.push(grouped(
            source(
                "autoselect",
                StreamKind::AudioOnly,
                "aac",
                None,
                Some(32.0),
                None,
            ),
            "a",
        ));
        video.formats.last_mut().unwrap().audio_autoselect = true;
        video.select_format(Some("v-a+bestaudio")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v-a+autoselect"));
        video.formats.push(grouped(
            source(
                "default",
                StreamKind::AudioOnly,
                "aac",
                None,
                Some(16.0),
                None,
            ),
            "a",
        ));
        let default = video.formats.last_mut().unwrap();
        default.audio_default = true;
        default.language = Some("en".into());
        default.format_note = Some("English original".into());
        video.select_format(Some("v-a+bestaudio")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v-a+default"));
        video.select_format(Some("v-a+a-a")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v-a+a-a"));
        let roundtrip: VideoInfo = serde_json::from_value(snapshot(&video)).unwrap();
        assert_eq!(
            roundtrip.formats.last().unwrap().language.as_deref(),
            Some("en")
        );
        assert_eq!(
            roundtrip.formats.last().unwrap().format_note.as_deref(),
            Some("English original")
        );
        assert!(roundtrip.formats.last().unwrap().audio_default);
    }

    #[test]
    fn shared_merge_policy_validates_kinds_and_uses_upstream_compatible_ext() {
        for video_ext in ["mp4", "ts", "webm"] {
            for audio_ext in ["m4a", "aac", "ts", "webm"] {
                let video = source("v", StreamKind::VideoOnly, video_ext, None, None, None);
                let audio = source("a", StreamKind::AudioOnly, audio_ext, None, None, None);
                // Without codecs, upstream's extension sets decide.
                let expected = match (video_ext, audio_ext) {
                    ("mp4", "m4a") => "mp4",
                    ("ts", "ts") => "ts",
                    ("webm", "webm") => "webm",
                    _ => "mkv",
                };
                assert!(mergeable(&video, &audio));
                assert!(!mergeable(&audio, &video));
                assert_eq!(merge_container(&video, &audio, None), expected);
            }
        }
        let mut video = source("v", StreamKind::VideoOnly, "mp4", None, None, None);
        let mut audio = source("a", StreamKind::AudioOnly, "webm", None, None, None);
        video.vcodec = Some("vp09.00.10.08".into());
        audio.acodec = Some("opus".into());
        assert_eq!(merge_container(&video, &audio, None), "webm");
        let preferences = ["mp4".to_owned()];
        assert_eq!(merge_container(&video, &audio, Some(&preferences)), "mp4");
        let video = source("v", StreamKind::Standalone, "mp4", None, None, None);
        let audio = source("a", StreamKind::AudioOnly, "aac", None, None, None);
        assert!(!mergeable(&video, &audio));
        let video = source("v", StreamKind::VideoOnly, "ts", None, None, None);
        let audio = source("a", StreamKind::AudioOnly, "mp3", None, None, None);
        assert!(!mergeable(&video, &audio));
    }

    #[test]
    fn direct_results_accept_fallback_but_validate_grammar() {
        let mut video = info(Vec::new());
        video.select_format(Some("missing/best")).unwrap();
        let before = snapshot(&video);
        assert!(video.select_format(Some("best/(bad")).is_err());
        assert_eq!(snapshot(&video), before);
    }

    #[test]
    fn failed_playlist_selection_preserves_preceding_entries() {
        let mut first = adaptive();
        first.id = "first".into();
        let mut second = adaptive();
        second.id = "second".into();
        second.formats.retain(|f| f.format_id != "v2");
        let mut playlist = ExtractResult::Playlist(PlaylistInfo {
            id: "list".into(),
            title: "List".into(),
            webpage_url: first.webpage_url.clone(),
            extractor: "test".into(),
            extractor_key: "Test".into(),
            entries: vec![first, second],
            extra: Default::default(),
        });
        let before = serde_json::to_value(&playlist).unwrap();
        let error = playlist
            .select_format_with_merge(Some("v2+a2"), false, None)
            .unwrap_err();
        assert!(error.to_string().contains("second"));
        assert_eq!(serde_json::to_value(&playlist).unwrap(), before);
    }

    #[test]
    fn filters_narrow_candidates_before_ranking_kind_fallback_and_pairing() {
        let mut video = adaptive();
        video
            .select_format(Some("bestvideo[height<=720]+bestaudio[abr>=128]"))
            .unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v1+a2"));
        video
            .select_format(Some("bestvideo[height<100]+bestaudio/bestaudio[ext=m4a]"))
            .unwrap();
        assert_eq!(video.format_id.as_deref(), Some("a2"));
        video.formats.push(source(
            "muxed",
            StreamKind::Standalone,
            "webm",
            Some(2160),
            None,
            None,
        ));
        // Upstream `best` needs a complete stream, so an audio-only match
        // fails while complete and separate formats coexist.
        video.select_format(Some("best[ext=m4a]")).unwrap_err();
        video.select_format(Some("best[height<=?720]")).unwrap_err();
    }

    #[test]
    fn filtered_literal_pairs_keep_explicit_group_override_and_failures_are_atomic() {
        let mut video = adaptive();
        video.formats[0].audio_group = Some("one".into());
        video.formats[2].audio_group = Some("two".into());
        video
            .select_format(Some("v1[height<=720]+a1[ext=m4a]"))
            .unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v1+a1"));
        let before = snapshot(&video);
        for expression in ["v1[height>720]+a1", "v1[width>=1]/best[unknown=1]"] {
            assert!(
                video.select_format(Some(expression)).is_err(),
                "{expression}"
            );
            assert_eq!(snapshot(&video), before);
        }
    }

    #[test]
    fn implicit_format_filters_use_known_metadata_and_keep_missing_fields_unknown() {
        let mut video = info(Vec::new());
        video.filesize = Some(1024);
        video.ext = "mp4".into();
        video
            .select_format(Some("best[filesize<=1KiB][ext=mp4]"))
            .unwrap();
        assert_eq!(video.format_id, None);
        assert!(video.formats.is_empty());
        video
            .select_format(Some("best[height<=?720][format_id!=?x]"))
            .unwrap();
        let before = snapshot(&video);
        for expression in [
            "best[height<=720]",
            "best[format_id!=x]",
            "best[filesize<1024]",
        ] {
            assert!(video.select_format(Some(expression)).is_err());
            assert_eq!(snapshot(&video), before);
        }
    }

    #[test]
    fn grouped_fallbacks_keep_boundaries_inside_pairs() {
        let mut video = adaptive();
        video.select_format(Some("(v1/bv)+a1")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v1+a1"));
        video.select_format(Some("(bv/v1)+a1")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v2+a1"));
        video.select_format(Some("(missing/v1)+a1")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v1+a1"));
        let before = snapshot(&video);
        assert!(video.select_format(Some("(bv/v1)+missing")).is_err());
        assert_eq!(snapshot(&video), before);
        video
            .select_format(Some("((bv/v1)+missing)/(v1+a1)"))
            .unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v1+a1"));
    }

    #[test]
    fn saved_selector_kinds_match_pinned_upstream_per_context() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/selectors/kinds.json")).unwrap();
        for context in fixture["contexts"].as_array().unwrap() {
            let original: VideoInfo = serde_json::from_value(context["info"].clone()).unwrap();
            for (expression, expected) in context["expected"].as_object().unwrap() {
                let mut info = original.clone();
                let expected: Vec<_> = expected
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|id| id.as_str().unwrap())
                    .collect();
                let name = context["name"].as_str().unwrap();
                match info.select_format(Some(expression)) {
                    Ok(()) => {
                        let actual: Vec<_> = if info.requested_formats.is_empty() {
                            vec![info.format_id.as_deref().unwrap()]
                        } else {
                            info.requested_formats
                                .iter()
                                .map(|format| format.format_id.as_str())
                                .collect()
                        };
                        assert_eq!(actual, expected, "{name}: {expression}");
                    }
                    Err(error) => assert!(expected.is_empty(), "{name}: {expression}: {error}"),
                }
            }
        }
    }

    #[test]
    fn saved_grouped_selector_expressions_match_pinned_upstream_ids() {
        let fixture: Value =
            serde_json::from_str(include_str!("../tests/fixtures/selectors/expressions.json"))
                .unwrap();
        let original: VideoInfo = serde_json::from_value(fixture["info"].clone()).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let expression = case["selector"].as_str().unwrap();
            let mut info = original.clone();
            let before = snapshot(&info);
            let expected: Vec<_> = case["expected_ids"]
                .as_array()
                .unwrap()
                .iter()
                .map(|id| id.as_str().unwrap())
                .collect();
            let selected = info.select_format(Some(expression));
            if expected.is_empty() {
                assert!(
                    matches!(selected, Err(crate::Error::FormatSelection(_))),
                    "{expression}"
                );
                assert_eq!(snapshot(&info), before, "{expression}");
            } else {
                selected.unwrap_or_else(|error| panic!("{expression}: {error}"));
                let actual: Vec<_> = if info.requested_formats.is_empty() {
                    vec![info.format_id.as_deref().unwrap()]
                } else {
                    info.requested_formats
                        .iter()
                        .map(|format| format.format_id.as_str())
                        .collect()
                };
                assert_eq!(actual, expected, "{expression}");
            }
        }
        for case in fixture["invalid_cases"]
            .as_array()
            .unwrap()
            .iter()
            .chain(fixture["excluded_index_cases"].as_array().unwrap())
        {
            let mut info = original.clone();
            let before = snapshot(&info);
            let expression = case["selector"].as_str().unwrap();
            assert!(
                info.select_format(Some(expression)).is_err(),
                "{expression}"
            );
            assert_eq!(snapshot(&info), before, "{expression}");
        }
    }

    #[test]
    fn indexed_atoms_choose_after_filters_and_keep_rank_ties_and_fallbacks() {
        let mut video = adaptive();
        video.select_format(Some("bv.2+ba.2")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v1+a1"));
        video.select_format(Some("wv.2+wa.2")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v2+a2"));
        video
            .select_format(Some("bv.2[height<=720]/bv.1[height<=720]"))
            .unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v1"));
        video.formats.push(source(
            "tie",
            StreamKind::VideoOnly,
            "mp4",
            Some(1080),
            None,
            Some(2000.0),
        ));
        video.select_format(Some("bv.2")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v2"));
        video.select_format(Some("(bv/ba).2")).unwrap_err();
    }

    #[test]
    fn explicit_indexes_pin_pair_candidates_and_fall_back_atomically() {
        let mut video = adaptive();
        video.select_format(Some("bv.1+ba")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v2+a2"));
        video.select_format(Some("bv.2+ba")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v1+a2"));
        let before = snapshot(&video);
        assert!(video.select_format(Some("bv.3+ba")).is_err());
        assert_eq!(snapshot(&video), before);
        video.select_format(Some("(bv.3+ba)/(bv.2+ba.2)")).unwrap();
        assert_eq!(video.format_id.as_deref(), Some("v1+a1"));
    }

    #[test]
    fn implicit_indexes_and_dotted_literal_ids_preserve_metadata_boundaries() {
        let mut direct = info(Vec::new());
        direct.select_format(Some("(b.1/w.1)[ext=mp4]")).unwrap();
        let before = snapshot(&direct);
        assert!(direct.select_format(Some("best.2")).is_err());
        assert_eq!(snapshot(&direct), before);
        direct.select_format(Some("b.2/w.1")).unwrap();
        assert_eq!(direct.format_id, None);
        for id in ["bv.0", "bv.01", "bv.x", "format.2"] {
            let mut video = info(vec![source(
                id,
                StreamKind::Standalone,
                "mp4",
                Some(720),
                None,
                None,
            )]);
            video.select_format(Some(id)).unwrap();
            assert_eq!(video.format_id.as_deref(), Some(id));
        }
    }
}

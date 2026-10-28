//! Deliberately narrow HTML5 subset of upstream GenericIE and
//! HTML5MediaEmbedIE / InfoExtractor._parse_html5_media_entries at the pinned revision.
use super::{Extractor, dash, direct, hls};
use crate::{
    Error, ExtractResult, PlaylistInfo, Result, SubtitleFormat, Subtitles, VideoFormat, VideoInfo,
};
use percent_encoding::percent_decode_str;
use reqwest::{
    blocking::Client,
    header::{CONTENT_ENCODING, CONTENT_TYPE},
};
use scraper::{Html, Selector};
use std::{collections::HashMap, io::Read};
use url::Url;

pub struct GenericHttp;

const MAX_HTML_BYTES: u64 = 2 * 1024 * 1024;

struct Html5Page {
    entries: Vec<Html5Entry>,
    id: String,
    title: String,
    playlist_title: String,
}

struct Html5Entry {
    sources: Vec<Html5Source>,
    subtitles: Subtitles,
    /// `absolute_url(poster)`.
    thumbnail: Option<String>,
}

// Match the pinned determine_ext URL suffix rules, without MIME requests.
fn subtitle_extension(url: &Url) -> String {
    let url = url.as_str().split('?').next().unwrap_or("");
    let Some((_, guess)) = url.rsplit_once('.') else {
        return "unknown_video".into();
    };
    if !guess.is_empty() && guess.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return guess.to_ascii_lowercase();
    }
    let trimmed = guess.trim_end_matches('/');
    if matches!(
        trimmed,
        "3g2"
            | "3gp"
            | "f4v"
            | "mk3d"
            | "divx"
            | "mpg"
            | "ogv"
            | "m4v"
            | "wmv"
            | "avi"
            | "flv"
            | "mkv"
            | "mov"
            | "mp4"
            | "webm"
            | "aac"
            | "ape"
            | "asf"
            | "f4a"
            | "f4b"
            | "m4b"
            | "m4r"
            | "oga"
            | "ogx"
            | "spx"
            | "vorbis"
            | "wma"
            | "weba"
            | "aiff"
            | "alac"
            | "flac"
            | "m4a"
            | "mka"
            | "mp3"
            | "ogg"
            | "opus"
            | "wav"
            | "f4f"
            | "f4m"
            | "m3u8"
            | "smil"
            | "mpd"
    ) {
        trimmed.into()
    } else {
        "unknown_video".into()
    }
}

/// `--force-generic-extractor` (GenericIE's `force_generic_extractor`),
/// which words the generic warning.
pub(crate) static FORCE_GENERIC: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

struct Html5Source {
    url: Url,
    hls_hint: bool,
    dash_hint: bool,
    width: Option<u32>,
    height: Option<u32>,
    /// Upstream `vcodec`: `none` inside `<audio>`, otherwise unknown.
    vcodec: Option<String>,
    /// `parse_content_type` codecs of a `<source>` (the media element's own
    /// `src` keeps only its type's extension upstream).
    codecs: Option<crate::codecs::Codecs>,
    /// A `<source>`'s `label` or `title`; otherwise the ID is the format's
    /// index after sorting.
    label: Option<String>,
    /// `parse_bitrate` of the labels.
    tbr: Option<f64>,
    /// A `<source>` element (rather than the media element's `src`).
    element: bool,
}

/// Upstream `parse_content_type`'s `codecs` group:
/// `[^/]+/[^;]+(?:;\s*codecs="?([^"]+))?`.
fn content_type_codecs(content_type: &str) -> Option<&str> {
    static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r#"[^/]+/[^;]+(?:;\s*codecs="?([^"]+))?"#).unwrap()
    });
    PATTERN
        .captures(content_type)?
        .get(1)
        .map(|codecs| codecs.as_str())
}

/// Upstream `int_or_none` (Python `int()`: surrounding whitespace, a sign and
/// single underscores between digits); values outside `u32` stay unknown.
fn dimension(value: Option<&str>) -> Option<u32> {
    let value = value?.trim();
    let digits = value.strip_prefix('+').unwrap_or(value);
    if digits.starts_with('-') {
        // Negative values are valid upstream but not representable here.
        return None;
    }
    let valid = !digits.is_empty()
        && !digits.starts_with('_')
        && !digits.ends_with('_')
        && !digits.contains("__")
        && digits
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'_');
    if !valid {
        return None;
    }
    digits.replace('_', "").parse().ok()
}

/// Upstream `parse_resolution` (non-lenient): `W x H`, `NNNp`/`i`, or `4k`/`8k`.
pub(crate) fn parse_resolution(text: &str) -> (Option<u32>, Option<u32>) {
    static SIZE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?:^|[^a-zA-Z0-9])(\d+)\s*[xX×,]\s*(\d+)(?:$|[^a-zA-Z0-9])").unwrap()
    });
    static HEIGHT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"(?:^|[^a-zA-Z0-9])(\d+)[pPiI](?:\d{2,3})?(?:$|[^a-zA-Z0-9])").unwrap()
    });
    static K: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\b([48])[kK](?:\d{2,3})?\b").unwrap());
    if let Some(found) = SIZE.captures(text) {
        return (found[1].parse().ok(), found[2].parse().ok());
    }
    if let Some(found) = HEIGHT.captures(text) {
        return (None, found[1].parse().ok());
    }
    if let Some(found) = K.captures(text) {
        return (None, found[1].parse::<u32>().ok().map(|k| k * 540));
    }
    (None, None)
}

/// Upstream `parse_bitrate`: `NNN kbps`.
fn parse_bitrate(text: &str) -> Option<f64> {
    static PATTERN: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"\b(\d+)\s*kbps").unwrap());
    PATTERN.captures(text)?[1].parse().ok()
}

/// Upstream `<source>` width/height/res with `label`/`title` resolution and
/// bitrate fallbacks.
fn source_metadata(source: &scraper::node::Element) -> (Option<u32>, Option<u32>, Option<f64>) {
    let labels: Vec<&str> = ["label", "title"]
        .into_iter()
        .filter_map(|name| source.attr(name))
        .filter(|label| !label.is_empty())
        .collect();
    let mut width = dimension(source.attr("width"));
    // `int_or_none(height) or int_or_none(res)`: zero counts as missing.
    let mut height = dimension(source.attr("height"))
        .filter(|height| *height != 0)
        .or_else(|| dimension(source.attr("res")));
    let missing = |value: Option<u32>| value.is_none_or(|value| value == 0);
    if missing(width) || missing(height) {
        for label in &labels {
            let (label_width, label_height) = parse_resolution(label);
            if label_width.is_none() && label_height.is_none() {
                continue;
            }
            if missing(width) {
                width = label_width.or(width);
            }
            if missing(height) {
                height = label_height.or(height);
            }
        }
    }
    let tbr = labels.iter().find_map(|label| parse_bitrate(label));
    (width, height, tbr)
}

fn unsupported(reason: &str) -> Error {
    Error::UnsupportedHtml5(reason.into())
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("static HTML selector")
}

/// `(?:[a-zA-Z][a-zA-Z0-9+-.]*:)?//` at the start (upstream `urljoin` keeps these).
fn scheme_relative(value: &str) -> bool {
    let rest = match value.find(':') {
        Some(colon)
            if value[..colon]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
                && value[..colon]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || ('+'..='.').contains(&c)) =>
        {
            &value[colon + 1..]
        }
        _ => value,
    };
    rest.starts_with("//")
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn parse_page(page_url: &Url, html: &str) -> Result<Option<Html5Page>> {
    let document = Html::parse_document(html);
    let base = document
        .select(&selector("base[href]"))
        .next()
        .map(|base| page_url.join(base.value().attr("href").unwrap_or("")))
        .transpose()?
        .unwrap_or_else(|| page_url.clone());
    let mut entries = Vec::new();
    for element in document.select(&selector("video, audio")) {
        // Validate every alternative, including unselected manifests.
        let mut sources = Vec::new();
        let vcodec = (element.value().name() == "audio").then(|| "none".to_owned());
        if let Some(src) = nonempty(element.value().attr("src")) {
            // Media element dimensions describe the player, not its source.
            sources.push((
                src,
                element.value().attr("type"),
                None,
                None,
                None,
                false,
                None,
            ));
        }
        for child in element.children().filter_map(scraper::ElementRef::wrap) {
            if child.value().name() == "source"
                && let Some(src) = nonempty(child.value().attr("src"))
            {
                let (width, height, tbr) = source_metadata(child.value());
                sources.push((
                    src,
                    child.value().attr("type"),
                    width,
                    height,
                    tbr,
                    true,
                    // `s_attr.get('label') or s_attr.get('title')`.
                    child
                        .value()
                        .attr("label")
                        .filter(|label| !label.is_empty())
                        .or_else(|| child.value().attr("title").filter(|t| !t.is_empty()))
                        .map(str::to_owned),
                ));
            }
        }
        if sources.is_empty() {
            continue;
        }
        let mut urls = Vec::new();
        for (source, content_type, width, height, tbr, child, label) in sources {
            let url = base.join(source)?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err(Error::UnsupportedScheme(url.scheme().into()));
            }
            let hint = content_type.unwrap_or("").to_ascii_lowercase();
            let codecs = content_type
                .filter(|_| child)
                .and_then(content_type_codecs)
                .and_then(|codecs| crate::codecs::parse(Some(codecs)));
            urls.push(Html5Source {
                url,
                width,
                height,
                vcodec: vcodec.clone(),
                codecs,
                label,
                tbr,
                element: child,
                hls_hint: hint.contains("mpegurl"),
                dash_hint: hint.contains("dash+xml"),
            });
        }
        let mut subtitles = Subtitles::default();
        let mut language_indices = HashMap::<Option<String>, usize>::new();
        for track in element.children().filter_map(scraper::ElementRef::wrap) {
            if track.value().name() != "track"
                || !matches!(
                    track.value().attr("kind"),
                    None | Some("" | "subtitles" | "captions")
                )
            {
                continue;
            }
            let Some(src) = nonempty(track.value().attr("src")) else {
                continue;
            };
            // Source postprocessing sanitizes protocol-relative subtitle URLs to
            // HTTP and repairs the httpss typo before completing extensions.
            let url = if src.starts_with("//") {
                Url::parse(&format!("http:{src}"))?
            } else if let Some(rest) = src.strip_prefix("httpss://") {
                Url::parse(&format!("https://{rest}"))?
            } else {
                base.join(src)?
            };
            if !matches!(url.scheme(), "http" | "https") {
                return Err(Error::UnsupportedScheme(url.scheme().into()));
            }
            let language = track
                .value()
                .attr("srclang")
                .filter(|lang| !lang.is_empty())
                .or_else(|| track.value().attr("lang").filter(|lang| !lang.is_empty()))
                .or_else(|| track.value().attr("label"))
                .map(str::to_owned);
            let format = SubtitleFormat {
                extra: Default::default(),
                fragments: Vec::new(),
                protocol: None,
                name: None,
                ext: subtitle_extension(&url),
                url,
            };
            if let Some(&index) = language_indices.get(&language) {
                subtitles.entries[index].1.push(format);
            } else {
                language_indices.insert(language.clone(), subtitles.entries.len());
                subtitles.entries.push((language, vec![format]));
            }
        }
        // Upstream `urljoin`: empty values are absent, scheme/protocol-relative
        // URLs are kept verbatim, and others resolve against the base.
        let thumbnail = match element.value().attr("poster") {
            None | Some("") => None,
            Some(poster) if scheme_relative(poster) => Some(poster.to_owned()),
            Some(poster) => base.join(poster).ok().map(String::from),
        };
        entries.push(Html5Entry {
            sources: urls,
            subtitles,
            thumbnail,
        });
    }
    if entries.is_empty() {
        return Ok(None);
    }
    let title = document
        .select(&selector("meta[property='og:title']"))
        .find_map(|meta| nonempty(meta.value().attr("content")))
        .map(str::to_owned)
        .or_else(|| {
            document
                .select(&selector("title"))
                .next()
                .map(|title| title.text().collect::<String>())
        });
    let title = title
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let filename = page_url
        .path()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("");
    let stem = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
    let id = percent_decode_str(stem).decode_utf8_lossy().into_owned();
    let id = if id.is_empty() { "video".into() } else { id };
    let playlist_title = if title.is_empty() { "video" } else { &title }.to_owned();
    let title = if title.is_empty() { &id } else { &title };
    Ok(Some(Html5Page {
        entries,
        title: title.to_owned(),
        playlist_title,
        id,
    }))
}

impl Extractor for GenericHttp {
    fn key(&self) -> &'static str {
        "Generic"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        self.extract_result(client, url)?
            .map(ExtractResult::into_video)
            .transpose()
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        use crate::screen::{generic_id, screen, warning};
        crate::screen::set_extractor("generic");
        // A JSON-LD `url_transparent` result's smuggled data.
        let smuggled = SMUGGLED.with(|smuggled| smuggled.borrow_mut().take());
        // Upstream `InfoExtractor.extract` and `GenericIE._real_extract` lines.
        screen(&format!(
            "[generic] Extracting URL: {}",
            crate::screen::extracting_url(
                smuggled
                    .as_ref()
                    .map_or(url.as_str(), |smuggled| smuggled.shown.as_str())
            )
        ));
        let id = smuggled.as_ref().map_or_else(
            || generic_id(url.as_str()),
            |smuggled| smuggled.video_id.clone(),
        );
        // `_request_webpage` sleeps before announcing the download.
        crate::screen::before_extraction_request();
        screen(&format!("[generic] {id}: Downloading webpage"));
        // Dispatch a single response so direct media does not need two probes.
        let referer = smuggled
            .as_ref()
            .and_then(|smuggled| smuggled.referer.as_ref());
        let response =
            direct::probe(client, url, referer).map_err(|error| webpage_error(&id, error))?;
        // Upstream's direct-link Content-Type pattern; anything else falls
        // back to sniffing the body.
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .map(|value| String::from_utf8_lossy(value.as_bytes()).to_lowercase())
            .unwrap_or_default();
        let media_type = {
            static MEDIA: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                regex::Regex::new(
                    r"^(?:audio|video|application/(?:ogg$|(?:vnd\.apple\.|x-)?mpegurl))",
                )
                .unwrap()
            });
            MEDIA.is_match(&content_type)
        };
        let fallback = || {
            warning(
                if FORCE_GENERIC.load(std::sync::atomic::Ordering::Relaxed) {
                    "[generic] Forcing generic information extractor"
                } else {
                    "[generic] Falling back on generic information extractor"
                },
            )
        };
        // Upstream `GenericIE.report_detected`.
        let detected = |name: &str| crate::screen::debug(&format!("Identified a {name}"));
        if media_type {
            detected("direct video link");
        }

        // Upstream sets the timestamp before inspecting the response.
        let timestamp = direct::last_modified(&response);
        let stamp = |mut info: VideoInfo| {
            info.metadata.timestamp = timestamp.map(crate::info::PyNumber::Int);
            if let Some(smuggled) = &smuggled {
                // `force_videoid` and `headers['Referer']` (the formats'
                // `http_headers`).
                info.id = smuggled.video_id.clone();
                if let Some(referer) = &smuggled.referer {
                    for format in &mut info.formats {
                        format.http_headers = Some(serde_json::Map::from_iter([(
                            "Referer".to_owned(),
                            referer.as_str().into(),
                        )]));
                    }
                }
            }
            ExtractResult::Video(info)
        };
        let dash_path = url.path().to_ascii_lowercase().ends_with(".mpd");
        if dash_path || dash::recognized(&response) {
            if !media_type {
                fallback();
            }
            // Read (and dumped) before `Extracting information`.
            crate::screen::expect_page(
                &id,
                None,
                Some(format!("[generic] {id}: Extracting information")),
            );
            let manifest = dash::from_response(response, url, dash_path);
            crate::screen::clear_page();
            let manifest = manifest?;
            if manifest.is_some() && !media_type {
                detected("DASH manifest");
            }
            return Ok(manifest
                .map(|mut info| {
                    crate::manifest_args::extra_manifest_info(&mut info, url);
                    info
                })
                .map(|info| crate::info_order::mark_info(info, "generic-dash"))
                .map(stamp));
        }
        let hls_path = url.path().to_ascii_lowercase().ends_with(".m3u8")
            || response
                .url()
                .path()
                .to_ascii_lowercase()
                .ends_with(".m3u8");
        // XML documents (RSS feeds) are read like pages: upstream's
        // `is_html` accepts any body starting with `<`.
        let xml = {
            let mime = direct::response_mime(&response);
            mime == "text/xml" || (mime.starts_with("application/") && mime.ends_with("xml"))
        };
        if hls_path || (direct::response_mime(&response) != "text/html" && !xml) {
            let mime = direct::response_mime(&response);
            // Upstream: a direct-link Content-Type takes `ext` from the URL,
            // then the response headers; an untyped binary response defaults
            // to `unknown_video`. Other types keep native recognition.
            let direct = if media_type {
                Some(direct::generic_direct(&response, url, None))
            } else if mime.is_empty() || mime == "application/octet-stream" {
                Some(direct::generic_untyped(&response, url))
            } else {
                direct::from_response(&response, url)
            }
            .map(|info| direct::with_format(info, &mime))
            // Without a media Content-Type, upstream returns the bare URL.
            .map(|info| {
                crate::info_order::mark_info(
                    info,
                    if media_type {
                        "generic-direct"
                    } else {
                        "generic-untyped"
                    },
                )
            });
            if !media_type {
                fallback();
            }
            crate::screen::expect_page(
                &id,
                Some(format!("[generic] {id}: Downloading m3u8 information")),
                None,
            );
            let hls = hls::from_response(client, response, url, None, hls_path, None);
            crate::screen::clear_page();
            let mut hls = hls?;
            if let Some(info) = &mut hls {
                if !media_type {
                    detected("M3U playlist");
                }
                // `_extra_manifest_info`; `is_live` replaces the check.
                match crate::manifest_args::extra_manifest_info(info, url) {
                    Some(live) => {
                        info.metadata.live_status = Some(
                            if live == "false" {
                                "not_live"
                            } else {
                                "is_live"
                            }
                            .into(),
                        );
                    }
                    None => {
                        screen(&format!("[generic] {id}: Checking m3u8 live status"));
                        crate::screen::expect_page(&id, None, None);
                        hls::check_live_status(client, info);
                        crate::screen::clear_page();
                    }
                }
            } else if direct.is_some() && !media_type {
                warning("[generic] URL could be a direct video link, returning it as such.");
            }
            let hls = hls.map(|info| {
                crate::info_order::mark_info(
                    info,
                    if media_type {
                        "generic-hls"
                    } else {
                        "generic-m3u"
                    },
                )
            });
            return Ok(hls.or(direct).map(stamp));
        }
        fallback();
        if response
            .content_length()
            .is_some_and(|size| size > MAX_HTML_BYTES)
        {
            return Err(Error::HtmlTooLarge(MAX_HTML_BYTES));
        }
        if response
            .headers()
            .get(CONTENT_ENCODING)
            .is_some_and(|value| !value.as_bytes().eq_ignore_ascii_case(b"identity"))
        {
            return Err(unsupported("compressed HTML is not supported"));
        }
        if let Some(content_type) = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
        {
            for parameter in content_type.split(';').skip(1) {
                if let Some((name, value)) = parameter.trim().split_once('=')
                    && name.trim().eq_ignore_ascii_case("charset")
                    && !matches!(
                        value
                            .trim()
                            .trim_matches(['\'', '"'])
                            .to_ascii_lowercase()
                            .as_str(),
                        "utf-8" | "utf8" | "us-ascii"
                    )
                {
                    return Err(unsupported("only UTF-8/ASCII HTML is supported"));
                }
            }
        }
        let page_url = response.url().clone();
        let mut bytes = Vec::new();
        response
            .take(MAX_HTML_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(Error::ExtractionRead)?;
        if bytes.len() as u64 > MAX_HTML_BYTES {
            return Err(Error::HtmlTooLarge(MAX_HTML_BYTES));
        }
        crate::screen::expect_page(
            &id,
            None,
            Some(format!("[generic] {id}: Extracting information")),
        );
        crate::screen::page_read(&page_url, &bytes)?;
        let html = std::str::from_utf8(&bytes)
            .map_err(|_| unsupported("only UTF-8/ASCII HTML is supported"))?;
        // `compat_etree_fromstring(webpage)`: an RSS feed.
        if let Some(feed) = super::rss::parse(html) {
            detected("RSS feed");
            return Ok(Some(rss_playlist(feed, url, &page_url)));
        }
        // `SmoothStreamingMedia`: the generic info dict with
        // `_parse_ism_formats_and_subtitles`.
        if let Some(parsed) = super::ism::parse(html, url, None) {
            let parsed = parsed?;
            detected("ISM manifest");
            let mut info = direct::build_at(page_url.clone(), &page_url, String::new());
            info.formats = parsed.formats;
            info.subtitles = parsed.subtitles;
            info.direct = None;
            info.id = id.clone();
            info.webpage_url = page_url.clone();
            info.metadata.timestamp = timestamp.map(crate::info::PyNumber::Int);
            if info.formats.is_empty() {
                return Err(Error::Reported(format!(
                    "[generic] {id}: No video formats found!; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            return Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
                info,
                "generic-ism",
            ))));
        }
        // `{http://ns.adobe.com/f4m/[12].0}manifest`: the generic info dict
        // with `_parse_f4m_formats`.
        if let Some(manifest) = super::f4m::parse(html, url.as_str(), None) {
            let super::f4m::Manifest::Formats(formats) = manifest else {
                return Err(unsupported(
                    "F4M manifests referring to nested manifests are not supported natively",
                ));
            };
            let formats = formats
                .into_iter()
                .map(|format| {
                    let ext = format["ext"]
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            direct::determine_ext(format["url"].as_str().unwrap_or_default())
                                .unwrap_or_else(|| "unknown_video".into())
                        });
                    let mut value = format;
                    value["ext"] = ext.into();
                    let mut format: VideoFormat = serde_json::from_value(value)
                        .map_err(|error| unsupported(&format!("F4M media: {error}")))?;
                    format.protocol = Some("f4m".into());
                    crate::info_order::mark_format(&mut format, "f4m");
                    Ok(format)
                })
                .collect::<Result<Vec<_>>>()?;
            detected("F4M manifest");
            let mut info = direct::build_at(page_url.clone(), &page_url, String::new());
            info.formats = formats;
            info.direct = None;
            info.id = id.clone();
            info.webpage_url = page_url.clone();
            info.metadata.timestamp = timestamp.map(crate::info::PyNumber::Int);
            if info.formats.is_empty() {
                return Err(Error::Reported(format!(
                    "[generic] {id}: No video formats found!; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            return Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
                info,
                "generic-f4m",
            ))));
        }
        if let Some(smil) = super::smil::parse(html) {
            let info = smil_info(client, smil, url, &crate::screen::generic_id(url.as_str()))?;
            detected("SMIL file");
            return Ok(Some(ExtractResult::Video(info)));
        }
        if let Some(tracks) = super::xspf::parse(html, &page_url) {
            detected("XSPF playlist");
            return xspf_playlist(tracks, url, &page_url).map(Some);
        }
        crate::screen::debug("Looking for embeds");
        // Other extractors' `_EMBED_REGEX` iframes, in extractor order.
        let embeds = embeds(url, html);
        let Some(media) = parse_page(&page_url, html)? else {
            if !embeds.is_empty() {
                return self.embed_results(client, url, &page_url, html, timestamp, embeds);
            }
            return self.found_urls(client, url, &page_url, html, timestamp);
        };
        if !embeds.is_empty() {
            return Err(unsupported(
                "pages with both HTML5 media and other extractors' embeds are not supported natively",
            ));
        }
        let mut entries = Vec::new();
        for (index, entry) in media.entries.iter().enumerate() {
            let mut formats = Vec::new();
            let mut first_info = None;
            for (index, source) in entry.sources.iter().enumerate() {
                let url = &source.url;
                // Upstream requests only manifest sources it recognizes by
                // type or extension; other sources are probed natively.
                let manifest_hint = source.hls_hint
                    || source.dash_hint
                    || [".m3u8", ".mpd"]
                        .iter()
                        .any(|suffix| url.path().to_ascii_lowercase().ends_with(suffix));
                let response = if manifest_hint {
                    direct::request(client, url, Some(&page_url))?
                } else {
                    direct::probe(client, url, Some(&page_url))?
                };
                let direct = direct::from_response(&response, url);
                let dash_hint =
                    source.dash_hint || url.path().to_ascii_lowercase().ends_with(".mpd");
                // The request note precedes the manifest's page dump.
                let manifest = if dash_hint || dash::recognized(&response) {
                    crate::screen::expect_page(
                        &id,
                        Some(format!("[html5] {id}: Downloading MPD manifest")),
                        None,
                    );
                    let manifest = dash::from_response(response, url, dash_hint);
                    crate::screen::clear_page();
                    manifest?
                } else {
                    crate::screen::expect_page(
                        &id,
                        Some(format!("[html5] {id}: Downloading m3u8 information")),
                        None,
                    );
                    let manifest = hls::from_response(
                        client,
                        response,
                        url,
                        Some(&page_url),
                        source.hls_hint || url.path().to_ascii_lowercase().ends_with(".m3u8"),
                        Some("hls"),
                    );
                    crate::screen::clear_page();
                    manifest?
                };
                let info = manifest
                    .or(direct)
                    .ok_or_else(|| unsupported("source does not serve supported HTTP media"))?;
                if !info.formats.is_empty() {
                    // Manifest IDs stay as upstream names them (`hls-…`, bare DASH
                    // representation IDs); duplicates are numbered after extraction.
                    formats.extend(info.formats.iter().cloned().map(|mut format| {
                        format.audio_group =
                            format.audio_group.map(|group| format!("{index}-{group}"));
                        // The page's Referer is added after the manifest's keys.
                        if let Some(serde_json::Value::String(shape)) =
                            format.extra.get(crate::info_order::FORMAT_SHAPE)
                        {
                            let shape = format!("html5+{shape}");
                            crate::info_order::mark_format(&mut format, &shape);
                        }
                        format
                    }));
                    first_info.get_or_insert(info);
                    continue;
                }
                // `f.update(formats[0])` keeps the source's acodec/dynamic_range
                // but replaces vcodec.
                let codecs = source.codecs.as_ref();
                formats.push(VideoFormat {
                    http_headers: None,
                    extra: Default::default(),
                    protocol: None,
                    vcodec: source.vcodec.clone(),
                    acodec: codecs.map(|codecs| codecs.acodec.clone()),
                    dynamic_range: codecs
                        .and_then(|codecs| codecs.dynamic_range)
                        .map(str::to_owned),
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
                    format_id: source.label.clone().unwrap_or_default(),
                    stream_kind: info.stream_kind,
                    audio_group: None,
                    language: None,
                    format_note: None,
                    audio_default: false,
                    audio_autoselect: false,
                    downloader_options: None,
                    manifest_stream_number: None,
                    format_index: None,
                    container: None,
                    abr: None,
                    tbr: source.tbr,
                    url: info.url.clone(),
                    ext: info.ext.clone(),
                    filesize: info.filesize,
                    fragments: info.fragments.clone(),
                    width: source.width,
                    height: source.height,
                });
                if let Some(format) = formats.last_mut() {
                    let shape = if source.element {
                        "html5-source"
                    } else {
                        "html5-src"
                    };
                    crate::info_order::mark_format(format, shape);
                }
                first_info.get_or_insert(info);
            }
            let mut info = first_info.expect("parsed HTML5 media has at least one source");
            info.formats = formats;
            info.subtitles = entry.subtitles.clone();
            info.metadata.thumbnail = entry.thumbnail.clone();
            info.id = format!("{}-{}", media.id, index + 1);
            info.title = format!("{} ({})", media.title, index + 1);
            info.webpage_url = page_url.clone();
            info.direct = None;
            info.extractor = "html5".into();
            info.extractor_key = "HTML5MediaEmbed".into();
            entries.push(crate::info_order::mark_info(info, "html5-entry"));
        }
        // `HTML5MediaEmbedIE`: archive IDs from before the embed extractor.
        let single = entries.len() == 1;
        for entry in &mut entries {
            let old = if single {
                media.id.as_str()
            } else {
                entry.id.as_str()
            };
            entry.extra.insert(
                "_old_archive_ids".into(),
                serde_json::json!([format!("generic {old}")]),
            );
        }
        crate::screen::debug(&match entries.len() {
            1 => "Identified a html5 embed".to_owned(),
            count => format!("Identified {count} html5 embeds"),
        });
        if entries.len() == 1 {
            // `merge_dicts(embed, info_dict)`: page fields fill missing values.
            let mut info = crate::info_order::mark_info(entries.pop().expect("one entry"), "html5");
            info.metadata.timestamp = info
                .metadata
                .timestamp
                .or(timestamp.map(crate::info::PyNumber::Int));
            info.metadata.description = crate::html::og_search_property(html, "description");
            if info.metadata.thumbnail.is_none() {
                info.metadata.thumbnail = crate::html::og_search_property(html, "image");
            }
            info.metadata.age_limit = crate::html::rta_search(html);
            return Ok(Some(ExtractResult::Video(info)));
        }
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: media.id,
            title: media.playlist_title,
            webpage_url: page_url,
            extractor: "generic".into(),
            extractor_key: "Generic".into(),
            entries,
            extra: {
                // The page's Last-Modified, with `_fill_common_fields`'s date.
                let mut extra = serde_json::Map::new();
                extra.insert(
                    crate::info_order::INFO_SHAPE.to_owned(),
                    "html5-playlist".into(),
                );
                if let Some(timestamp) = timestamp {
                    extra.insert("timestamp".into(), timestamp.into());
                    if let Some(date) = crate::dates::strftime_date(timestamp) {
                        extra.insert("upload_date".into(), date.into());
                    }
                }
                extra
            },
        })))
    }
}

impl GenericHttp {
    /// `_extract_embeds`' result for other extractors' embeds: one is
    /// followed as a `url` result (the page fields are dropped), several
    /// are `url` entries of the page's playlist that
    /// [`crate::YoutubeDl::resolve_entry`] extracts when processed.
    fn embed_results(
        &self,
        client: &Client,
        url: &Url,
        page_url: &Url,
        html: &str,
        timestamp: Option<i64>,
        embeds: Vec<(&'static EmbedSite, Url)>,
    ) -> Result<Option<ExtractResult>> {
        if let [(site, embed)] = embeds.as_slice() {
            if let Some(extractor) = site.extractor {
                return extractor.extract_result(client, embed);
            }
            // `url_result(embed)`: followed by the downloader.
            let mut info = direct::build_at(embed.clone(), embed, String::new());
            info.formats.clear();
            info.direct = None;
            info.extra
                .insert(crate::FOLLOW_URL.into(), embed.as_str().into());
            return Ok(Some(ExtractResult::Video(info)));
        }
        let entries = embeds
            .into_iter()
            .map(|(site, embed)| {
                let mut info = direct::build_at(embed.clone(), &embed, String::new());
                info.formats.clear();
                info.direct = None;
                info.id = (site.temp_id)(embed.as_str()).unwrap_or_default();
                info.extra.insert(
                    crate::TRANSPARENT.into(),
                    serde_json::json!({
                        "url": embed.as_str(),
                        "force_videoid": null,
                        "fields": {},
                        "ie_key": site.key,
                        "extractor": site.name,
                    }),
                );
                info
            })
            .collect();
        // `playlist_result(entries, **info_dict)`.
        let mut extra = serde_json::Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.to_owned(),
            "html5-playlist".into(),
        );
        if let Some(timestamp) = timestamp {
            extra.insert("timestamp".into(), timestamp.into());
            if let Some(date) = crate::dates::strftime_date(timestamp) {
                extra.insert("upload_date".into(), date.into());
            }
        }
        for (key, value) in [
            (
                "description",
                crate::html::og_search_property(html, "description").map(serde_json::Value::from),
            ),
            (
                "thumbnail",
                crate::html::og_search_property(html, "image").map(serde_json::Value::from),
            ),
            (
                "age_limit",
                crate::html::rta_search(html).map(serde_json::Value::from),
            ),
        ] {
            if let Some(value) = value {
                extra.insert(key.into(), value);
            }
        }
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: crate::screen::generic_id(url.as_str()),
            title: page_title(html).unwrap_or_else(|| "video".into()),
            webpage_url: page_url.clone(),
            extractor: "generic".into(),
            extractor_key: "Generic".into(),
            entries,
            extra,
        })))
    }

    /// `_extract_embeds`' URL patterns for a page without HTML5 media:
    /// direct media URLs as entries (`merge_dicts(entry, info_dict)` for
    /// one), or a refresh/`twitter:player` redirect.
    fn found_urls(
        &self,
        client: &Client,
        url: &Url,
        page_url: &Url,
        html: &str,
        timestamp: Option<i64>,
    ) -> Result<Option<ExtractResult>> {
        use super::generic_urls::{Found, find};
        // `_find_jwplayer_data`, then `_parse_jwplayer_data` with formats.
        if let Some(data) = super::jwplayer::find_data(html) {
            if data
                .get("playlist")
                .is_some_and(serde_json::Value::is_string)
            {
                return Err(unsupported(
                    "JW Player playlist URLs are not supported natively",
                ));
            }
            let video_id = crate::screen::generic_id(url.as_str());
            if let Some(entry) = super::jwplayer::parse(&data, &video_id, url)?
                && let Some(result) =
                    jwplayer_info(client, entry, page_url, html, timestamp, &video_id)?
            {
                return Ok(Some(result));
            }
        }
        if let Some(embed) = super::generic_urls::videojs(html, url, page_url)?
            && let Some(result) = videojs_info(client, embed, url, page_url, html, timestamp)?
        {
            return Ok(Some(result));
        }
        // The KVS player (before JSON-LD).
        if let Some(version) = super::kvs::player_version(html) {
            crate::screen::debug("Identified a KVS Player");
            if !matches!(version.split('.').next(), Some("4" | "5" | "6")) {
                crate::screen::warning(&format!(
                    "[generic] Untested major version ({version}) in player engine - download may fail."
                ));
            }
            return kvs_info(url, page_url, html, timestamp).map(Some);
        }
        // `_search_json_ld`: a media URL other than the page's.
        let json_ld = super::json_ld::search(html);
        if let Some(target) = json_ld
            .get("url")
            .and_then(serde_json::Value::as_str)
            .filter(|target| *target != url.as_str())
            .map(str::to_owned)
        {
            crate::screen::debug("Identified a JSON LD");
            return self.json_ld_result(client, url, page_url, html, timestamp, &target, json_ld);
        }
        let urls = match find(url, html) {
            None => return Ok(None),
            Some(Found::Redirect(target)) => {
                crate::screen::screen(&format!("[redirect] Following redirect to {target}"));
                if super::youtube_suitable(target.as_str()) {
                    return Err(unsupported(
                        "redirects to YouTube are not supported natively",
                    ));
                }
                return self.extract_result(client, &target);
            }
            Some(Found::Urls(name, urls)) => {
                // `report_detected(name)` (no count).
                crate::screen::debug(&format!("Identified a {name}"));
                urls
            }
        };
        // `_generic_title('', webpage, default='video')` and the page fields.
        let title = page_title(html).unwrap_or_else(|| "video".into());
        let description = crate::html::og_search_property(html, "description");
        let thumbnail = crate::html::og_search_property(html, "image");
        let age_limit = crate::html::rta_search(html);
        static DOMAIN: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new(r"^(?:https?://)?([^/]*)/").unwrap());
        let uploader = DOMAIN
            .captures(url.as_str())
            .map(|found| found[1].to_owned());
        let count = urls.len();
        let mut entries = Vec::new();
        for (index, media) in urls.into_iter().enumerate() {
            if super::youtube_suitable(media.as_str()) {
                return Err(unsupported(
                    "embedded YouTube URLs are not supported natively",
                ));
            }
            if media.scheme().starts_with("rtmp") {
                return Err(unsupported("RTMP URLs are not supported natively"));
            }
            let ext = direct::determine_ext(media.as_str());
            let manifest = matches!(ext.as_deref(), Some("m3u8" | "mpd"));
            let smil = ext.as_deref() == Some("smil");
            if smil && count > 1 {
                return Err(unsupported(
                    "several SMIL URLs found on a page are not supported natively",
                ));
            }
            if !manifest
                && !smil
                && matches!(
                    ext.as_deref(),
                    Some("smil" | "xspf" | "f4m" | "m3u8" | "mpd")
                )
            {
                return Err(unsupported(&format!(
                    "{} URLs found on a page are not supported natively",
                    ext.unwrap_or_default()
                )));
            }
            // `os.path.splitext(unquote(basename(video_url)))[0]`.
            let basename = media.as_str().rsplit('/').next().unwrap_or("");
            let basename = percent_decode_str(basename).decode_utf8_lossy();
            let id = match basename.rfind('.') {
                Some(dot) if !basename[..dot].trim_start_matches('.').is_empty() => {
                    basename[..dot].to_owned()
                }
                _ => basename.into_owned(),
            };
            if smil {
                // `{**self._extract_smil_info(video_url, video_id),
                // **entry_info_dict}`, then `merge_dicts(entry, info_dict)`
                // (which drops the entry's `None` values).
                let (document, smil_url) = download_smil(client, &media, &id)?;
                let mut info = smil_info(client, document, &smil_url, &id)?;
                let own_description = info.metadata.description.is_some();
                info.id = id;
                info.title = title.clone();
                info.webpage_url = page_url.clone();
                info.extractor = "generic".into();
                info.extractor_key = "Generic".into();
                if let Some(uploader) = &uploader {
                    info.extra
                        .insert("uploader".into(), uploader.clone().into());
                }
                info.metadata.age_limit = age_limit;
                info.metadata.timestamp = timestamp.map(crate::info::PyNumber::Int);
                if !own_description {
                    info.metadata.description = description.clone();
                }
                info.metadata.thumbnail = thumbnail.clone();
                for format in &mut info.formats {
                    format.http_headers = Some(serde_json::Map::from_iter([(
                        "referer".to_owned(),
                        page_url.as_str().into(),
                    )]));
                }
                info.extra.insert(
                    "http_headers".into(),
                    serde_json::json!({"Referer": page_url.as_str()}),
                );
                let shape = if own_description {
                    "generic-found-smil"
                } else {
                    "generic-found-smil-page-description"
                };
                entries.push(crate::info_order::mark_info(info, shape));
                continue;
            }
            if manifest {
                let info =
                    found_manifest(client, &media, page_url, &id, ext.as_deref() == Some("mpd"))?;
                let mut info = info;
                info.id = id;
                info.title = if count > 1 {
                    format!("{title} ({})", index + 1)
                } else {
                    title.clone()
                };
                info.webpage_url = page_url.clone();
                info.direct = None;
                info.extractor = "generic".into();
                info.extractor_key = "Generic".into();
                if let Some(uploader) = &uploader {
                    info.extra
                        .insert("uploader".into(), uploader.clone().into());
                }
                info.metadata.age_limit = age_limit;
                if count == 1 {
                    info.metadata.timestamp = timestamp.map(crate::info::PyNumber::Int);
                    info.metadata.description = description.clone();
                    info.metadata.thumbnail = thumbnail.clone();
                }
                for format in &mut info.formats {
                    format.http_headers = Some(serde_json::Map::from_iter([(
                        "referer".to_owned(),
                        page_url.as_str().into(),
                    )]));
                }
                // The entry's own `http_headers` (`HTTPHeaderDict` title case).
                info.extra.insert(
                    "http_headers".into(),
                    serde_json::json!({"Referer": page_url.as_str()}),
                );
                let dash = ext.as_deref() == Some("mpd");
                let shape = match (dash, count == 1) {
                    (true, true) => "generic-found-dash",
                    (true, false) => "generic-found-dash-entry",
                    (false, true) => "generic-found-hls",
                    (false, false) => "generic-found-hls-entry",
                };
                entries.push(crate::info_order::mark_info(info, shape));
                continue;
            }
            let ext = ext
                .filter(|ext| crate::output::allowed_extension(ext))
                .unwrap_or_else(|| "unknown_video".into());
            let mut info = direct::with_format(direct::build_at(media.clone(), &media, ext), "");
            info.id = id;
            info.title = if count > 1 {
                format!("{title} ({})", index + 1)
            } else {
                title.clone()
            };
            info.webpage_url = page_url.clone();
            info.direct = None;
            if let Some(uploader) = &uploader {
                info.extra
                    .insert("uploader".into(), uploader.clone().into());
            }
            info.metadata.age_limit = age_limit;
            if count == 1 {
                info.metadata.timestamp = timestamp.map(crate::info::PyNumber::Int);
                info.metadata.description = description.clone();
                info.metadata.thumbnail = thumbnail.clone();
            }
            // `headers = {'referer': actual_url}`.
            for format in &mut info.formats {
                format.http_headers = Some(serde_json::Map::from_iter([(
                    "referer".to_owned(),
                    page_url.as_str().into(),
                )]));
            }
            entries.push(crate::info_order::mark_info(
                info,
                if count == 1 {
                    "generic-found"
                } else {
                    "generic-found-entry"
                },
            ));
        }
        if entries.len() == 1 {
            return Ok(entries.pop().map(ExtractResult::Video));
        }
        // `playlist_result(entries, **info_dict)`.
        let mut extra = serde_json::Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.to_owned(),
            "html5-playlist".into(),
        );
        if let Some(timestamp) = timestamp {
            extra.insert("timestamp".into(), timestamp.into());
            if let Some(date) = crate::dates::strftime_date(timestamp) {
                extra.insert("upload_date".into(), date.into());
            }
        }
        for (key, value) in [
            ("description", description.map(serde_json::Value::from)),
            ("thumbnail", thumbnail.map(serde_json::Value::from)),
            ("age_limit", age_limit.map(serde_json::Value::from)),
        ] {
            if let Some(value) = value {
                extra.insert(key.into(), value);
            }
        }
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: crate::screen::generic_id(url.as_str()),
            title,
            webpage_url: page_url.clone(),
            extractor: "generic".into(),
            extractor_key: "Generic".into(),
            entries,
            extra,
        })))
    }
}

/// `_download_smil(smil_url, video_id)`: the parsed document and its final
/// URL.
fn download_smil(client: &Client, url: &Url, video_id: &str) -> Result<(super::smil::Smil, Url)> {
    use std::io::Read;
    crate::screen::screen(&format!("[generic] {video_id}: Downloading SMIL file"));
    let response = direct::request(client, url, None).map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor: "generic",
            id: video_id.to_owned(),
            what: "SMIL file",
            source: Box::new(error),
        },
        error => error,
    })?;
    let final_url = response.url().clone();
    let mut body = Vec::new();
    response
        .take(MAX_HTML_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let text = std::str::from_utf8(&body)
        .map_err(|_| unsupported("only UTF-8 SMIL files are supported"))?;
    let smil = super::smil::parse(text)
        .ok_or_else(|| unsupported("the found SMIL URL does not serve a SMIL document"))?;
    Ok((smil, final_url))
}

/// `_extract_kvs(url, webpage, video_id)` merged with the page fields
/// (`merge_dicts` drops its `None` values).
fn kvs_info(
    url: &Url,
    page_url: &Url,
    html: &str,
    timestamp: Option<i64>,
) -> Result<ExtractResult> {
    use serde_json::Value;
    static DISPLAY_ID: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(
            r#"(?:<link href="https?://[^"]+/(.+?)/?" rel="canonical"\s*/?>|<link rel="canonical" href="https?://[^"]+/(.+?)/?"\s*/?>)"#,
        )
        .expect("static pattern")
    });
    static TITLE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"<(?:h1|title)>(?:Video: )?(.+?)</(?:h1|title)>")
            .expect("static pattern")
    });
    let report =
        |message: String| Error::Reported(format!("{message}; {}", crate::BUG_REPORTS_MESSAGE));
    let key_error = |key: &str| {
        report(format!(
            "An extractor error has occurred. (caused by KeyError('{key}'))"
        ))
    };
    let flashvars = match super::kvs::flashvars(html) {
        None => return Err(report("[generic] Unable to extract flashvars".into())),
        Some(None) => {
            return Err(unsupported(
                "KVS flashvars that do not parse as a JSON object are not supported natively",
            ));
        }
        Some(Some(flashvars)) => flashvars,
    };
    let display_id = DISPLAY_ID.captures(html).and_then(|found| {
        found
            .iter()
            .skip(1)
            .flatten()
            .next()
            .map(|found| found.as_str().to_owned())
    });
    if display_id.is_none() {
        super::search_warning("generic", "display_id");
    }
    let title = TITLE
        .captures(html)
        .map(|found| crate::html::clean_html(&found[1]))
        .ok_or_else(|| report("[generic] Unable to extract title".into()))?;
    let text = |key: &str| -> Result<String> {
        match flashvars.get(key) {
            Some(Value::String(text)) => Ok(text.clone()),
            Some(_) => Err(unsupported(&format!(
                "non-string KVS {key} values are not supported natively"
            ))),
            None => Err(key_error(key)),
        }
    };
    let mut thumbnail = text("preview_url")?;
    if thumbnail.starts_with("//") {
        // `url.partition('/')[0]`: the scheme with its colon.
        let scheme = url.as_str().split('/').next().unwrap_or_default();
        thumbnail = format!("{scheme}{thumbnail}");
    }
    let thumbnail = url
        .join(&thumbnail)
        .map_err(|_| unsupported("KVS preview URLs must be joinable"))?;
    static URL_KEY: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"^video_(?:url|alt_url\d*)$").expect("static pattern")
    });
    let mut formats = Vec::new();
    for key in flashvars.keys().filter(|key| URL_KEY.is_match(key)) {
        let video_url = text(key)?;
        if !video_url.contains("/get_file/") {
            continue;
        }
        let format_id = match flashvars.get(&format!("{key}_text")) {
            Some(Value::String(text)) => text.clone(),
            Some(_) => {
                return Err(unsupported(
                    "non-string KVS format labels are not supported natively",
                ));
            }
            None => key.clone(),
        };
        let real = super::kvs::real_url(&video_url, &text("license_code")?)
            .ok_or_else(|| unsupported("KVS license codes or URLs upstream cannot decode"))?;
        let media = url
            .join(&real)
            .map_err(|_| unsupported("KVS video URLs must be joinable"))?;
        // `parse_resolution(format_id) or parse_resolution(video_url)`.
        let (mut width, mut height) = parse_resolution(&format_id);
        if width.is_none() && height.is_none() {
            (width, height) = parse_resolution(&video_url);
        }
        let mut format: VideoFormat = serde_json::from_value(serde_json::json!({
            "format_id": format_id,
            "url": media.as_str(),
            "ext": "mp4",
            "width": width,
            "height": height,
        }))
        .map_err(|error| unsupported(&format!("KVS format: {error}")))?;
        format.http_headers = Some(serde_json::Map::from_iter([(
            "Referer".to_owned(),
            url.as_str().into(),
        )]));
        if height.is_none() {
            format.quality = Some(1.0);
        }
        crate::info_order::mark_format(&mut format, "kvs");
        formats.push(format);
    }
    let (id, id_is_string) = match flashvars.get("video_id") {
        Some(Value::String(id)) => (id.clone(), true),
        Some(Value::Number(number)) => (number.to_string(), false),
        Some(_) => {
            return Err(unsupported(
                "non-string KVS video IDs are not supported natively",
            ));
        }
        None => return Err(key_error("video_id")),
    };
    let mut info = direct::build_at(page_url.clone(), page_url, String::new());
    info.formats = formats;
    info.direct = None;
    info.id = id;
    info.title = title;
    info.webpage_url = page_url.clone();
    let shape = if display_id.is_some() {
        "kvs"
    } else {
        "kvs-without-display-id"
    };
    if let Some(display_id) = display_id {
        info.extra.insert("display_id".into(), display_id.into());
    }
    info.metadata.thumbnail = Some(thumbnail.to_string());
    info.metadata.timestamp = timestamp.map(crate::info::PyNumber::Int);
    info.metadata.description = crate::html::og_search_property(html, "description");
    info.metadata.age_limit = crate::html::rta_search(html);
    if !id_is_string {
        crate::screen::warning(
            "\"id\" field is not a string - forcing string conversion, there is an error in extractor",
        );
    }
    Ok(ExtractResult::Video(crate::info_order::mark_info(
        info, shape,
    )))
}

/// `_merge_subtitles(source, target=target)`: tracks append per language.
pub(super) fn merge_subtitles(target: &mut Subtitles, source: Subtitles) {
    for (language, tracks) in source.entries {
        match target.entries.iter_mut().find(|(key, _)| *key == language) {
            Some((_, existing)) => existing.extend(tracks),
            None => target.entries.push((language, tracks)),
        }
    }
}

/// GenericIE's `_parse_smil(doc, url, video_id)` for HTTP media sources;
/// other source kinds are unsupported errors.
fn smil_info(
    client: &Client,
    smil: super::smil::Smil,
    url: &Url,
    video_id: &str,
) -> Result<VideoInfo> {
    use super::smil::get;
    static ISM: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\.ism/[Mm]anifest").expect("static pattern")
    });
    let int = |value: Option<&str>| value.and_then(|value| value.trim().parse::<u32>().ok());
    // `_parse_smil_formats_and_subtitles`.
    let base = smil
        .meta
        .iter()
        .find_map(|meta| {
            get(meta, "base")
                .filter(|base| !base.is_empty())
                .or_else(|| get(meta, "httpBase").filter(|base| !base.is_empty()))
        })
        .unwrap_or(url.as_str())
        .to_owned();
    if !smil.imagestreams.is_empty() {
        return Err(unsupported(
            "SMIL imagestream storyboards are not supported natively",
        ));
    }
    let mut formats = Vec::new();
    let mut sources: Vec<String> = Vec::new();
    let mut http_count = 0;
    let mut m3u8_count = 0;
    let mut subtitles = Subtitles::default();
    for medium in &smil.media {
        let Some(src) = get(medium, "src").filter(|src| !src.is_empty()) else {
            continue;
        };
        if sources.iter().any(|seen| seen == src) {
            continue;
        }
        sources.push(src.to_owned());
        let float = |key: &str, alternative: &str| {
            get(medium, key)
                .filter(|value| !value.is_empty())
                .or_else(|| get(medium, alternative))
                .and_then(|value| value.trim().parse::<f64>().ok())
        };
        let bitrate = float("system-bitrate", "systemBitrate").map(|bitrate| bitrate / 1000.0);
        let filesize = get(medium, "size")
            .filter(|value| !value.is_empty())
            .or_else(|| get(medium, "fileSize"))
            .and_then(|value| value.trim().parse::<u64>().ok());
        let width = int(get(medium, "width"));
        let height = int(get(medium, "height"));
        let proto = get(medium, "proto");
        let ext = get(medium, "ext").filter(|ext| !ext.is_empty());
        let streamer = get(medium, "streamer")
            .filter(|value| !value.is_empty())
            .unwrap_or(&base);
        if proto == Some("rtmp") || streamer.starts_with("rtmp") {
            return Err(unsupported("SMIL RTMP sources are not supported natively"));
        }
        let Some(src_ext) = direct::determine_ext(src).or(ext.map(str::to_owned)) else {
            return Err(unsupported(
                "SMIL sources without an extension are not supported natively",
            ));
        };
        let src_url = if src.starts_with("http") {
            src.to_owned()
        } else {
            Url::parse(&format!("{base}/"))
                .and_then(|base| base.join(src))
                .map(String::from)
                .map_err(|_| unsupported("SMIL source URLs must be absolute or joinable"))?
        };
        let src_url = src_url.trim().to_owned();
        let hls = proto == Some("m3u8") || src_ext == "m3u8";
        if hls || src_ext == "mpd" {
            // `_extract_m3u8_formats_and_subtitles(src_url, video_id, ext or
            // 'mp4', m3u8_id='hls', fatal=False)` or the `mpd_id='dash'` MPD.
            let manifest_url = Url::parse(&src_url)?;
            let (note, errnote) = if hls {
                (
                    "Downloading m3u8 information",
                    "Failed to download m3u8 information",
                )
            } else {
                (
                    "Downloading MPD manifest",
                    "Failed to download MPD manifest",
                )
            };
            let note = format!("[generic] {video_id}: {note}");
            crate::screen::expect_page(video_id, Some(note.clone()), None);
            let response = direct::request(client, &manifest_url, None);
            if response.is_err() {
                // The note precedes a failed request too.
                crate::screen::screen(&note);
            }
            let parsed = response.and_then(|response| {
                if hls {
                    hls::from_response(client, response, &manifest_url, None, true, Some("hls"))
                } else {
                    dash::from_response(response, &manifest_url, true)
                }
            });
            crate::screen::clear_page();
            let manifest = match parsed {
                Ok(manifest) => manifest,
                Err(error) => {
                    crate::screen::warning(&format!(
                        "[generic] {video_id}: {errnote}: {}",
                        crate::upstream_error_text(&error)
                    ));
                    None
                }
            };
            let Some(manifest) = manifest else {
                continue;
            };
            let mut found: Vec<VideoFormat> = manifest.formats;
            for format in &mut found {
                if hls {
                    if let Some(ext) = ext {
                        format.ext = ext.to_owned();
                    }
                } else {
                    format.format_id = format!("dash-{}", format.format_id);
                }
            }
            if hls && found.len() == 1 {
                m3u8_count += 1;
                let format = &mut found[0];
                format.format_id = match bitrate {
                    Some(bitrate) => format!("hls-{}", bitrate.trunc() as i64),
                    None => format!("hls-{m3u8_count}"),
                };
                format.tbr = bitrate;
                format.width = width;
                format.height = height;
                if let Some(serde_json::Value::String(shape)) =
                    format.extra.get(crate::info_order::FORMAT_SHAPE)
                {
                    let shape = format!("{shape}|update:tbr,width,height");
                    crate::info_order::mark_format(format, &shape);
                }
            }
            formats.extend(found);
            merge_subtitles(&mut subtitles, manifest.subtitles);
            continue;
        }
        if src_ext == "f4m" || ISM.is_match(&src_url) {
            return Err(unsupported(
                "SMIL F4M and ISM sources are not supported natively",
            ));
        }
        if !src_url.starts_with("http") {
            continue;
        }
        // `_is_valid_url(src, video_id)`: absolute sources are requested.
        let checked = crate::extractor::proto_relative_http(src);
        if checked.starts_with("http://") || checked.starts_with("https://") {
            crate::screen::screen(&format!("[generic] {video_id}: Checking video URL"));
            let target = Url::parse(&checked)?;
            if let Err(error) = direct::request(client, &target, None) {
                crate::screen::screen(&format!(
                    "[generic] {video_id}: video URL is invalid, skipping: {}",
                    crate::upstream_error_text(&error)
                ));
                continue;
            }
        }
        http_count += 1;
        let format_id = match bitrate.filter(|bitrate| *bitrate != 0.0) {
            Some(bitrate) => format!("http-{}", bitrate.trunc() as i64),
            None => format!("http-{http_count}"),
        };
        let mut format: VideoFormat = serde_json::from_value(serde_json::json!({
            "format_id": format_id,
            "url": src_url,
            "ext": ext.map(str::to_owned).unwrap_or(src_ext),
            "tbr": bitrate,
            "filesize": filesize,
            "width": width,
            "height": height,
        }))
        .map_err(|error| unsupported(&format!("SMIL source: {error}")))?;
        crate::info_order::mark_format(&mut format, "smil-http");
        formats.push(format);
    }
    // `_parse_smil_subtitles(smil, namespace)`, merged after the manifests'.
    let mut smil_subtitles = Subtitles::default();
    let mut subtitle_urls: Vec<String> = Vec::new();
    for stream in &smil.textstreams {
        let Some(src) = get(stream, "src").filter(|src| !src.is_empty()) else {
            continue;
        };
        if subtitle_urls.iter().any(|seen| seen == src) {
            continue;
        }
        subtitle_urls.push(src.to_owned());
        let ext = get(stream, "ext")
            .filter(|ext| !ext.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                get(stream, "type")
                    .map(|mime| direct::mimetype2ext_or(mime, None))
                    .filter(|ext| !ext.is_empty())
            })
            .or_else(|| direct::determine_ext(src))
            .unwrap_or_else(|| "unknown_video".into());
        let language = ["systemLanguage", "systemLanguageName", "lang"]
            .iter()
            .find_map(|key| get(stream, key).filter(|value| !value.is_empty()))
            .unwrap_or("en")
            .to_owned();
        let track = SubtitleFormat {
            name: None,
            url: Url::parse(src).map_err(|_| {
                unsupported("relative SMIL text stream URLs are not supported natively")
            })?,
            ext,
            protocol: None,
            fragments: Vec::new(),
            extra: Default::default(),
        };
        merge_subtitles(
            &mut smil_subtitles,
            Subtitles {
                entries: vec![(Some(language), vec![track])],
            },
        );
    }
    merge_subtitles(&mut subtitles, smil_subtitles);
    // `_parse_smil`'s head metadata.
    let (mut title, mut description, mut upload_date) = (None, None, None);
    for meta in &smil.meta {
        let (Some(name), Some(content)) = (
            get(meta, "name").filter(|name| !name.is_empty()),
            get(meta, "content").filter(|content| !content.is_empty()),
        ) else {
            continue;
        };
        if title.is_none() && name == "title" {
            title = Some(content.to_owned());
        } else if description.is_none() && matches!(name, "description" | "abstract") {
            description = Some(content.to_owned());
        } else if upload_date.is_none() && name == "date" {
            upload_date = crate::dates::unified_strdate(content, true);
        }
    }
    let thumbnails = smil
        .images
        .iter()
        .filter_map(|image| {
            Some(crate::info::Thumbnail {
                url: get(image, "src").filter(|src| !src.is_empty())?.to_owned(),
                id: get(image, "type").map(str::to_owned),
                width: int(get(image, "width")),
                height: int(get(image, "height")),
                preference: None,
                resolution: None,
                extra: Default::default(),
            })
        })
        .collect();
    let mut info = direct::build_at(url.clone(), url, String::new());
    info.formats = formats;
    info.direct = None;
    // `video_id = os.path.splitext(url_basename(smil_url))[0]`.
    let id = crate::screen::generic_id(url.as_str());
    info.id = id.clone();
    info.title = title.unwrap_or(id);
    info.webpage_url = url.clone();
    info.subtitles = subtitles;
    info.metadata.description = description;
    info.metadata.upload_date = upload_date;
    info.metadata.thumbnails = thumbnails;
    Ok(crate::info_order::mark_info(info, "smil"))
}

/// GenericIE's `playlist_result(self._parse_xspf(doc, video_id, ...),
/// video_id)`: untitled, each track an entry with the page's ID.
fn xspf_playlist(
    tracks: Vec<super::xspf::Track>,
    url: &Url,
    page_url: &Url,
) -> Result<ExtractResult> {
    let id = crate::screen::generic_id(url.as_str());
    let mut entries = Vec::new();
    for track in tracks {
        let mut formats = Vec::new();
        for location in track.locations {
            let ext = direct::determine_ext(location.url.as_str())
                .unwrap_or_else(|| "unknown_video".into());
            let mut format: VideoFormat = serde_json::from_value(serde_json::json!({
                "format_id": location.label.unwrap_or_default(),
                "url": location.url.as_str(),
                "ext": ext,
                "width": location.width,
                "height": location.height,
            }))
            .map_err(|error| unsupported(&format!("XSPF location: {error}")))?;
            format
                .extra
                .insert("manifest_url".into(), page_url.as_str().into());
            crate::info_order::mark_format(&mut format, "xspf");
            formats.push(format);
        }
        let mut info = direct::build_at(page_url.clone(), page_url, String::new());
        info.formats = formats;
        info.direct = None;
        info.id = id.clone();
        info.title = track.title.unwrap_or_else(|| id.clone());
        info.webpage_url = page_url.clone();
        info.metadata.description = track.description;
        info.metadata.thumbnail = track.thumbnail;
        info.metadata.duration = track.duration.map(crate::info::PyNumber::Float);
        entries.push(crate::info_order::mark_info(info, "xspf-entry"));
    }
    let mut extra = serde_json::Map::new();
    extra.insert(
        crate::info_order::INFO_SHAPE.to_owned(),
        "xspf-playlist".into(),
    );
    Ok(ExtractResult::Playlist(PlaylistInfo {
        id,
        title: String::new(),
        webpage_url: page_url.clone(),
        extractor: "generic".into(),
        extractor_key: "Generic".into(),
        entries,
        extra,
    }))
}

/// An extractor with `_EMBED_REGEX` iframes (upstream `_extract_embeds`
/// asks every extractor, in `gen_extractor_classes` order).
pub(crate) struct EmbedSite {
    name: &'static str,
    key: &'static str,
    /// `None`: a configured extractor (YouTube), followed through
    /// [`crate::YoutubeDl`]'s registry.
    extractor: Option<&'static dyn super::Extractor>,
    embed_urls: fn(&Url, &str) -> Vec<Url>,
    temp_id: fn(&str) -> Option<String>,
}

static EMBED_SITES: [EmbedSite; 8] = [
    EmbedSite {
        name: super::bitmovin::NAME,
        key: super::bitmovin::KEY,
        extractor: Some(&super::bitmovin::Bitmovin),
        embed_urls: super::bitmovin::embed_urls,
        temp_id: super::bitmovin::temp_id,
    },
    EmbedSite {
        name: super::blogger::NAME,
        key: super::blogger::KEY,
        extractor: Some(&super::blogger::Blogger),
        embed_urls: super::blogger::embed_urls,
        temp_id: super::blogger::temp_id,
    },
    EmbedSite {
        name: super::crooksandliars::NAME,
        key: super::crooksandliars::KEY,
        extractor: Some(&super::crooksandliars::CrooksAndLiars),
        embed_urls: super::crooksandliars::embed_urls,
        temp_id: super::crooksandliars::temp_id,
    },
    EmbedSite {
        name: "JWPlatform",
        key: "JWPlatform",
        extractor: Some(&super::jwplatform::JwPlatform),
        embed_urls: super::jwplatform::embed_urls,
        temp_id: super::jwplatform::temp_id,
    },
    EmbedSite {
        name: super::maariv::NAME,
        key: super::maariv::KEY,
        extractor: Some(&super::maariv::Maariv),
        embed_urls: super::maariv::embed_urls,
        temp_id: super::maariv::temp_id,
    },
    EmbedSite {
        name: super::megaphone::NAME,
        key: super::megaphone::KEY,
        extractor: Some(&super::megaphone::Megaphone),
        embed_urls: super::megaphone::embed_urls,
        temp_id: super::megaphone::temp_id,
    },
    EmbedSite {
        name: "Streamable",
        key: "Streamable",
        extractor: Some(&super::streamable::Streamable),
        embed_urls: super::streamable::embed_urls,
        temp_id: super::streamable::temp_id,
    },
    EmbedSite {
        name: "youtube",
        key: "Youtube",
        extractor: None,
        embed_urls: super::youtube::embed_urls,
        temp_id: super::youtube::temp_id,
    },
];

/// Every embed site's URLs with `report_detected(f'{IE_NAME} embed', n)`.
fn embeds(url: &Url, html: &str) -> Vec<(&'static EmbedSite, Url)> {
    let mut found = Vec::new();
    for site in &EMBED_SITES {
        let urls = (site.embed_urls)(url, html);
        match urls.len() {
            0 => {}
            1 => crate::screen::debug(&format!("Identified a {} embed", site.name)),
            count => crate::screen::debug(&format!("Identified {count} {} embeds", site.name)),
        }
        found.extend(urls.into_iter().map(|embed| (site, embed)));
    }
    found
}

/// `_extract_rss`'s playlist: `url_transparent` placeholder entries that
/// [`crate::YoutubeDl::resolve_entry`] extracts when they are processed.
fn rss_playlist(feed: super::rss::Feed, url: &Url, page_url: &Url) -> ExtractResult {
    use serde_json::Value;
    let entries = feed
        .items
        .into_iter()
        .map(|item| {
            let mut info = direct::build_at(page_url.clone(), page_url, String::new());
            info.formats.clear();
            info.direct = None;
            info.id = item.guid.clone().unwrap_or_default();
            if let Some(Value::String(title)) = item.fields.first().map(|(_, value)| value) {
                info.title = title.clone();
            }
            let fields: serde_json::Map<String, Value> = item
                .fields
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect();
            info.extra.insert(
                crate::TRANSPARENT.into(),
                serde_json::json!({"url": item.url, "force_videoid": item.guid, "fields": fields}),
            );
            info
        })
        .collect();
    let mut extra = serde_json::Map::new();
    extra.insert(
        crate::info_order::INFO_SHAPE.to_owned(),
        "rss-playlist".into(),
    );
    extra.insert(
        "description".into(),
        feed.description.map_or(Value::Null, Value::from),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id: url.to_string(),
        title: feed.title.unwrap_or_default(),
        webpage_url: page_url.clone(),
        extractor: "generic".into(),
        extractor_key: "Generic".into(),
        entries,
        extra,
    })
}

/// Upstream `smuggle_url` data for a JSON-LD `url_transparent` result:
/// `force_videoid`, `to_generic` and `referer`.
struct Smuggled {
    video_id: String,
    referer: Option<Url>,
    /// The smuggled URL as upstream prints it.
    shown: String,
}

thread_local! {
    static SMUGGLED: std::cell::RefCell<Option<Smuggled>> = const { std::cell::RefCell::new(None) };
}

/// `smuggle_url(url, {'force_videoid': id})` for the next generic
/// extraction; returns the smuggled URL as upstream prints it.
pub(crate) fn smuggle_video_id(url: &Url, video_id: &str) -> String {
    let shown = smuggled_text(url.as_str(), video_id);
    SMUGGLED.with(|smuggled| {
        smuggled.replace(Some(Smuggled {
            video_id: video_id.to_owned(),
            referer: None,
            shown: shown.clone(),
        }))
    });
    shown
}

/// `smuggle_url(text, {'force_videoid': video_id})`.
pub(crate) fn smuggled_text(text: &str, video_id: &str) -> String {
    let data = serde_json::json!({ "force_videoid": video_id });
    format!(
        "{text}#__youtubedl_smuggle={}",
        quote_plus(&crate::output::python_json_dumps(&data))
    )
}

/// Drop smuggled data a non-generic extraction did not consume.
pub(crate) fn clear_smuggled() {
    SMUGGLED.with(|smuggled| smuggled.borrow_mut().take());
}

/// Python `urllib.parse.quote_plus`.
pub(crate) fn quote_plus(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' => {
                out.push(char::from(byte));
            }
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

impl GenericHttp {
    /// Upstream's JSON-LD embed: a `url_transparent` result of the
    /// smuggled media URL (re-extracted by GenericIE with the page's ID and
    /// Referer), overlaid with the JSON-LD and page fields.
    #[allow(clippy::too_many_arguments)]
    fn json_ld_result(
        &self,
        client: &Client,
        url: &Url,
        page_url: &Url,
        html: &str,
        timestamp: Option<i64>,
        target: &str,
        json_ld: serde_json::Map<String, serde_json::Value>,
    ) -> Result<Option<ExtractResult>> {
        use serde_json::Value;
        // An `ext` makes a `video` result of the bare URL instead.
        if json_ld.contains_key("ext") {
            return Err(unsupported(
                "JSON-LD media with an encoding format is not supported natively",
            ));
        }
        let target = url
            .join(target)
            .map_err(|_| unsupported("JSON-LD media URL is invalid"))?;
        if super::youtube_suitable(target.as_str()) || !matches!(target.scheme(), "http" | "https")
        {
            return Err(unsupported(
                "JSON-LD media URLs other than HTTP are not supported natively",
            ));
        }
        let video_id = crate::screen::generic_id(url.as_str());
        let data = serde_json::json!({
            "force_videoid": video_id,
            "to_generic": true,
            "referer": url.as_str(),
        });
        let shown = format!(
            "{target}#__youtubedl_smuggle={}",
            quote_plus(&crate::output::python_json_dumps(&data))
        );
        SMUGGLED.with(|smuggled| {
            smuggled.replace(Some(Smuggled {
                video_id,
                referer: Some(url.clone()),
                shown,
            }))
        });
        let inner = self.extract_result(client, &target);
        SMUGGLED.with(|smuggled| smuggled.borrow_mut().take());
        let Some(ExtractResult::Video(mut info)) = inner? else {
            return Err(unsupported(
                "JSON-LD media that is not direct media is not supported natively",
            ));
        };
        let inner_shape = match info.extra.get(crate::info_order::INFO_SHAPE) {
            Some(Value::String(shape))
                if matches!(shape.as_str(), "generic-direct" | "generic-untyped") =>
            {
                shape.clone()
            }
            _ => {
                return Err(unsupported(
                    "JSON-LD media that is not direct media is not supported natively",
                ));
            }
        };
        // `merge_dicts({...}, json_ld)` then the page's `info_dict`; the
        // `url_transparent` update skips `_type`, `url`, `id`, `extractor`.
        let mut outer: Vec<(String, Value)> = json_ld
            .into_iter()
            .filter(|(key, _)| key != "url")
            .collect();
        let page = [
            ("title", page_title(html).map(Value::from)),
            ("timestamp", timestamp.map(Value::from)),
            (
                "description",
                crate::html::og_search_property(html, "description").map(Value::from),
            ),
            (
                "thumbnail",
                crate::html::og_search_property(html, "image").map(Value::from),
            ),
            ("age_limit", crate::html::rta_search(html).map(Value::from)),
        ];
        for (key, value) in page {
            if let Some(value) = value
                && !outer.iter().any(|(existing, _)| existing == key)
            {
                outer.push((key.to_owned(), value));
            }
        }
        let keys: Vec<String> = outer.iter().map(|(key, _)| key.clone()).collect();
        for (key, value) in outer {
            match key.as_str() {
                "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
        }
        info.webpage_url = page_url.clone();
        let shape = format!("{inner_shape}|transparent:{}", keys.join(","));
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info, &shape,
        ))))
    }
}

/// A found manifest URL: `_extract_m3u8_formats_and_subtitles` /
/// `_extract_mpd_formats_and_subtitles` with the page as Referer, then
/// `_extra_manifest_info` (and HLS's live status check).
fn found_manifest(
    client: &Client,
    media: &Url,
    page_url: &Url,
    id: &str,
    dash_hint: bool,
) -> Result<VideoInfo> {
    use crate::screen::screen;
    let note = if dash_hint {
        format!("[generic] {id}: Downloading MPD manifest")
    } else {
        format!("[generic] {id}: Downloading m3u8 information")
    };
    crate::screen::expect_page(id, Some(note), None);
    let manifest = direct::request(client, media, Some(page_url)).and_then(|response| {
        if dash_hint {
            dash::from_response(response, media, true)
        } else {
            hls::from_response(client, response, media, Some(page_url), true, None)
        }
    });
    crate::screen::clear_page();
    let mut info = manifest?
        .ok_or_else(|| unsupported("the found manifest URL is not a supported manifest"))?;
    match crate::manifest_args::extra_manifest_info(&mut info, media) {
        Some(live) => {
            info.metadata.live_status = Some(
                if live == "false" {
                    "not_live"
                } else {
                    "is_live"
                }
                .into(),
            );
        }
        None if !dash_hint => {
            screen(&format!("[generic] {id}: Checking m3u8 live status"));
            crate::screen::expect_page(id, None, None);
            hls::check_live_status(client, &mut info);
            crate::screen::clear_page();
        }
        None => {}
    }
    Ok(info)
}

/// `{'formats': ..., 'subtitles': ...}` merged with the page fields:
/// manifest sources' formats and subtitles (`hls`/`dash` IDs,
/// `fatal=False`), a direct source only while there are no formats yet
/// (upstream's in-loop check), and `_extra_manifest_info`.
fn videojs_info(
    client: &Client,
    embed: super::generic_urls::VideoJs,
    url: &Url,
    page_url: &Url,
    html: &str,
    timestamp: Option<i64>,
) -> Result<Option<ExtractResult>> {
    use serde_json::Value;
    let invalid = |error: serde_json::Error| unsupported(&format!("Video.js embed: {error}"));
    let video_id = crate::screen::generic_id(url.as_str());
    let mut formats: Vec<VideoFormat> = Vec::new();
    let mut manifest_subtitles = Subtitles::default();
    for (src, kind, ext) in &embed.sources {
        let dash = kind.as_deref() == Some("application/dash+xml") || ext == "mpd";
        let hls = !dash && (kind.as_deref() == Some("application/x-mpegurl") || ext == "m3u8");
        if dash || hls {
            let note = if dash {
                format!("[generic] {video_id}: Downloading MPD manifest")
            } else {
                format!("[generic] {video_id}: Downloading m3u8 information")
            };
            crate::screen::expect_page(&video_id, Some(note), None);
            let parsed = direct::request(client, src, None).and_then(|response| {
                if dash {
                    dash::from_response(response, src, true)
                } else {
                    hls::from_response(client, response, src, None, true, Some("hls"))
                }
            });
            crate::screen::clear_page();
            match parsed {
                Ok(Some(info)) => {
                    // `mpd_id='dash'` (HLS takes `m3u8_id='hls'` itself).
                    formats.extend(info.formats.into_iter().map(|mut format| {
                        if dash {
                            format.format_id = format!("dash-{}", format.format_id);
                        }
                        format
                    }));
                    for (language, tracks) in info.subtitles.entries {
                        match manifest_subtitles
                            .entries
                            .iter_mut()
                            .find(|(key, _)| *key == language)
                        {
                            Some((_, existing)) => existing.extend(tracks),
                            None => manifest_subtitles.entries.push((language, tracks)),
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => crate::screen::warning(&format!(
                    "[generic] {video_id}: Failed to download {} manifest: {error}",
                    if dash { "MPD" } else { "m3u8" }
                )),
            }
        }
        if formats.is_empty() {
            // `mimetype2ext(src_type) or ext if ext in KNOWN_EXTENSIONS else 'mp4'`.
            let ext = if direct::known_extension(ext) {
                kind.as_deref()
                    .map(|kind| direct::mimetype2ext_or(kind, None))
                    .filter(|mime| !mime.is_empty())
                    .unwrap_or_else(|| ext.clone())
            } else {
                "mp4".into()
            };
            let mut format = serde_json::Map::new();
            format.insert("url".into(), src.as_str().into());
            format.insert("ext".into(), ext.into());
            format.insert(
                "http_headers".into(),
                serde_json::json!({"Referer": page_url.as_str()}),
            );
            format.insert("format_id".into(), "".into());
            let mut format: VideoFormat =
                serde_json::from_value(Value::Object(format)).map_err(invalid)?;
            crate::info_order::mark_format(&mut format, "videojs");
            formats.push(format);
        }
    }
    if formats.is_empty() && embed.subtitles.is_empty() {
        return Ok(None);
    }
    if formats.is_empty() {
        return Err(unsupported(
            "Video.js embeds without media sources are not supported natively",
        ));
    }
    crate::screen::debug("Identified a video.js embed");
    let first = &formats[0];
    let mut info = direct::build_at(first.url.clone(), &first.url, first.ext.clone());
    info.formats = formats;
    info.webpage_url = page_url.clone();
    info.direct = None;
    info.id = video_id;
    info.title = page_title(html).unwrap_or_else(|| "video".into());
    info.metadata.timestamp = timestamp.map(crate::info::PyNumber::Int);
    info.metadata.description = crate::html::og_search_property(html, "description");
    info.metadata.thumbnail = crate::html::og_search_property(html, "image");
    info.metadata.age_limit = crate::html::rta_search(html);
    let mut subtitles = manifest_subtitles;
    for (language, mut track) in embed.subtitles {
        let url = Url::parse(track["url"].as_str().unwrap_or_default())
            .map_err(|_| unsupported("Video.js subtitle URL is invalid"))?;
        track.shift_remove("url");
        let name = track
            .shift_remove("name")
            .and_then(|name| name.as_str().map(str::to_owned));
        let format = SubtitleFormat {
            name,
            ext: subtitle_extension(&url),
            url,
            protocol: None,
            fragments: Vec::new(),
            extra: track,
        };
        // `subtitles.setdefault(lang, []).append(...)`.
        match subtitles
            .entries
            .iter_mut()
            .find(|(key, _)| key.as_deref() == Some(language.as_str()))
        {
            Some((_, tracks)) => tracks.push(format),
            None => subtitles.entries.push((Some(language), vec![format])),
        }
    }
    info.subtitles = subtitles;
    // `_extra_manifest_info(info_dict, src)` (the last source's URL).
    if let Some((src, _, _)) = embed.sources.last() {
        match crate::manifest_args::extra_manifest_info(&mut info, src) {
            Some(live) => {
                info.metadata.live_status = Some(
                    if live == "false" {
                        "not_live"
                    } else {
                        "is_live"
                    }
                    .into(),
                );
            }
            None if info
                .formats
                .iter()
                .any(|format| format.determine_protocol() == "m3u8_native") =>
            {
                crate::screen::screen("[generic] Checking m3u8 live status");
                crate::screen::expect_page("", None, None);
                hls::check_live_status(client, &mut info);
                crate::screen::clear_page();
            }
            None => {}
        }
    }
    Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
        info, "videojs",
    ))))
}

/// `merge_dicts(jwplayer_entry, info_dict)` as an info dict.
fn jwplayer_info(
    client: &Client,
    entry: serde_json::Map<String, serde_json::Value>,
    page_url: &Url,
    html: &str,
    timestamp: Option<i64>,
    video_id: &str,
) -> Result<Option<ExtractResult>> {
    let Some((mut info, keys)) = jwplayer_entry_info(client, entry, page_url, video_id, "generic")?
    else {
        return Ok(None);
    };
    // `report_detected` follows `_parse_jwplayer_data` (and its requests).
    crate::screen::debug("Identified a JW Player data");
    if !keys.iter().any(|key| key == "title") {
        info.title = page_title(html).unwrap_or_else(|| "video".into());
    }
    if !keys.iter().any(|key| key == "timestamp") {
        info.metadata.timestamp = timestamp.map(crate::info::PyNumber::Int);
    }
    if !keys.iter().any(|key| key == "description") {
        info.metadata.description = crate::html::og_search_property(html, "description");
    }
    if !keys.iter().any(|key| key == "thumbnail") {
        info.metadata.thumbnail = crate::html::og_search_property(html, "image");
    }
    if !keys.iter().any(|key| key == "age_limit") {
        info.metadata.age_limit = crate::html::rta_search(html);
    }
    let shape = format!("jwplayer;{}", keys.join(","));
    Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
        info, &shape,
    ))))
}

/// A `_parse_jwplayer_data` entry as an info dict (`None` without formats):
/// its manifests requested under `extractor`, its sources as formats, and
/// its keys (in order).
pub(crate) fn jwplayer_entry_info(
    client: &Client,
    entry: serde_json::Map<String, serde_json::Value>,
    page_url: &Url,
    video_id: &str,
    extractor: &str,
) -> Result<Option<(VideoInfo, Vec<String>)>> {
    use serde_json::Value;
    let invalid = |error: serde_json::Error| unsupported(&format!("JW Player data: {error}"));
    let mut formats = Vec::new();
    for format in entry["formats"].as_array().into_iter().flatten() {
        let Value::Object(map) = format else { continue };
        // `_extract_m3u8_formats`/`_extract_mpd_formats(..., fatal=False)`:
        // formats only (subtitle tracks are reported as ignored).
        if let Some(manifest) = map.get(super::jwplayer::MANIFEST).and_then(Value::as_str) {
            let manifest =
                Url::parse(manifest).map_err(|_| unsupported("JW Player source URL is invalid"))?;
            let dash = map.get("dash") == Some(&Value::Bool(true));
            let note = if dash {
                format!("[{extractor}] {video_id}: Downloading MPD manifest")
            } else {
                format!("[{extractor}] {video_id}: Downloading m3u8 information")
            };
            crate::screen::expect_page(video_id, Some(note), None);
            let parsed = direct::request(client, &manifest, None).and_then(|response| {
                if dash {
                    dash::from_response(response, &manifest, true)
                } else {
                    hls::from_response(client, response, &manifest, None, true, None)
                }
            });
            crate::screen::clear_page();
            match parsed {
                Ok(Some(info)) => {
                    if !info.subtitles.is_empty() {
                        crate::screen::warning_once(&format!(
                            "[{extractor}] Ignoring subtitle tracks found in the {} manifest; if any subtitle tracks are missing, {}",
                            if dash { "DASH" } else { "HLS" },
                            crate::BUG_REPORTS_MESSAGE
                        ));
                    }
                    formats.extend(info.formats);
                }
                Ok(None) => {}
                Err(error) => crate::screen::warning(&format!(
                    "[{extractor}] {video_id}: Failed to download {} manifest: {error}",
                    if dash { "MPD" } else { "m3u8" }
                )),
            }
            continue;
        }
        let url = Url::parse(map["url"].as_str().unwrap_or_default())
            .map_err(|_| unsupported("JW Player source URL is invalid"))?;
        let mut source = map.clone();
        let audio = source.get("vcodec").and_then(Value::as_str) == Some("none");
        if source.get("ext").is_none_or(Value::is_null) {
            source.insert(
                "ext".into(),
                direct::determine_ext(url.as_str())
                    .unwrap_or_else(|| "unknown_video".into())
                    .to_lowercase()
                    .into(),
            );
        }
        if source.get("format_id").is_none_or(Value::is_null) {
            source.insert("format_id".into(), "".into());
        }
        if audio {
            source.insert("stream_kind".into(), "audio_only".into());
        }
        let mut format: VideoFormat =
            serde_json::from_value(Value::Object(source)).map_err(invalid)?;
        crate::info_order::mark_format(
            &mut format,
            if audio {
                "jwplayer-audio"
            } else {
                "jwplayer-source"
            },
        );
        formats.push(format);
    }
    // Without formats (failed manifests) the next steps run.
    let Some(first) = formats.first() else {
        return Ok(None);
    };
    let mut info = direct::build_at(first.url.clone(), &first.url, first.ext.clone());
    info.formats = formats;
    info.webpage_url = page_url.clone();
    info.direct = None;
    // The entry's keys (their order is the info dict's), then the page's.
    let keys: Vec<String> = entry.keys().cloned().collect();
    let mut subtitles = crate::Subtitles::default();
    for (key, value) in entry {
        match key.as_str() {
            "formats" => {}
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            "subtitles" => {
                for (language, tracks) in value.as_object().into_iter().flatten() {
                    let tracks = tracks
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|track| track["url"].as_str())
                        .filter_map(|url| Url::parse(url).ok())
                        .map(|url| SubtitleFormat {
                            name: None,
                            ext: subtitle_extension(&url),
                            url,
                            protocol: None,
                            fragments: Vec::new(),
                            extra: Default::default(),
                        })
                        .collect();
                    subtitles.insert(Some(language.clone()), tracks);
                }
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    info.subtitles = subtitles;
    Ok(Some((info, keys)))
}

/// `_generic_title`: `og:title`, else the `<title>` text (whitespace
/// collapsed), `None` when both are empty.
pub(crate) fn page_title(html: &str) -> Option<String> {
    let document = Html::parse_document(html);
    let title = document
        .select(&selector("meta[property='og:title']"))
        .find_map(|meta| nonempty(meta.value().attr("content")))
        .map(str::to_owned)
        .or_else(|| {
            document
                .select(&selector("title"))
                .next()
                .map(|title| title.text().collect::<String>())
        })?;
    let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
    (!title.is_empty()).then_some(title)
}

/// The generic page request's failure (see `Error::Webpage`).
fn webpage_error(id: &str, error: Error) -> Error {
    match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::Webpage {
            id: id.into(),
            source: Box::new(error),
        },
        error => error,
    }
}

/// `Unable to download webpage: ERR (caused by REPR)` for `Error::Webpage`.
pub(crate) fn webpage_error_text(id: &str, error: &Error) -> String {
    super::request_error_text("generic", id, "webpage", error)
}

#[cfg(test)]
mod subtitle_tests {
    use super::*;
    #[test]
    fn saved_html_tracks_match_actual_upstream_parser_and_extension_rules() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/subtitles/cases.json"))
                .unwrap();
        for case in fixture["html_cases"].as_array().unwrap() {
            let page = parse_page(
                &Url::parse(case["page_url"].as_str().unwrap()).unwrap(),
                case["html"].as_str().unwrap(),
            )
            .unwrap()
            .unwrap();
            let actual: Vec<_> = page.entries.iter().map(|entry| {
                let groups: Vec<_> = entry.subtitles.entries.iter().map(|(language, formats)| serde_json::json!({"language":language,"formats":formats})).collect();
                serde_json::json!({"subtitles":groups})
            }).collect();
            assert_eq!(
                serde_json::json!({"entries":actual}),
                case["expected"],
                "{}",
                case["name"]
            );
        }
    }
    #[test]
    fn subtitle_extension_keeps_pinned_fragment_query_and_trailing_slash_quirks() {
        for (url, ext) in [
            ("https://example.test/a.VTT?token=1#f", "vtt"),
            ("https://example.test/a.vtt#f", "unknown_video"),
            ("https://example.test/a.vtt/", "unknown_video"),
            ("https://example.test/a.mp4/", "mp4"),
            ("https://example.test/a.MP4/", "unknown_video"),
            ("https://example.test/no-ext", "unknown_video"),
        ] {
            assert_eq!(subtitle_extension(&Url::parse(url).unwrap()), ext);
        }
    }
}

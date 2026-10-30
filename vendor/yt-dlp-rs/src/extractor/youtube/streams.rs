//! Upstream `YoutubeIE._extract_formats_and_subtitles` over the player
//! responses of several InnerTube clients: HTTPS formats (deduplicated by
//! stream ID, PO token policies, signature and `n` challenges), then each
//! response's HLS and DASH manifests.
use super::innertube::{self, Policy};
use crate::{Error, Result, VideoFormat};
use serde_json::{Map, Value, json};
use std::collections::{BTreeSet, HashMap};
use url::Url;

/// `CHUNK_SIZE = 10 << 20`.
const CHUNK_SIZE: u64 = 10 << 20;
const ORIGINAL_LANG_VALUE: i64 = 10;
const DEFAULT_LANG_VALUE: i64 = 5;
/// `qualities([...])`'s order.
const QUALITIES: [&str; 14] = [
    "tiny",
    "audio_quality_ultralow",
    "audio_quality_low",
    "audio_quality_medium",
    "audio_quality_high",
    "small",
    "medium",
    "large",
    "hd720",
    "hd1080",
    "hd1440",
    "hd2160",
    "hd2880",
    "highres",
];

fn q(quality: Option<&str>) -> i64 {
    quality
        .and_then(|quality| QUALITIES.iter().position(|known| *known == quality))
        .map_or(-1, |index| index as i64)
}

/// One client's player response with upstream's `STREAMING_DATA_*` details.
pub(super) struct ClientResponse {
    pub client: &'static str,
    pub response: Value,
    pub player_token_provided: bool,
    /// `_get_available_at_timestamp`.
    pub available_at: i64,
}

/// The `n`/signature solver upstream calls through `solve_js_challenges`.
pub(super) trait Solver {
    /// Solve every challenge at once; `n` maps challenges to results and
    /// `sig` maps signature lengths to permutations (`spec`).
    fn solve(
        &mut self,
        n: &BTreeSet<String>,
        sig: &BTreeSet<usize>,
    ) -> (HashMap<String, String>, HashMap<usize, Vec<usize>>);
}

/// `traverse_obj(fmt_url, ({parse_qs}, KEY, 0))`.
fn query_value(url: &str, key: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    parsed
        .query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

/// `urllib.parse.parse_qs(signatureCipher)`'s first values.
fn parse_qs(text: &str) -> HashMap<String, Vec<String>> {
    let mut found: HashMap<String, Vec<String>> = HashMap::new();
    for (name, value) in url::form_urlencoded::parse(text.as_bytes()) {
        if !value.is_empty() {
            found
                .entry(name.into_owned())
                .or_default()
                .push(value.into_owned());
        }
    }
    found
}

/// `get_manifest_n_challenge`: the `/n/VALUE/` path segment.
fn manifest_n_challenge(manifest: &str) -> Option<String> {
    let parsed = Url::parse(manifest).ok()?;
    let path = parsed.path();
    let start = path.find("/n/")? + 3;
    let rest = &path[start..];
    let end = rest.find('/')?;
    (end > 0).then(|| rest[..end].to_owned())
}

/// `join_nonempty(*values, delim=DELIM)`.
fn join_nonempty(values: &[Option<String>], delim: &str) -> Option<String> {
    let parts: Vec<&str> = values
        .iter()
        .flatten()
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join(delim))
}

/// `short_client_name`.
pub(super) fn short_client_name(client: &str) -> String {
    let base = client
        .rsplit_once('.')
        .map_or(client, |(variant, _)| variant);
    let mut parts = base.split('_');
    let main = parts.next().unwrap_or_default();
    let initials: String = parts.filter_map(|part| part.chars().next()).collect();
    let head: String = main.chars().take(4).collect();
    join_nonempty(&[Some(head), Some(initials)], "-")
        .unwrap_or_default()
        .to_uppercase()
}

fn gvs_pot_required(policy: &Policy, premium: bool, player_token: bool) -> bool {
    policy.required
        && !(policy.not_required_with_player_token && player_token)
        && !(policy.not_required_for_premium && premium)
}

fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    }
}

fn float_or_none(value: Option<&Value>, scale: f64) -> Option<f64> {
    match value {
        Some(Value::Number(number)) => number.as_f64().map(|value| value / scale),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok().map(|value| value / scale),
        _ => None,
    }
}

/// Python truthiness of an optional JSON value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(object)) => !object.is_empty(),
    }
}

/// `mimetype2ext` for YouTube's stream MIME types.
fn mimetype_ext(mime: &str) -> String {
    let subtype = mime.rsplit('/').next().unwrap_or(mime).to_ascii_lowercase();
    match (mime.to_ascii_lowercase().as_str(), subtype.as_str()) {
        ("audio/mp4", _) => "m4a".into(),
        ("audio/mpeg", _) => "mp3".into(),
        ("video/3gpp", _) => "3gp".into(),
        (_, "x-flv") => "flv".into(),
        (_, other) => other.into(),
    }
}

/// The output of [`extract`]: formats in upstream's yield order and the
/// merged manifest subtitles.
pub(super) struct Streams {
    pub formats: Vec<Map<String, Value>>,
    pub manifest_formats: Vec<VideoFormat>,
    /// Indices into the combined order (`true` for manifest formats).
    pub order: Vec<(bool, usize)>,
    pub subtitles: crate::Subtitles,
    pub manifest_ms: u64,
}

struct Language {
    original: Option<String>,
    default: Option<String>,
}

impl Language {
    /// `get_language_code_and_preference`.
    fn code_and_preference(&mut self, stream: &Value) -> (Option<String>, i64) {
        let track = stream.get("audioTrack");
        let display = track
            .and_then(|track| track.get("displayName"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase();
        let code = track
            .and_then(|track| track.get("id"))
            .and_then(Value::as_str)
            .and_then(|id| id.split('.').next())
            .filter(|code| !code.is_empty())
            .map(str::to_owned);
        if display.contains("descriptive") {
            return (join_nonempty(&[code, Some("desc".into())], "-"), -10);
        }
        if display.contains("original") {
            if code.is_some() && self.original.is_none() {
                self.original.clone_from(&code);
            }
            return (code, ORIGINAL_LANG_VALUE);
        }
        if truthy(track.and_then(|track| track.get("audioIsDefault"))) {
            if code.is_some() && self.default.is_none() {
                self.default.clone_from(&code);
            }
            return (code, DEFAULT_LANG_VALUE);
        }
        (code, -1)
    }
}

/// The context `_extract_formats_and_subtitles` reads.
pub(super) struct Context<'a> {
    pub client: &'a reqwest::blocking::Client,
    pub video_id: &'a str,
    pub player_url: Option<&'a str>,
    pub live_status: Option<&'a str>,
    pub duration: Option<i64>,
    pub verbose: bool,
    pub all_formats: bool,
    pub skip_bad_formats: bool,
    pub dashy: bool,
    pub missing_pot: bool,
    pub skip_player_js: bool,
    pub skip_manifests: BTreeSet<String>,
    pub playback: bool,
    pub premium: bool,
}

/// `process_format_stream`.
#[allow(clippy::too_many_arguments)]
fn process_format_stream(
    context: &Context<'_>,
    stream: &Value,
    proto: &str,
    client_name: &str,
    missing_pot: bool,
    super_resolution: bool,
    language: &mut Language,
    itag_qualities: &mut HashMap<String, String>,
    res_qualities: &mut Vec<(i64, Option<String>)>,
) -> Option<Map<String, Value>> {
    let itag = stream.get("itag").and_then(|itag| match itag {
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) => Some(text.clone()),
        _ => None,
    });
    let track = stream.get("audioTrack");
    let mut quality = stream
        .get("quality")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let height = int_or_none(stream.get("height"));
    if quality
        .as_deref()
        .is_none_or(|quality| quality == "tiny" || quality.is_empty())
    {
        let audio = stream
            .get("audioQuality")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase();
        if !audio.is_empty() {
            quality = Some(audio);
        }
    }
    // The 3gp format (17) in android client has a quality of "small".
    if itag.as_deref() == Some("17") {
        quality = Some("tiny".into());
    }
    if let Some(quality) = quality.as_ref().filter(|quality| !quality.is_empty()) {
        if let Some(itag) = &itag {
            itag_qualities.insert(itag.clone(), quality.clone());
        }
        if let Some(height) = height.filter(|height| *height != 0) {
            match res_qualities.iter_mut().find(|(known, _)| *known == height) {
                Some(entry) => entry.1 = Some(quality.clone()),
                None => res_qualities.push((height, Some(quality.clone()))),
            }
        }
    }
    let (language_code, language_preference) = language.code_and_preference(stream);
    let has_drm = truthy(stream.get("drmFamilies"));
    if has_drm {
        let mut message = format!(
            "Some {client_name} client {proto} formats have been skipped as they are DRM protected. "
        );
        if client_name == "tv" {
            message.push_str(
                "The current session may have an experiment that applies DRM to all videos on the tv client. See  https://github.com/yt-dlp/yt-dlp/issues/12563  for more details.",
            );
        }
        crate::screen::warning_once(&format!("[youtube] {}: {message}", context.video_id));
    }
    let bitrate = stream
        .get("averageBitrate")
        .filter(|value| truthy(Some(value)))
        .or_else(|| stream.get("bitrate"));
    let tbr = float_or_none(bitrate, 1000.0);
    let format_duration = float_or_none(stream.get("approxDurationMs"), 1000.0);
    let is_damaged = match (format_duration, context.duration) {
        (Some(format_duration), Some(duration)) => format_duration < duration.div_euclid(2) as f64,
        _ => false,
    };
    if is_damaged {
        crate::screen::warning_once(&format!(
            "[youtube] {}: Some {client_name} client {proto} formats are possibly damaged. They will be deprioritized",
            context.video_id
        ));
    }
    if missing_pot && !context.missing_pot {
        report_pot_format_skipped(context.video_id, client_name, proto);
        return None;
    }
    let name = stream
        .get("qualityLabel")
        .and_then(Value::as_str)
        .filter(|label| !label.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            quality
                .clone()
                .unwrap_or_default()
                .replace("audio_quality_", "")
        });
    let fps = int_or_none(stream.get("fps")).unwrap_or(0);
    let is_drc = truthy(stream.get("isDrc"));
    let display = join_nonempty(
        &[
            track
                .and_then(|track| track.get("displayName"))
                .and_then(Value::as_str)
                .map(str::to_owned),
            truthy(track.and_then(|track| track.get("audioIsDefault")))
                .then(|| "(default)".to_owned()),
        ],
        " ",
    );
    let projection = stream
        .get("projectionType")
        .and_then(Value::as_str)
        .map(|projection| projection.replace("RECTANGULAR", "").to_lowercase());
    let spatial = stream
        .get("spatialAudioType")
        .and_then(Value::as_str)
        .map(|spatial| spatial.replace("SPATIAL_AUDIO_TYPE_", "").to_lowercase());
    let format_note = join_nonempty(
        &[
            display,
            Some(name.clone()),
            is_drc.then(|| "DRC".to_owned()),
            super_resolution.then(|| "AI-upscaled".to_owned()),
            projection,
            spatial,
            is_damaged.then(|| "DAMAGED".to_owned()),
            missing_pot.then(|| "MISSING POT".to_owned()),
            (context.verbose || context.all_formats).then(|| short_client_name(client_name)),
        ],
        ", ",
    );
    let format_id = join_nonempty(
        &[
            itag.clone(),
            if is_drc {
                Some("drc".into())
            } else if super_resolution {
                Some("sr".into())
            } else {
                None
            },
        ],
        "-",
    );
    let source_preference = (if itag.as_deref() == Some("22") {
        -5
    } else {
        -1
    }) + if name.contains("Premium") { 100 } else { 0 };
    let filesize_approx = match (tbr, format_duration) {
        (Some(tbr), Some(duration)) => Some((duration * tbr * (1000.0 / 8.0)) as i64),
        _ => None,
    };
    let mut format = Map::new();
    format.insert(
        "asr".into(),
        int_or_none(stream.get("audioSampleRate")).into(),
    );
    format.insert(
        "filesize".into(),
        int_or_none(stream.get("contentLength")).into(),
    );
    format.insert("format_id".into(), format_id.into());
    if context.playback {
        // Native players can use these indexes directly instead of probing a
        // whole adaptive file. Keep normal CLI/download output unchanged.
        for (source, target) in [
            ("initRange", "ytui_init_range"),
            ("indexRange", "ytui_index_range"),
            ("approxDurationMs", "ytui_duration_ms"),
        ] {
            if let Some(value) = stream.get(source) {
                format.insert(target.into(), value.clone());
            }
        }
    }
    format.insert("format_note".into(), format_note.into());
    format.insert("source_preference".into(), source_preference.into());
    format.insert("fps".into(), (fps > 1).then_some(fps).into());
    format.insert(
        "audio_channels".into(),
        stream.get("audioChannels").cloned().unwrap_or(Value::Null),
    );
    format.insert("height".into(), height.into());
    format.insert(
        "quality".into(),
        (q(quality.as_deref()) as f64 - if is_drc { 0.5 } else { 0.0 }).into(),
    );
    format.insert("has_drm".into(), has_drm.into());
    format.insert("tbr".into(), tbr.into());
    format.insert("filesize_approx".into(), filesize_approx.into());
    format.insert("width".into(), int_or_none(stream.get("width")).into());
    format.insert("language".into(), language_code.into());
    format.insert("language_preference".into(), language_preference.into());
    format.insert(
        "preference".into(),
        if is_damaged {
            Value::from(-10)
        } else if itag.as_deref() == Some("17") {
            Value::from(-2)
        } else {
            Value::Null
        },
    );
    // `((?:[^/]+)/(?:[^;]+))(?:;\s*codecs="([^"]+)")?`
    let mime = stream
        .get("mimeType")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some((kind, rest)) = mime.split_once('/')
        && !kind.is_empty()
    {
        let (subtype, parameters) = rest.split_once(';').unwrap_or((rest, ""));
        if !subtype.is_empty() {
            let full = format!("{kind}/{subtype}");
            format.insert("ext".into(), mimetype_ext(&full).into());
            let codecs = parameters
                .trim_start()
                .strip_prefix("codecs=\"")
                .and_then(|codecs| codecs.split_once('"'))
                .map(|(codecs, _)| codecs);
            if let Some(parsed) = crate::codecs::parse_codecs(codecs, &mut |warning| {
                crate::screen::warning(warning);
            }) {
                format.insert("vcodec".into(), parsed.vcodec.into());
                format.insert("acodec".into(), parsed.acodec.into());
                format.insert("dynamic_range".into(), parsed.dynamic_range.into());
            }
        }
    }
    let single = ["acodec", "vcodec"]
        .iter()
        .any(|key| format.get(*key).and_then(Value::as_str) == Some("none"));
    if single
        && let Some(ext) = format
            .get("ext")
            .and_then(Value::as_str)
            .filter(|ext| !ext.is_empty())
    {
        let container = format!("{ext}_dash");
        format.insert("container".into(), container.into());
    }
    Some(format)
}

/// `_report_pot_format_skipped`: a debug message for default clients.
fn report_pot_format_skipped(video_id: &str, client_name: &str, proto: &str) {
    let message = format!(
        "{video_id}: {client_name} client {proto} formats require a GVS PO Token which was not provided. They will be skipped as they may yield HTTP Error 403. You can manually pass a GVS PO Token for this client with --extractor-args \"youtube:po_token={client_name}.gvs+XXX\". For more information, refer to  https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide"
    );
    if super::watch::is_default_client(client_name) {
        crate::screen::debug(&message);
    } else {
        crate::screen::warning_once(&format!("[youtube] {message}"));
    }
}

/// `is_super_resolution`: `xtags` with `sr=1`.
fn is_super_resolution(url: &str) -> bool {
    Url::parse(url).is_ok_and(|parsed| {
        parsed
            .query_pairs()
            .filter(|(name, _)| name == "xtags")
            .any(|(_, value)| {
                url::form_urlencoded::parse(value.as_bytes())
                    .any(|(name, value)| name == "sr" && value == "1")
            })
    })
}

/// Replace one query parameter like `update_url_query` (others kept).
fn set_query(url: &str, key: &str, value: &str) -> String {
    let Ok(parsed) = Url::parse(url) else {
        return url.to_owned();
    };
    super::super::update_url_query(&parsed, &[(key, value)]).to_string()
}

/// Upstream's two-pass extraction: collect challenges, then build formats.
pub(super) fn extract(
    context: &Context<'_>,
    responses: &[ClientResponse],
    solver: &mut dyn Solver,
) -> Result<Streams> {
    let mut language = Language {
        original: None,
        default: None,
    };
    let mut itags: HashMap<String, Vec<(String, Option<String>)>> = HashMap::new();
    let mut stream_ids: Vec<(Option<String>, Option<String>, bool)> = Vec::new();
    let mut itag_qualities: HashMap<String, String> = HashMap::new();
    let mut res_qualities: Vec<(i64, Option<String>)> = vec![(0, None)];
    let mut subtitles = crate::Subtitles::default();
    let mut manifest_ms = 0;

    // First pass: every n and signature challenge, solved at once later.
    let mut n_challenges: BTreeSet<String> = BTreeSet::new();
    let mut s_challenges: BTreeSet<usize> = BTreeSet::new();
    for response in responses {
        let Some(data) = response
            .response
            .get("streamingData")
            .filter(|data| data.is_object())
        else {
            continue;
        };
        for key in ["formats", "adaptiveFormats"] {
            for stream in data
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if !stream.is_object() {
                    continue;
                }
                let mut url = stream.get("url").and_then(Value::as_str).map(str::to_owned);
                if url.is_none() {
                    let cipher = parse_qs(
                        stream
                            .get("signatureCipher")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    );
                    url = cipher
                        .get("url")
                        .and_then(|values| values.first())
                        .and_then(|url| super::super::json_ld::url_or_none(url));
                    if let Some(s) = cipher.get("s").and_then(|values| values.first()) {
                        s_challenges.insert(s.chars().count());
                    }
                }
                if let Some(n) = url.as_deref().and_then(|url| query_value(url, "n")) {
                    n_challenges.insert(n);
                }
            }
        }
        for key in ["hlsManifestUrl", "dashManifestUrl"] {
            if let Some(n) = data
                .get(key)
                .and_then(Value::as_str)
                .and_then(manifest_n_challenge)
            {
                n_challenges.insert(n);
            }
        }
    }
    let mut solved: Option<Solved> = None;

    let mut formats: Vec<Map<String, Value>> = Vec::new();
    let mut manifest_formats: Vec<VideoFormat> = Vec::new();
    let mut order: Vec<(bool, usize)> = Vec::new();
    let needs_live = matches!(context.live_status, Some("post_live"));
    for response in responses {
        let Some(data) = response
            .response
            .get("streamingData")
            .filter(|data| data.is_object())
        else {
            continue;
        };
        let client_name = response.client;
        let config = innertube::client(client_name);
        let policy = |proto: &str| -> Policy {
            config.map_or_else(Policy::default, |config| match proto {
                "https" => config.https,
                "dash" => config.dash,
                _ => config.hls,
            })
        };
        // `process_https_formats`.
        let mut https = Vec::new();
        for key in ["formats", "adaptiveFormats"] {
            for stream in data
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if truthy(stream.get("targetDurationSec"))
                    && context.skip_bad_formats
                    && !needs_live
                {
                    continue;
                }
                if stream.get("type").and_then(Value::as_str) == Some("FORMAT_STREAM_TYPE_OTF")
                    && !truthy(stream.get("drmFamilies"))
                {
                    continue;
                }
                let stream_id = (
                    stream.get("itag").and_then(|itag| match itag {
                        Value::Number(number) => Some(number.to_string()),
                        Value::String(text) => Some(text.clone()),
                        _ => None,
                    }),
                    stream
                        .get("audioTrack")
                        .and_then(|track| track.get("id"))
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    truthy(stream.get("isDrc")),
                );
                if !context.all_formats && stream_ids.contains(&stream_id) {
                    continue;
                }
                let require_po_token = stream_id.0.as_deref() != Some("18")
                    && gvs_pot_required(
                        &policy("https"),
                        context.premium,
                        response.player_token_provided,
                    );
                // No PO token provider is available.
                let po_token: Option<String> = None;
                let mut url = stream.get("url").and_then(Value::as_str).map(str::to_owned);
                let mut encrypted = None;
                let mut cipher = HashMap::new();
                if url.is_none() {
                    language.code_and_preference(stream);
                    cipher = parse_qs(
                        stream
                            .get("signatureCipher")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    );
                    url = cipher
                        .get("url")
                        .and_then(|values| values.first())
                        .and_then(|url| super::super::json_ld::url_or_none(url));
                    encrypted = cipher.get("s").and_then(|values| values.first()).cloned();
                    if cipher.is_empty()
                        || url.is_none()
                        || !(context.skip_player_js || context.player_url.is_some())
                        || encrypted.is_none()
                    {
                        let template = |prefix: &str, reason: &str| {
                            format!(
                                "{prefix}Some {client_name} client https formats have been skipped as they are missing a URL. {reason}. See  https://github.com/yt-dlp/yt-dlp/issues/12482  for more details"
                            )
                        };
                        if matches!(client_name, "web" | "web_safari") {
                            crate::screen::debug(&template(
                                &format!("{}: ", context.video_id),
                                "YouTube is forcing SABR streaming for this client",
                            ));
                        } else {
                            crate::screen::warning_once(&format!(
                                "[youtube] {}: {}",
                                context.video_id,
                                template(
                                    "",
                                    "YouTube may have enabled the SABR-only streaming experiment for the current session"
                                )
                            ));
                        }
                        continue;
                    }
                }
                let Some(mut url) = url else {
                    continue;
                };
                let Some(mut format) = process_format_stream(
                    context,
                    stream,
                    "https",
                    client_name,
                    require_po_token && po_token.is_none(),
                    is_super_resolution(&url),
                    &mut language,
                    &mut itag_qualities,
                    &mut res_qualities,
                ) else {
                    continue;
                };
                if let Some(encrypted) = &encrypted {
                    if context.skip_player_js {
                        continue;
                    }
                    let (_, sig) = solve_once(
                        &mut solved,
                        solver,
                        &n_challenges,
                        &s_challenges,
                        context.video_id,
                    );
                    let Some(spec) = sig.get(&encrypted.chars().count()) else {
                        continue;
                    };
                    let chars: Vec<char> = encrypted.chars().collect();
                    let signature: String =
                        spec.iter().filter_map(|index| chars.get(*index)).collect();
                    let parameter = cipher
                        .get("sp")
                        .and_then(|values| values.last())
                        .cloned()
                        .unwrap_or_else(|| "signature".into());
                    // `fmt_url += '&{}={}'.format(sp, solve_sig(...))`: unencoded.
                    url.push_str(&format!("&{parameter}={signature}"));
                }
                if let Some(n) = query_value(&url, "n") {
                    if context.skip_player_js {
                        continue;
                    }
                    let (solutions, _) = solve_once(
                        &mut solved,
                        solver,
                        &n_challenges,
                        &s_challenges,
                        context.video_id,
                    );
                    let Some(result) = solutions.get(&n) else {
                        continue;
                    };
                    url = set_query(&url, "n", result);
                }
                format.insert("url".into(), url.into());
                if let Some(itag) = &stream_id.0 {
                    itags.entry(itag.clone()).or_default().push((
                        "https".into(),
                        format
                            .get("language")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                    ));
                }
                stream_ids.push(stream_id);
                if !matches!(context.live_status, Some("is_live" | "post_live")) {
                    format.insert("available_at".into(), response.available_at.into());
                }
                https.push(format);
            }
        }
        for mut format in https {
            let filesize = format.get("filesize").and_then(Value::as_i64).unwrap_or(0);
            if (context.all_formats || context.dashy) && filesize != 0 {
                let mut dashy = format.clone();
                if context.all_formats {
                    let id = dashy
                        .get("format_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    dashy.insert("format_id".into(), format!("{id}-dashy").into());
                }
                dashy.insert("protocol".into(), "http_dash_segments".into());
                let base = dashy
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let fragments: Vec<Value> = (0..filesize as u64)
                    .step_by(CHUNK_SIZE as usize)
                    .map(|start| {
                        let end = (start + CHUNK_SIZE - 1).min(filesize as u64);
                        json!({"url": set_query(&base, "range", &format!("{start}-{end}"))})
                    })
                    .collect();
                dashy.insert("fragments".into(), Value::Array(fragments));
                order.push((false, formats.len()));
                formats.push(dashy);
            }
            if context.all_formats || !context.dashy {
                format.insert(
                    "downloader_options".into(),
                    json!({"http_chunk_size": CHUNK_SIZE}),
                );
                order.push((false, formats.len()));
                formats.push(format);
            }
        }

        let mut skip_manifests = context.skip_manifests.clone();
        if needs_live && context.skip_bad_formats {
            skip_manifests.insert("hls".into());
        }
        if context.skip_bad_formats
            && (context.live_status == Some("is_live")
                || (context.live_status == Some("post_live")
                    && context.duration.unwrap_or(0) > 2 * 3600))
        {
            skip_manifests.insert("dash".into());
        }
        for (proto, key) in [("hls", "hlsManifestUrl"), ("dash", "dashManifestUrl")] {
            if skip_manifests.contains(proto) {
                continue;
            }
            let Some(manifest) = data
                .get(key)
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
            else {
                continue;
            };
            let Ok(parsed) = Url::parse(manifest) else {
                continue;
            };
            let mut path = parsed.path().to_owned();
            let mut suffix = String::new();
            if proto == "hls" {
                for tail in ["/file/index.m3u8", "/playlist/index.m3u8"] {
                    if let Some(head) = path.strip_suffix(tail).filter(|head| !head.is_empty()) {
                        suffix = tail.to_owned();
                        path = head.to_owned();
                        break;
                    }
                }
            }
            let n = manifest_n_challenge(manifest);
            let mut solved_n = false;
            if let Some(n) = &n
                && !context.skip_player_js
            {
                let (solutions, _) = solve_once(
                    &mut solved,
                    solver,
                    &n_challenges,
                    &s_challenges,
                    context.video_id,
                );
                if let Some(result) = solutions.get(n) {
                    path = path.replace(&format!("/n/{n}"), &format!("/n/{result}"));
                    solved_n = path.contains(result.as_str());
                }
            }
            let require_po_token = gvs_pot_required(
                &policy(proto),
                context.premium,
                response.player_token_provided,
            );
            if require_po_token && !context.missing_pot {
                report_pot_format_skipped(context.video_id, client_name, proto);
                continue;
            }
            if !(solved_n || n.is_none()) {
                continue;
            }
            let mut manifest_url = parsed.clone();
            manifest_url.set_path(&format!("{path}{suffix}"));
            let manifest_at = std::time::Instant::now();
            let stream = if proto == "hls" && context.playback {
                super::super::extract_m3u8_for_playback(
                    context.client,
                    &manifest_url,
                    context.video_id,
                )
            } else if proto == "hls" {
                super::super::extract_m3u8_optional(
                    context.client,
                    &manifest_url,
                    "youtube",
                    context.video_id,
                    None,
                    true,
                )
            } else {
                super::super::extract_mpd_optional(
                    context.client,
                    &manifest_url,
                    "youtube",
                    context.video_id,
                    true,
                )
            };
            manifest_ms += manifest_at.elapsed().as_millis() as u64;
            let Some(stream) = stream else {
                continue;
            };
            merge_subtitles(&mut subtitles, stream.subtitles);
            for mut format in stream.formats {
                if proto == "hls" {
                    format.ext = "mp4".into();
                }
                let itag = if proto == "hls" {
                    itag_of(format.url.as_str())
                } else {
                    Some(format.format_id.clone())
                };
                let language_code = format.language.clone();
                let key = (proto.to_owned(), language_code.clone());
                let entry = itags.entry(itag.clone().unwrap_or_default()).or_default();
                if !context.all_formats && entry.contains(&key) {
                    continue;
                }
                if !matches!(context.live_status, Some("is_live" | "post_live")) {
                    format
                        .extra
                        .insert("available_at".into(), response.available_at.into());
                }
                let mut source_preference = format.source_preference.unwrap_or(-1);
                if matches!(client_name, "web_safari" | "web_embedded")
                    && proto == "hls"
                    && context.live_status != Some("is_live")
                {
                    source_preference -= 1;
                }
                entry.push(key);
                let itag_proto: Vec<String> =
                    entry.iter().map(|(proto, _)| proto.clone()).collect();
                if let Some(itag) = &itag {
                    if context.all_formats || itag_proto.iter().any(|known| known != proto) {
                        format.format_id = format!("{itag}-{proto}");
                    } else {
                        format.format_id = itag.clone();
                    }
                }
                if language_code.is_some() && language_code == language.original {
                    format.format_note =
                        join_nonempty(&[format.format_note.take(), Some("(original)".into())], " ");
                    format
                        .extra
                        .insert("language_preference".into(), ORIGINAL_LANG_VALUE.into());
                } else if language_code.is_some() && language_code == language.default {
                    format.format_note =
                        join_nonempty(&[format.format_note.take(), Some("(default)".into())], " ");
                    format
                        .extra
                        .insert("language_preference".into(), DEFAULT_LANG_VALUE.into());
                }
                if matches!(itag.as_deref(), Some("616" | "235")) {
                    format.format_note =
                        join_nonempty(&[format.format_note.take(), Some("Premium".into())], " ");
                    source_preference += 100;
                }
                let base = format
                    .format_id
                    .split('-')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let mut quality = q(itag_qualities.get(&base).map(String::as_str));
                if quality == -1
                    && let Some(height) = format.height
                {
                    let nearest = res_qualities
                        .iter()
                        .min_by_key(|(known, _)| (known - i64::from(height)).abs())
                        .and_then(|(_, quality)| quality.clone());
                    quality = q(nearest.as_deref());
                }
                format.quality = Some(quality as f64);
                if context.verbose || context.all_formats {
                    format.format_note = join_nonempty(
                        &[
                            format.format_note.take(),
                            Some(short_client_name(client_name)),
                        ],
                        ", ",
                    );
                }
                if format.fps.is_some_and(|fps| fps.as_f64() <= 1.0) {
                    format.fps = None;
                }
                format.source_preference = Some(source_preference);
                if proto == "dash" {
                    let clen = format
                        .fragments
                        .first()
                        .map(|fragment| fragment.url.as_str().to_owned())
                        .unwrap_or_else(|| format.url.as_str().to_owned());
                    format.filesize = clen
                        .split("/clen/")
                        .nth(1)
                        .and_then(|rest| rest.split('/').next())
                        .and_then(|digits| digits.parse().ok());
                }
                order.push((true, manifest_formats.len()));
                manifest_formats.push(format);
            }
        }
    }
    Ok(Streams {
        manifest_ms,
        formats,
        manifest_formats,
        order,
        subtitles,
    })
}

type Solved = (HashMap<String, String>, HashMap<usize, Vec<usize>>);

/// `solve_js_challenges`: every challenge at once on first use, with
/// upstream's warnings for any left unsolved.
fn solve_once<'a>(
    solved: &'a mut Option<Solved>,
    solver: &mut dyn Solver,
    n_challenges: &BTreeSet<String>,
    s_challenges: &BTreeSet<usize>,
    video_id: &str,
) -> &'a Solved {
    solved.get_or_insert_with(|| {
        let (n, sig) = if n_challenges.is_empty() && s_challenges.is_empty() {
            (HashMap::new(), HashMap::new())
        } else {
            solver.solve(n_challenges, s_challenges)
        };
        let help = "Ensure you have a supported JavaScript runtime and challenge solver script distribution installed. Review any warnings presented before this message. For more details, refer to  https://github.com/yt-dlp/yt-dlp/wiki/EJS";
        if s_challenges.iter().any(|length| !sig.contains_key(length)) {
            crate::screen::warning_once(&format!(
                "[youtube] {video_id}: Signature solving failed: Some formats may be missing. {help}"
            ));
        }
        if n_challenges.iter().any(|challenge| !n.contains_key(challenge)) {
            crate::screen::warning_once(&format!(
                "[youtube] {video_id}: n challenge solving failed: Some formats may be missing. {help}"
            ));
        }
        (n, sig)
    })
}

/// `_extract_storyboard`: `mhtml` storyboard formats from the first
/// response's spec (the levels in reverse).
pub(super) fn storyboards(
    player_responses: &[Value],
    duration: Option<i64>,
    video_id: &str,
) -> Vec<Map<String, Value>> {
    let spec = player_responses
        .iter()
        .find_map(|response| {
            response
                .pointer("/storyboards/playerStoryboardSpecRenderer/spec")
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    let mut levels: Vec<&str> = spec.split('|').collect();
    levels.reverse();
    let base = levels.pop().filter(|base| !base.is_empty());
    let Some(base) = base
        .and_then(|base| Url::parse("https://i.ytimg.com/").ok()?.join(base).ok())
        .map(String::from)
        .and_then(|base| super::super::json_ld::url_or_none(&base))
    else {
        return Vec::new();
    };
    let Some(duration) = duration else {
        return Vec::new();
    };
    let last = levels.len() as i64 - 1;
    let mut found = Vec::new();
    for (index, level) in levels.iter().enumerate() {
        let args: Vec<&str> = level.split('#').collect();
        let counts: Vec<Option<i64>> = args
            .iter()
            .take(5)
            .map(|count| count.trim().parse::<i64>().ok())
            .collect();
        if args.len() != 8
            || counts
                .iter()
                .any(|count| count.is_none_or(|count| count == 0))
        {
            crate::screen::warning(&format!(
                "[youtube] {video_id}: Malformed storyboard {index}: {}; {}",
                args.join("#"),
                crate::BUG_REPORTS_MESSAGE
            ));
            continue;
        }
        let [width, height, frames, columns, rows] =
            [0, 1, 2, 3, 4].map(|position| counts[position].unwrap_or_default());
        let (name, sigh) = (args[6], args[7]);
        let url = base
            .replace("$L", &(last - index as i64).to_string())
            .replace("$N", name)
            + &format!("&sigh={sigh}");
        let fragment_count = frames as f64 / (columns * rows) as f64;
        let fragment_duration = duration as f64 / fragment_count;
        let fragments: Vec<Value> = (0..fragment_count.ceil() as i64)
            .map(|fragment| {
                json!({
                    "url": url.replace("$M", &fragment.to_string()),
                    "duration": fragment_duration.min(duration as f64 - fragment as f64 * fragment_duration),
                })
            })
            .collect();
        let mut format = Map::new();
        format.insert("format_id".into(), format!("sb{index}").into());
        format.insert("format_note".into(), "storyboard".into());
        format.insert("ext".into(), "mhtml".into());
        format.insert("protocol".into(), "mhtml".into());
        format.insert("acodec".into(), "none".into());
        format.insert("vcodec".into(), "none".into());
        format.insert("url".into(), url.into());
        format.insert("width".into(), width.into());
        format.insert("height".into(), height.into());
        format.insert("fps".into(), (frames as f64 / duration as f64).into());
        format.insert("rows".into(), rows.into());
        format.insert("columns".into(), columns.into());
        format.insert("fragments".into(), Value::Array(fragments));
        found.push(format);
    }
    found
}

/// `/itag/(\d+)` in a manifest format's URL.
fn itag_of(url: &str) -> Option<String> {
    let start = url.find("/itag/")? + "/itag/".len();
    let digits: String = url[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    (!digits.is_empty()).then_some(digits)
}

/// `_merge_subtitles(subs, target=subtitles)`.
fn merge_subtitles(target: &mut crate::Subtitles, source: crate::Subtitles) {
    for (language, tracks) in source.entries {
        match target
            .entries
            .iter_mut()
            .find(|(known, _)| *known == language)
        {
            Some((_, existing)) => existing.extend(tracks),
            None => target.entries.push((language, tracks)),
        }
    }
}

/// A format dict as this port's typed format (unmodelled keys in `extra`).
pub(super) fn to_format(dict: &Map<String, Value>, video_id: &str) -> Result<VideoFormat> {
    let mut value = dict.clone();
    // Typed fields must hold their own types; upstream keeps `None`s.
    for key in [
        "asr",
        "filesize",
        "fps",
        "audio_channels",
        "height",
        "width",
        "tbr",
        "language",
        "format_note",
        "vcodec",
        "acodec",
        "dynamic_range",
    ] {
        if value.get(key).is_some_and(Value::is_null) {
            value.shift_remove(key);
        }
    }
    let format: VideoFormat = serde_json::from_value(Value::Object(value))
        .map_err(|error| Error::Reported(format!("[youtube] {video_id}: {error}")))?;
    Ok(format)
}

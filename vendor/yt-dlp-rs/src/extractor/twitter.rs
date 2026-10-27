//! Upstream `TwitterIE` and `TwitterCardIE` (`yt_dlp/extractor/twitter.py`):
//! a status through the GraphQL API with a guest token (or the logged-in
//! `auth_token`/`ct0` cookies), its videos' HLS and MP4 variants, unified
//! and player cards, and the post's metadata.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Twitter;
pub struct TwitterCard;

const NAME: &str = "twitter";
const KEY: &str = "Twitter";
const CARD_NAME: &str = "twitter:card";
const API_BASE: &str = "https://api.x.com/1.1/";
const GRAPHQL_API_BASE: &str = "https://x.com/i/api/graphql/";
const GRAPHQL_ENDPOINT: &str = "2ICDjqPd81tulZcYrtpTuQ/TweetResultByRestId";
const AUTH: &str = "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";
const BASE_REGEX: &str = r"https?://(?:(?:www|m(?:obile)?)\.)?(?:(?:twitter|x)\.com|twitter3e4tixl4xyajtrzo62zg5vztmjuricljdp2c5kshju4avyoid\.onion)/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^{BASE_REGEX}(?:(?:i/web|[^/]+)/status|statuses)/(?P<id>\d+)(?:/(?:video|photo)/(?P<index>\d+))?"
    ))
    .unwrap()
});
static CARD_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^{BASE_REGEX}i/(?:cards/tfw/v1|videos(?:/tweet)?)/(?P<id>\d+)"
    ))
    .unwrap()
});
static DIMENSIONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/(?P<width>\d+)x(?P<height>\d+)/").unwrap());
static AUDIO_BITRATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^hls-[Aa]udio-(?P<bitrate>\d{4,})").unwrap());
static URL_JUNK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+(https?://[^ ]+)").unwrap());

/// `TwitterIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `TwitterCardIE._match_valid_url`.
pub fn card_suitable(url: &str) -> bool {
    CARD_URL.is_match(url)
}

fn int(value: Option<&Value>, scale: i64) -> Option<i64> {
    let value = value?;
    let number = value
        .as_i64()
        .map(|number| number as f64)
        .or_else(|| value.as_f64())
        .or_else(|| value.as_str()?.trim().parse::<f64>().ok())?;
    Some((number / scale as f64) as i64)
}

fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

fn login_required(id: &str, message: &str) -> Error {
    fail(
        id,
        &format!(
            "{message}. Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies"
        ),
    )
}

fn logged_in() -> bool {
    crate::cookies::get_cookie(API_BASE, "auth_token").is_some()
}

/// `_set_base_headers()`: the bearer token and any `ct0` CSRF token.
fn base_headers() -> Vec<(&'static str, String)> {
    let mut headers = vec![("Authorization", format!("Bearer {AUTH}"))];
    if let Some(csrf) = crate::cookies::get_cookie(API_BASE, "ct0") {
        headers.push(("x-csrf-token", csrf));
    }
    headers
}

fn read_json(response: reqwest::blocking::Response, id: &str) -> Result<Value> {
    use std::io::Read;
    let mut response = response;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    serde_json::from_slice(&body).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

fn request_error<'a>(id: &'a str, what: &'static str) -> impl Fn(Error) -> Error + 'a {
    move |error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor: NAME,
            id: id.to_owned(),
            what,
            source: Box::new(error),
        },
        error => error,
    }
}

/// `_fetch_guest_token`: a POST to `guest/activate.json`.
fn guest_token(client: &Client, id: &str) -> Result<String> {
    let url = Url::parse(&format!("{API_BASE}guest/activate.json"))?;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading guest token"));
    let headers = base_headers();
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let response = super::direct::post(client, &url, &headers, String::new())
        .map_err(request_error(id, "JSON metadata"))?;
    read_json(response, id)?
        .get("guest_token")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| fail(id, "Could not retrieve guest token"))
}

/// `_build_graphql_query` as `_call_graphql_api`'s compact JSON query.
fn graphql_query(id: &str) -> Vec<(&'static str, String)> {
    let variables = json!({
        "tweetId": id,
        "withCommunity": false,
        "includePromotedContent": false,
        "withVoice": false,
    });
    let features = json!({
        "creator_subscriptions_tweet_preview_api_enabled": true,
        "tweetypie_unmention_optimization_enabled": true,
        "responsive_web_edit_tweet_api_enabled": true,
        "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
        "view_counts_everywhere_api_enabled": true,
        "longform_notetweets_consumption_enabled": true,
        "responsive_web_twitter_article_tweet_consumption_enabled": false,
        "tweet_awards_web_tipping_enabled": false,
        "freedom_of_speech_not_reach_fetch_enabled": true,
        "standardized_nudges_misinfo": true,
        "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled": true,
        "longform_notetweets_rich_text_read_enabled": true,
        "longform_notetweets_inline_media_enabled": true,
        "responsive_web_graphql_exclude_directive_enabled": true,
        "verified_phone_label_enabled": false,
        "responsive_web_media_download_video_enabled": false,
        "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
        "responsive_web_graphql_timeline_navigation_enabled": true,
        "responsive_web_enhance_cards_enabled": false,
    });
    let toggles = json!({"withArticleRichContentState": false});
    [
        ("variables", variables),
        ("features", features),
        ("fieldToggles", toggles),
    ]
    .into_iter()
    .map(|(key, value)| (key, value.to_string()))
    .collect()
}

/// `_call_api(endpoint, id, query, graphql=True)`'s `data`.
fn graphql(client: &Client, id: &str) -> Result<Value> {
    let mut headers = base_headers();
    if logged_in() {
        headers.push(("x-twitter-auth-type", "OAuth2Session".into()));
        headers.push(("x-twitter-client-language", "en".into()));
        headers.push(("x-twitter-active-user", "yes".into()));
    } else {
        headers.push(("x-guest-token", guest_token(client, id)?));
    }
    let mut url = Url::parse(&format!("{GRAPHQL_API_BASE}{GRAPHQL_ENDPOINT}"))?;
    {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in graphql_query(id) {
            pairs.append_pair(key, &value);
        }
    }
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading GraphQL JSON"));
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let response = super::direct::request_expecting(client, &url, &headers, &[400, 401, 403, 404])
        .map_err(request_error(id, "JSON metadata"))?;
    let result = read_json(response, id)?;
    // `', '.join(set(...))`: one message, or this port's order for several.
    let mut messages: Vec<String> = Vec::new();
    for message in result
        .get("errors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|error| error.get("message").and_then(Value::as_str))
    {
        if !messages.iter().any(|known| known == message) {
            messages.push(message.to_owned());
        }
    }
    let error = messages.join(", ");
    if !messages.is_empty() || result.get("errors").is_some_and(|errors| !errors.is_null()) {
        if error.to_lowercase() == "dependency: unspecified" {
            crate::screen::debug(&format!("Ignoring Twitter API error: \"{error}\""));
        } else if error.to_lowercase().contains("not authorized") {
            return Err(login_required(
                id,
                error.strip_suffix('.').unwrap_or(&error),
            ));
        } else if !messages.is_empty() {
            return Err(fail(id, &format!("Error(s) while querying API: {error}")));
        }
    }
    Ok(result.get("data").cloned().unwrap_or(Value::Null))
}

/// `_graphql_to_legacy`: the legacy status dict of a GraphQL result.
fn graphql_to_legacy(data: &Value, id: &str) -> Result<Map<String, Value>> {
    let mut result = data
        .pointer("/tweetResult/result")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let typename = result
        .get("__typename")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if !matches!(
        typename.as_deref(),
        None | Some("Tweet" | "TweetWithVisibilityResults" | "TweetTombstone" | "TweetUnavailable")
    ) {
        crate::screen::warning_once(&format!(
            "[{NAME}] {id}: Unknown typename: {}",
            typename.as_deref().unwrap_or_default()
        ));
    }
    if result.get("tombstone").is_some() {
        let cause = result
            .pointer("/tombstone/text/text")
            .and_then(Value::as_str)
            .map(|text| text.strip_suffix(". Learn more").unwrap_or(text).to_owned());
        return Err(fail(
            id,
            &format!(
                "Twitter API says: {}",
                cause.as_deref().unwrap_or("Unknown error")
            ),
        ));
    }
    match typename.as_deref() {
        Some("TweetUnavailable") => {
            let reason = result.get("reason").and_then(Value::as_str);
            return Err(match reason {
                Some("NsfwLoggedOut" | "NsfwViewerHasNoStatedAge") => {
                    login_required(id, "NSFW tweet requires authentication")
                }
                Some("Protected") => {
                    login_required(id, "You are not authorized to view this protected tweet")
                }
                _ => fail(id, reason.unwrap_or("Requested tweet is unavailable")),
            });
        }
        Some("TweetWithVisibilityResults") => {
            result = result
                .get("tweet")
                .filter(|value| value.is_object())
                .cloned()
                .unwrap_or_else(|| json!({}));
        }
        _ => {}
    }
    let mut status = result
        .get("legacy")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (key, path) in [
        ("user", "/core/user_results/result/legacy"),
        ("card", "/card/legacy"),
        ("quoted_status", "/quoted_status_result/result/legacy"),
        (
            "retweeted_status",
            "/legacy/retweeted_status_result/result/legacy",
        ),
    ] {
        if let Some(value) = result.pointer(path).filter(|value| value.is_object()) {
            status.insert(key.into(), value.clone());
        }
    }
    if status
        .get("retweeted_status")
        .is_some_and(|value| truthy(Some(value)))
    {
        let user = status
            .get("retweeted_status_result")
            .and_then(|value| value.pointer("/result/core/user_results/result/legacy"))
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        if let Some(retweeted) = status
            .get_mut("retweeted_status")
            .and_then(Value::as_object_mut)
        {
            retweeted.insert("user".into(), user);
        }
    }
    let bindings: Map<String, Value> = status
        .get("card")
        .and_then(|card| card.get("binding_values"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|value| value.is_object())
        .map(|binding| {
            (
                binding
                    .get("key")
                    .and_then(Value::as_str)
                    .unwrap_or("None")
                    .to_owned(),
                binding.get("value").cloned().unwrap_or(Value::Null),
            )
        })
        .collect();
    if !bindings.is_empty()
        && let Some(card) = status.get_mut("card").and_then(Value::as_object_mut)
    {
        card.insert("binding_values".into(), Value::Object(bindings));
    }
    if let Some(views) = int(result.pointer("/views/count"), 1) {
        status.insert("view_count".into(), views.into());
    }
    Ok(status)
}

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

/// `_extract_variant_formats`.
fn variant_formats(
    client: &Client,
    variant: &Value,
    id: &str,
    subtitles: &mut crate::Subtitles,
) -> Result<Vec<crate::VideoFormat>> {
    let Some(url) = variant
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
    else {
        return Ok(Vec::new());
    };
    if url.contains(".m3u8") {
        let Ok(manifest) = Url::parse(url) else {
            return Ok(Vec::new());
        };
        let Some(mut stream) =
            super::extract_m3u8_optional(client, &manifest, NAME, id, Some("hls"), true)
        else {
            return Ok(Vec::new());
        };
        for format in &mut stream.formats {
            if format.vcodec.as_deref() == Some("none")
                && format.tbr.is_none()
                && let Some(found) = AUDIO_BITRATE.captures(&format.format_id)
            {
                format.tbr = found["bitrate"]
                    .parse::<i64>()
                    .ok()
                    .map(|rate| (rate / 1000) as f64);
                crate::info_order::mark_format_update(format, "tbr");
                crate::info_order::mark_int_rates(format);
            }
        }
        super::html5::merge_subtitles(subtitles, stream.subtitles);
        return Ok(stream.formats);
    }
    let tbr = int(
        variant.get("bitrate").or_else(|| variant.get("bit_rate")),
        1000,
    )
    .filter(|tbr| *tbr != 0);
    let mut format = Map::new();
    format.insert("url".into(), url.into());
    format.insert(
        "format_id".into(),
        match tbr {
            Some(tbr) => format!("http-{tbr}"),
            None => "http".into(),
        }
        .into(),
    );
    if let Some(tbr) = tbr {
        format.insert("tbr".into(), tbr.into());
    }
    // Processing's `determine_ext(url)` (after the extractor's keys).
    format.insert(
        "ext".into(),
        super::direct::determine_ext(url)
            .unwrap_or_else(|| "unknown_video".into())
            .into(),
    );
    if let Some(found) = DIMENSIONS.captures(url) {
        format.insert(
            "width".into(),
            found["width"].parse::<i64>().unwrap_or(0).into(),
        );
        format.insert(
            "height".into(),
            found["height"].parse::<i64>().unwrap_or(0).into(),
        );
    }
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
        .map_err(|error| fail(id, &error.to_string()))?;
    crate::info_order::mark_format(&mut format, "twitter-http");
    crate::info_order::mark_int_rates(&mut format);
    Ok(vec![format])
}

/// `extract_from_video_info`.
fn video_info(client: &Client, media: &Value, twid: &str) -> Result<Media> {
    let media_id = str_or_none(media.get("id_str")).or_else(|| str_or_none(media.get("id")));
    crate::screen::debug(&format!(
        "Extracting from video info: {}",
        media_id.as_deref().unwrap_or("None")
    ));
    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    for variant in media
        .pointer("/video_info/variants")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        formats.extend(variant_formats(client, variant, twid, &mut subtitles)?);
    }
    let mut thumbnails = Vec::new();
    let media_url = media
        .get("media_url_https")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .or_else(|| {
            media
                .get("media_url")
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
        });
    if let Some(media_url) = media_url.and_then(|url| Url::parse(url).ok()) {
        let mut add = |name: &str, size: &Value| {
            let dimension = |short: &str, long: &str| {
                int(
                    size.get(short)
                        .filter(|value| truthy(Some(value)))
                        .or_else(|| size.get(long)),
                    1,
                )
            };
            thumbnails.push(json!({
                "id": name,
                "url": super::update_url_query(&media_url, &[("name", name)]).as_str(),
                "width": dimension("w", "width"),
                "height": dimension("h", "height"),
            }));
        };
        for (name, size) in media
            .get("sizes")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            add(name, size);
        }
        add("orig", media.get("original_info").unwrap_or(&json!({})));
    }
    let duration = media
        .pointer("/video_info/duration_millis")
        .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
        .map(|millis| millis / 1000.0);
    let mut fields = Map::new();
    fields.insert("id".into(), media_id.into());
    fields.insert("formats".into(), Value::Null);
    fields.insert("subtitles".into(), Value::Null);
    fields.insert("thumbnails".into(), Value::Array(thumbnails));
    fields.insert(
        "duration".into(),
        duration.map_or(Value::Null, |d| json!(d)),
    );
    fields.insert(
        "_format_sort_fields".into(),
        json!(["res", "proto:m3u8", "br", "size"]),
    );
    Ok(Media::Video {
        fields,
        formats,
        subtitles,
    })
}

/// One of `selected_entries`.
enum Media {
    Video {
        fields: Map<String, Value>,
        formats: Vec<crate::VideoFormat>,
        subtitles: crate::Subtitles,
    },
    /// A card's `{'_type': 'url', 'url': ...}`.
    Url(String),
}

/// `get_binding_value(k)`.
fn binding(bindings: &Value, key: &str) -> Option<Value> {
    let value = bindings.get(key)?;
    let kind = value.get("type")?.as_str()?.to_lowercase();
    value.get(format!("{kind}_value")).cloned()
}

/// `extract_from_card_info`.
fn card_info(client: &Client, card: &Value, twid: &str) -> Result<Vec<Media>> {
    if !truthy(Some(card)) {
        return Ok(Vec::new());
    }
    let name = card.get("name").and_then(Value::as_str).unwrap_or_default();
    let name = name.rsplit(':').next().unwrap_or(name);
    crate::screen::debug(&format!(
        "Extracting from {name} card info: {}",
        card.get("url").and_then(Value::as_str).unwrap_or("None")
    ));
    let bindings = card.get("binding_values").cloned().unwrap_or(Value::Null);
    let text =
        |key: &str| binding(&bindings, key).and_then(|value| value.as_str().map(str::to_owned));
    match name {
        "player" => Ok(text("player_url").map(Media::Url).into_iter().collect()),
        "summary" => Ok(text("card_url").map(Media::Url).into_iter().collect()),
        "periscope_broadcast" | "broadcast" | "audiospace" => Err(Error::UnsupportedHtml5(
            format!("Twitter {name} cards are not supported natively"),
        )),
        "unified_card" => {
            let unified: Value = text("unified_card")
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or(Value::Null);
            let mut found = Vec::new();
            for media in unified
                .get("media_entities")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|entities| entities.values())
                .filter(|media| media.get("type").and_then(Value::as_str) == Some("video"))
            {
                found.push(video_info(client, media, twid)?);
            }
            Ok(found)
        }
        _ => Err(Error::UnsupportedHtml5(format!(
            "Twitter {name} (VMAP) cards are not supported natively"
        ))),
    }
}

/// A video result in `{**info, **data, 'display_id': twid}` order.
fn video(
    url: &Url,
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
    subtitles: crate::Subtitles,
) -> VideoInfo {
    let first = formats
        .first()
        .map_or_else(|| url.clone(), |format| format.url.clone());
    let mut info = super::direct::build_at(first.clone(), &first, String::new());
    info.direct = None;
    if formats.is_empty() {
        info.extra
            .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
    }
    info.formats = formats;
    info.subtitles = subtitles;
    info.webpage_url = url.clone();
    info.extractor = NAME.into();
    info.extractor_key = KEY.into();
    let mut keys = Vec::new();
    for (key, value) in dict {
        keys.push(key.clone());
        match key.as_str() {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            "formats" | "subtitles" => {}
            "_format_sort_fields" => {
                info.format_sort_fields = value
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect();
            }
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    crate::info_order::mark_info(
        info,
        &format!("dict;{};thumbnail=id,url,width,height", keys.join(",")),
    )
}

/// `url_result(url)`, followed by the registry.
fn follow(target: &str) -> Result<ExtractResult> {
    let target = Url::parse(target)?;
    let mut info = super::direct::build_at(target.clone(), &target, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra
        .insert(crate::FOLLOW_URL.into(), target.as_str().into());
    Ok(ExtractResult::Video(info))
}

impl Twitter {
    fn status(&self, client: &Client, twid: &str) -> Result<Map<String, Value>> {
        let status = graphql_to_legacy(&graphql(client, twid)?, twid)?;
        Ok(status
            .get("retweeted_status")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or(status))
    }
}

impl Extractor for Twitter {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let twid = found["id"].to_owned();
        let selected = found.name("index").map(|index| index.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let status = Value::Object(self.status(client, &twid)?);
        let description = ["full_text", "text"]
            .iter()
            .find_map(|key| status.get(*key).and_then(Value::as_str))
            .map(|text| text.replace('\n', " "))
            .filter(|text| !text.is_empty())
            .unwrap_or_default();
        let stripped = URL_JUNK.replace_all(&description, "");
        let chars: Vec<char> = stripped.chars().collect();
        let mut title = if chars.len() <= 72 {
            stripped.into_owned()
        } else {
            format!("{}...", chars[..69].iter().collect::<String>())
        };
        let user = status.get("user").cloned().unwrap_or_else(|| json!({}));
        let uploader = user
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty());
        if let Some(uploader) = uploader {
            title = format!("{uploader} - {title}");
        }
        let uploader_id = user.get("screen_name").and_then(Value::as_str);
        let mut info = Map::new();
        info.insert("id".into(), twid.clone().into());
        info.insert("title".into(), title.clone().into());
        info.insert("description".into(), description.into());
        info.insert(
            "uploader".into(),
            user.get("name").cloned().unwrap_or(Value::Null),
        );
        info.insert(
            "timestamp".into(),
            status
                .get("created_at")
                .and_then(Value::as_str)
                .and_then(|date| crate::dates::unified_timestamp(date, false))
                .into(),
        );
        info.insert(
            "channel_id".into(),
            str_or_none(status.get("user_id_str"))
                .or_else(|| str_or_none(user.get("id_str")))
                .into(),
        );
        info.insert("uploader_id".into(), uploader_id.into());
        info.insert(
            "uploader_url".into(),
            uploader_id
                .map(|id| format!("https://twitter.com/{id}"))
                .into(),
        );
        for (key, source) in [
            ("view_count", "view_count"),
            ("like_count", "favorite_count"),
            ("repost_count", "retweet_count"),
            ("comment_count", "reply_count"),
        ] {
            info.insert(key.into(), int(status.get(source), 1).into());
        }
        info.insert(
            "age_limit".into(),
            if truthy(status.get("possibly_sensitive")) {
                18
            } else {
                0
            }
            .into(),
        );
        let tags: Vec<Value> = status
            .pointer("/entities/hashtags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tag| tag.get("text").cloned())
            .collect();
        info.insert("tags".into(), Value::Array(tags));

        let videos: Vec<Value> = [Some(&status), status.get("quoted_status")]
            .into_iter()
            .flatten()
            .filter_map(|source| source.pointer("/extended_entities/media"))
            .filter_map(Value::as_array)
            .flatten()
            .filter(|media| {
                media.is_object() && media.get("type").and_then(Value::as_str) != Some("photo")
            })
            .cloned()
            .collect();
        let archive_id = json!([format!("twitter {twid}")]);

        // `_yes_playlist(twid, selected_index, video_label=...)`.
        let all = match &selected {
            None => true,
            Some(index) if super::no_playlist() => {
                crate::screen::screen(&format!(
                    "[{NAME}] Downloading just the URL-specified video number {index} because of --no-playlist"
                ));
                false
            }
            Some(index) => {
                crate::screen::screen(&format!(
                    "[{NAME}] Downloading playlist {twid} - add --no-playlist to download just the URL-specified video number {index}"
                ));
                true
            }
        };
        if !all {
            let index: usize = selected.as_deref().unwrap_or("1").parse().unwrap_or(0);
            let all_media: Vec<&Value> = [Some(&status), status.get("quoted_status")]
                .into_iter()
                .flatten()
                .filter_map(|source| source.pointer("/extended_entities/media"))
                .filter_map(Value::as_array)
                .filter_map(|media| media.get(index.wrapping_sub(1)))
                .filter(|media| media.is_object())
                .collect();
            let Some(desired) = all_media.first().copied() else {
                return Err(fail(&twid, &format!("Video #{index} is unavailable")));
            };
            if desired.get("type").and_then(Value::as_str) != Some("video") {
                return Err(fail(&twid, &format!("Media #{index} is not a video")));
            }
            for (position, entry) in videos.iter().enumerate() {
                if entry.get("id") != desired.get("id") {
                    continue;
                }
                if position == 0 {
                    info.insert("_old_archive_ids".into(), archive_id.clone());
                }
                if videos.len() != 1 {
                    title = format!("{title} #{}", position + 1);
                    info.insert("title".into(), title.clone().into());
                }
                break;
            }
            let Media::Video {
                fields,
                formats,
                subtitles,
            } = video_info(client, desired, &twid)?
            else {
                unreachable!("video info is a video");
            };
            let mut dict = info;
            dict.extend(fields);
            dict.insert("display_id".into(), twid.clone().into());
            return Ok(Some(ExtractResult::Video(video(
                url, dict, formats, subtitles,
            ))));
        }

        let mut selected_entries = Vec::new();
        for media in &videos {
            selected_entries.push(video_info(client, media, &twid)?);
        }
        if let Some(card) = status.get("card") {
            selected_entries.extend(card_info(client, card, &twid)?);
        }
        if selected_entries.is_empty() {
            let expanded = status
                .pointer("/entities/urls/0/expanded_url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none);
            return match expanded {
                // `url_result(expanded_url, display_id=twid, **info)`: a
                // `url` result, whose fields processing does not keep.
                Some(expanded) if expanded != url.as_str() => follow(&expanded).map(Some),
                _ => Err(fail(&twid, "No video could be found in this tweet")),
            };
        }
        if selected_entries.len() == 1
            && let Some(Media::Url(target)) = selected_entries.first()
        {
            return follow(target).map(Some);
        }
        if selected_entries
            .iter()
            .any(|entry| matches!(entry, Media::Url(_)))
        {
            return Err(Error::UnsupportedHtml5(
                "tweets mixing videos and linked cards are not supported natively".into(),
            ));
        }
        let several = selected_entries.len() > 1;
        let mut entries = Vec::new();
        for (index, entry) in selected_entries.into_iter().enumerate() {
            let Media::Video {
                fields,
                formats,
                subtitles,
            } = entry
            else {
                continue;
            };
            let mut dict = info.clone();
            dict.extend(fields);
            dict.insert("display_id".into(), twid.clone().into());
            if index == 0 {
                dict.insert("_old_archive_ids".into(), archive_id.clone());
            }
            if several {
                dict.insert("title".into(), format!("{title} #{}", index + 1).into());
            }
            entries.push(video(url, dict, formats, subtitles));
        }
        if !several {
            return Ok(entries.pop().map(ExtractResult::Video));
        }
        // `playlist_result(entries, **info)`.
        let keys: Vec<String> = info.keys().cloned().collect();
        let mut extra = info;
        extra.shift_remove("id");
        extra.shift_remove("title");
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: twid,
            title,
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: KEY.into(),
            entries,
            extra,
        })))
    }
}

impl Extractor for TwitterCard {
    fn key(&self) -> &'static str {
        CARD_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = CARD_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(CARD_NAME);
        crate::screen::screen(&format!(
            "[{CARD_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        follow(&format!("https://twitter.com/statuses/{}", &found["id"])).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://twitter.com/freethenipple/status/643211948184596480"
        ));
        assert!(suitable("https://x.com/i/web/status/910031516746514432"));
        assert!(suitable("https://mobile.twitter.com/a/status/1/video/2"));
        assert!(!suitable("https://twitter.com/freethenipple"));
        assert!(card_suitable(
            "https://twitter.com/i/cards/tfw/v1/560070183650213889"
        ));
        assert!(card_suitable(
            "https://twitter.com/i/videos/tweet/705235433198714880"
        ));
    }
}

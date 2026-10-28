//! Upstream `OdnoklassnikiIE` (`yt_dlp/extractor/odnoklassniki.py`): the
//! video/embed page's player `data-options` (stub errors, restricted access,
//! external players as `url` results), the flashvars metadata (or its
//! metadata URL), progressive, HLS, DASH, live and RTMP formats with
//! upstream's type qualities, subtitles, page meta fields and YouTube-hosted
//! videos as `url_transparent` results.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Odnoklassniki;

const NAME: &str = "Odnoklassniki";
const LOGIN_HINT: &str = "Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies";
const QUALITIES: [&str; 8] = ["4", "0", "1", "2", "3", "5", "6", "7"];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?(?:odnoklassniki|ok)\.ru/(?:video(?P<embed>embed)?/|web-api/video/moviePlayer/|live/|dk\?.*?st\.mvId=)(?P<id>[\d-]+)")
        .unwrap()
});
static TYPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\btype[/=](\d)").unwrap());

/// `OdnoklassnikiIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

fn key_error(id: &str, key: &str) -> Error {
    fail(
        id,
        &format!(
            "An extractor error has occurred. (caused by KeyError('{key}')); {}",
            crate::BUG_REPORTS_MESSAGE
        ),
    )
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn truthy(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| match value {
        Value::Null | Value::Bool(false) => false,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(items) => !items.is_empty(),
        Value::Bool(true) => true,
    })
}

/// `_clear_cookies(cdn_url)`: CDN cookies break direct downloads.
fn clear_cookies(cdn_url: &str) {
    crate::cookies::clear_domain(".mycdn.me");
    if let Some(host) = Url::parse(cdn_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
    {
        crate::cookies::clear_domain(&host);
    }
}

/// The player JSON from `data-options="{...ID...}"` (either quote).
fn player_options(webpage: &str, video_id: &str) -> Option<String> {
    let id = regex::escape(video_id);
    [
        Regex::new(&format!(r#"(?s)data-options="(\{{.+?{id}.+?\}})""#)),
        Regex::new(&format!(r"(?s)data-options='(\{{.+?{id}.+?\}})'")),
    ]
    .into_iter()
    .flatten()
    .filter_map(|pattern| {
        pattern.captures(webpage).map(|found| {
            (
                found.get(0).map_or(0, |all| all.start()),
                found[1].to_owned(),
            )
        })
    })
    .min_by_key(|(start, _)| *start)
    .map(|(_, json)| json)
}

impl Extractor for Odnoklassniki {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let start_time = page
            .query_pairs()
            .find(|(key, _)| key == "fromTime")
            .and_then(|(_, value)| value.trim().parse::<i64>().ok());
        let video_id = found["id"].to_owned();
        let mode = if found.name("embed").is_some() {
            "videoembed"
        } else {
            "video"
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        let url = Url::parse(&format!("https://ok.ru/{mode}/{video_id}"))?;
        let webpage = super::download_webpage(client, &url, NAME, &video_id)?;
        let stub =
            crate::html_elements::elements_text_and_html_by_class("vp_video_stub_txt", &webpage)
                .ok()
                .and_then(|elements| elements.into_iter().next())
                .map(|(text, _)| text);
        if let Some(error) = stub.filter(|error| !error.is_empty()) {
            if error == "The author of this video has not been found or is blocked"
                && mode == "videoembed"
            {
                // Upstream calls a missing `_extract_desktop` here.
                return Err(Error::Reported(
                    "'OdnoklassnikiIE' object has no attribute '_extract_desktop'".into(),
                ));
            }
            return Err(fail(&video_id, &error));
        }
        if webpage.contains(">Access to this video is restricted</div>") {
            return Err(fail(
                &video_id,
                &format!("This video is only available for registered users. {LOGIN_HINT}"),
            ));
        }
        let Some(options) = player_options(&webpage, &video_id) else {
            return Err(fail(
                &video_id,
                &format!("Unable to extract player; {}", crate::BUG_REPORTS_MESSAGE),
            ));
        };
        let player: Value =
            serde_json::from_str(&crate::html::unescape_html(&options)).map_err(|error| {
                fail(
                    &video_id,
                    &format!("Failed to parse JSON  (caused by {error})"),
                )
            })?;
        if truthy(player.get("isExternalPlayer")).is_some()
            && let Some(target) = truthy(player.get("url")).and_then(Value::as_str)
        {
            let target = Url::parse(target)?;
            let mut follow = super::direct::build_at(page.clone(), page, String::new());
            follow.formats.clear();
            follow.direct = None;
            follow
                .extra
                .insert(crate::FOLLOW_URL.into(), target.as_str().into());
            return Ok(Some(ExtractResult::Video(follow)));
        }
        let flashvars = player
            .get("flashvars")
            .ok_or_else(|| key_error(&video_id, "flashvars"))?;
        let metadata = match truthy(flashvars.get("metadata")) {
            Some(Value::String(_)) => {
                return Err(Error::Reported(
                    "string indices must be integers, not 'str'".into(),
                ));
            }
            Some(metadata) => metadata.clone(),
            None => {
                let mut body = url::form_urlencoded::Serializer::new(String::new());
                if let Some(location) = truthy(flashvars.get("location")).and_then(Value::as_str) {
                    body.append_pair("st.location", location);
                }
                let metadata_url = flashvars
                    .get("metadataUrl")
                    .ok_or_else(|| key_error(&video_id, "metadataUrl"))?
                    .as_str()
                    .unwrap_or_default();
                let decoded = percent_encoding::percent_decode_str(metadata_url)
                    .decode_utf8_lossy()
                    .into_owned();
                let metadata_url = Url::parse(&decoded)?;
                crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading metadata JSON"));
                let mut response = super::direct::post(
                    client,
                    &metadata_url,
                    &[("Content-Type", "application/x-www-form-urlencoded")],
                    body.finish(),
                )
                .map_err(|error| match error {
                    Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                        extractor: NAME,
                        id: video_id.clone(),
                        what: "JSON metadata",
                        source: Box::new(error),
                    },
                    error => error,
                })?;
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut response, &mut bytes)
                    .map_err(Error::ExtractionRead)?;
                serde_json::from_slice(&bytes).map_err(|error| {
                    fail(
                        &video_id,
                        &format!("Failed to parse JSON  (caused by {error})"),
                    )
                })?
            }
        };
        let movie = metadata
            .get("movie")
            .ok_or_else(|| key_error(&video_id, "movie"))?;
        let provider = metadata.get("provider").and_then(Value::as_str);
        let title = if provider == Some("UPLOADED_ODKL") {
            Some(
                movie
                    .get("title")
                    .ok_or_else(|| key_error(&video_id, "title"))?
                    .clone(),
            )
        } else {
            movie.get("title").cloned()
        };
        let author = metadata.get("author").cloned().unwrap_or_else(|| json!({}));
        let upload_date = crate::html::html_search_meta(&webpage, "ya:ovs:upload_date")
            .and_then(|date| crate::dates::unified_strdate(&date, true));
        let age_limit = crate::html::html_search_meta(&webpage, "ya:ovs:adult")
            .filter(|adult| !adult.is_empty())
            .map(|adult| if adult == "true" { 18 } else { 0 });
        let mut subtitles = crate::Subtitles::default();
        for sub in movie
            .get("subtitleTracks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|sub| sub.is_object())
        {
            let Some(url) = truthy(sub.get("url")).and_then(Value::as_str) else {
                continue;
            };
            let language = truthy(sub.get("language"))
                .and_then(Value::as_str)
                .unwrap_or("en")
                .to_owned();
            let track: crate::SubtitleFormat =
                serde_json::from_value(json!({"url": url, "ext": "vtt"}))
                    .map_err(|error| fail(&video_id, &error.to_string()))?;
            let mut single = crate::Subtitles::default();
            single.entries.push((Some(language), vec![track]));
            super::html5::merge_subtitles(&mut subtitles, single);
        }
        let mut dict = Map::new();
        dict.insert("id".into(), video_id.clone().into());
        dict.insert("title".into(), title.clone().unwrap_or(Value::Null));
        dict.insert(
            "thumbnail".into(),
            movie.get("poster").cloned().unwrap_or(Value::Null),
        );
        dict.insert("duration".into(), int(movie.get("duration")).into());
        dict.insert("upload_date".into(), upload_date.into());
        dict.insert(
            "uploader".into(),
            author.get("name").cloned().unwrap_or(Value::Null),
        );
        dict.insert(
            "uploader_id".into(),
            author.get("id").cloned().unwrap_or(Value::Null),
        );
        dict.insert("like_count".into(), int(metadata.get("likeCount")).into());
        dict.insert("age_limit".into(), age_limit.into());
        dict.insert("start_time".into(), start_time.into());
        dict.insert("subtitles".into(), json!({}));

        if provider == Some("USER_YOUTUBE") {
            let content = movie
                .get("contentId")
                .ok_or_else(|| key_error(&video_id, "contentId"))?
                .as_str()
                .unwrap_or_default()
                .to_owned();
            let Ok(target) = Url::parse(&content) else {
                return Err(Error::UnsupportedHtml5(format!(
                    "OK.ru YouTube video {content:?} is not a URL"
                )));
            };
            let mut fields = dict;
            fields.shift_remove("id");
            fields.shift_remove("subtitles");
            let mut follow = super::direct::build_at(page.clone(), page, String::new());
            follow.formats.clear();
            follow.direct = None;
            follow.extra.insert(
                crate::FOLLOW_TRANSPARENT.into(),
                json!({"url": target.as_str(), "fields": Value::Object(fields)}),
            );
            return Ok(Some(ExtractResult::Video(follow)));
        }
        if title
            .as_ref()
            .is_none_or(|title| truthy(Some(title)).is_none())
        {
            return Err(Error::Reported(String::new()));
        }

        let quality = |url: &str| -> Option<i64> {
            let kind = TYPE.captures(url).map(|found| found[1].to_owned())?;
            Some(
                QUALITIES
                    .iter()
                    .position(|item| *item == kind)
                    .map_or(-1, |index| index as i64),
            )
        };
        let mut formats: Vec<crate::VideoFormat> = Vec::new();
        for video in metadata
            .get("videos")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(url) = video
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            let mut map = Map::new();
            map.insert("url".into(), url.clone().into());
            map.insert("ext".into(), "mp4".into());
            map.insert(
                "format_id".into(),
                video.get("name").cloned().unwrap_or(Value::Null),
            );
            let mut keys: Vec<String> = map.keys().cloned().collect();
            if let Some(quality) = quality(&url) {
                map.insert("quality".into(), quality.into());
                keys.push("quality".into());
            }
            map.retain(|_, value| !value.is_null());
            if !map.contains_key("format_id") {
                map.insert("format_id".into(), "".into());
            }
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                .map_err(|error| fail(&video_id, &error.to_string()))?;
            crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
            crate::info_order::mark_int_quality(&mut format);
            formats.push(format);
        }
        let mut manifests: Vec<crate::VideoFormat> = Vec::new();
        if let Some(m3u8) = ["hlsManifestUrl", "ondemandHls"]
            .iter()
            .find_map(|key| metadata.get(*key).filter(|value| !value.is_null()))
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        {
            if let Some(stream) = Url::parse(m3u8).ok().and_then(|manifest| {
                super::extract_m3u8_optional(client, &manifest, NAME, &video_id, Some("hls"), false)
            }) {
                manifests.extend(stream.formats);
            }
            clear_cookies(m3u8);
        }
        for (mpd_id, key) in [("dash", "ondemandDash"), ("webm", "metadataWebmUrl")] {
            if let Some(mpd) = truthy(metadata.get(key)).and_then(Value::as_str) {
                if let Some(stream) = Url::parse(mpd).ok().and_then(|manifest| {
                    super::extract_mpd_optional_note(
                        client,
                        &manifest,
                        NAME,
                        &video_id,
                        Some(mpd_id),
                        "Downloading MPD manifest",
                    )
                }) {
                    manifests.extend(stream.formats);
                }
                clear_cookies(mpd);
            }
        }
        if truthy(metadata.get("metadataEmbedded")).is_some() {
            return Err(Error::UnsupportedHtml5(
                "OK.ru embedded DASH manifests are not supported natively".into(),
            ));
        }
        for mut format in manifests {
            if let Some(quality) = quality(format.url.as_str()) {
                format.quality = Some(quality as f64);
                crate::info_order::mark_int_quality(&mut format);
            }
            formats.push(format);
        }
        if let Some(m3u8) = truthy(metadata.get("hlsMasterPlaylistUrl")).and_then(Value::as_str) {
            if let Some(stream) = Url::parse(m3u8).ok().and_then(|manifest| {
                super::extract_m3u8_optional(client, &manifest, NAME, &video_id, Some("hls"), false)
            }) {
                formats.extend(stream.formats);
            }
            clear_cookies(m3u8);
        }
        if let Some(rtmp) = truthy(metadata.get("rtmpUrl")).and_then(Value::as_str) {
            let map = json!({"url": rtmp, "format_id": "rtmp", "ext": "flv"});
            let mut format: crate::VideoFormat =
                serde_json::from_value(map).map_err(|error| fail(&video_id, &error.to_string()))?;
            crate::info_order::mark_format(&mut format, "dict:url,format_id,ext");
            formats.push(format);
        }
        if formats.is_empty() && truthy(metadata.get("paymentInfo")).is_some() {
            return Err(fail(
                &video_id,
                "This video is paid, subscribe to download it",
            ));
        }
        dict.insert("formats".into(), json!([]));

        let first = formats
            .first()
            .map_or_else(|| page.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.direct = None;
        if formats.is_empty() {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        info.formats = formats;
        info.subtitles = subtitles;
        info.webpage_url = page.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let mut keys = Vec::new();
        for (key, value) in dict {
            keys.push(key.clone());
            match key.as_str() {
                "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
                "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
                "formats" | "subtitles" => {}
                _ if value.is_null() => {
                    info.extra.insert(key, Value::Null);
                }
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
        }
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info,
            &format!("dict;{};subtitle=url,ext", keys.join(",")),
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://ok.ru/video/1484130554189"));
        assert!(suitable("http://ok.ru/videoembed/20648036891"));
        assert!(suitable("https://ok.ru/live/484531969818"));
        assert!(suitable(
            "http://www.ok.ru/dk?cmd=videoPlayerMetadata&mid=7157617216&st.mvId=62036049272859-0"
        ));
        assert!(!suitable("https://ok.ru/group/123"));
    }
}

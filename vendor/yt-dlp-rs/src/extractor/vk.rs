//! Upstream `VKIE` (`yt_dlp/extractor/vk.py`): `al_video.php`'s `show`
//! payload for video/clip pages and `video_ext.php` embeds (with their
//! error pages), the player params' progressive, HLS, DASH and RTMP
//! formats, subtitles, chapters and metadata. A YouTube embed is followed;
//! other embedded players and VK's JS challenge are reported as unsupported.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Vk;

const NAME: &str = "vk";
const KEY: &str = "VK";
const LOGIN_HINT: &str = "Use --cookies, --cookies-from-browser, --username and --password, --netrc-cmd, or --netrc (vk) to provide account credentials. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^https?://(?:",
        r"(?:(?:(?:m|new|vksport)\.)?vk(?:(?:video)?\.ru|\.com)/video_|(?:www\.)?daxab\.com/)",
        r"ext\.php\?(?P<embed_query>.*?\boid=(?P<oid>-?\d+).*?\bid=(?P<id>\d+).*)|",
        r"(?:(?:(?:m|new|vksport)\.)?vk(?:(?:video)?\.ru|\.com)/(?:.+?\?.*?z=)?(?:video|clip)|(?:www\.)?daxab\.com/embed/)",
        r"(?P<videoid>-?\d+_\d+)(?:.*\blist=(?P<list_id>([\da-f]+)|(ln-[\da-zA-Z]+)))?",
        r")"
    ))
    .unwrap()
});
static ERROR_MESSAGES: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"(?s)<!><div[^>]+class="video_layer_message"[^>]*>(.+?)</div>"#).unwrap(),
        Regex::new(r#"(?s)<div[^>]+id="video_ext_msg"[^>]*>(.+?)</div>"#).unwrap(),
    ]
});
static SECURITY_CHECK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<!>/login\.php\?.*\bact=security_check").unwrap());
static ERRORS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    const COPYRIGHT: &str =
        "Video %s has been removed from public access due to rightholder complaint.";
    const BLOCKED: &str = "Video %s is no longer available, because its author has been blocked.";
    [
        (
            r">Видеозапись .*? была изъята из публичного доступа в связи с обращением правообладателя.<",
            COPYRIGHT,
        ),
        (
            r">The video .*? was removed from public access by request of the copyright holder.<",
            COPYRIGHT,
        ),
        (
            r"<!>Please log in or <",
            "Video %s is only available for registered users, use --username and --password options to provide account credentials.",
        ),
        (r"<!>Unknown error", "Video %s does not exist."),
        (
            r"<!>Видео временно недоступно",
            "Video %s is temporarily unavailable.",
        ),
        (r"<!>Access denied", "Access denied to video %s."),
        (
            r"<!>Видеозапись недоступна, так как её автор был заблокирован.",
            BLOCKED,
        ),
        (
            r"<!>This video is no longer available, because its author has been blocked.",
            BLOCKED,
        ),
        (
            r"<!>This video is no longer available, because it has been deleted.",
            "Video %s is no longer available, because it has been deleted.",
        ),
        (
            r"<!>The video .+? is not available in your region.",
            "Video %s is not available in your region.",
        ),
    ]
    .into_iter()
    .map(|(pattern, message)| (Regex::new(pattern).unwrap(), message))
    .collect()
});
static PLAYER_PARAMS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)var\s+playerParams\s*=\s*(\{.+?\})\s*;\s*\n").unwrap());
/// Embedded players upstream hands to extractors this port lacks.
static OTHER_EMBEDS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (
            r#"<iframe[^>]+?src=["'](?:https?:)?//player\.vimeo\.com/video/\d+"#,
            "Vimeo",
        ),
        (
            r#"<embed[^>]+?src=["'](?:https?:)?//(?:www\.)?vimeo\.com/moogaloop\.swf"#,
            "Vimeo",
        ),
        (
            r#"<video[^>]+src=["'](?:https?:)?//(?:www\.)?vimeo\.com/[0-9]+["']"#,
            "Vimeo",
        ),
        (
            r#"\ssrc="(?:https?:)?//rutube\.ru\\?/(?:video|play)\\?/embed"#,
            "Rutube",
        ),
        (
            r#"(?i)<(?:(?:embed|iframe)[^>]+?src=|input[^>]+id=['"]dmcloudUrlEmissionSelect['"][^>]+value=)["'](?:https?:)?//(?:(?:(?:www|touch|geo)\.)?dailymotion\.[a-z]{2,3}|dai\.ly|(?:www\.)?dailymotion\.com/(?:embed/video|player\.html\?video=))"#,
            "Dailymotion",
        ),
        (
            r#"(?s)DM\.player\([^,]+,\s*\{.*?video['"]?\s*:\s*["']?[0-9a-zA-Z]+.+?\}\s*\);"#,
            "Dailymotion",
        ),
        (
            r#"<iframe[^>]+src=["'](?:https?:)?//(?:odnoklassniki|ok)\.ru/videoembed/"#,
            "Odnoklassniki",
        ),
        (
            r#"<iframe\b[^>]+\bsrc=["'](?:https?:)?//video\.sibnet\.ru/shell\.php\?.*?\bvideoid=\d+"#,
            "SibnetEmbed",
        ),
    ]
    .into_iter()
    .map(|(pattern, name)| (Regex::new(pattern).unwrap(), name))
    .collect()
});
static OPTS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)var\s+opts\s*=\s*(\{.+?\});").unwrap());
static OPTS_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"url\s*:\s*'([^']+)").unwrap());
static DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"class=["']mv_info_date[^>]+>([^<]+)(?:<|from)"#).unwrap());
static VIEWS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"class=["']mv_views_count[^>]+>\s*([\d,.]+)"#).unwrap());
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?:url|cache)(\d+)").unwrap());

/// `VKIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// An error with upstream's temp ID (`None` for video/clip page URLs).
fn fail(id: Option<&str>, message: &str) -> Error {
    Error::Reported(match id {
        Some(id) => format!("[{NAME}] {id}: {message}"),
        None => format!("[{NAME}] {message}"),
    })
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn unescaped(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(crate::html::unescape_html)
}

fn cleaned(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(crate::html::clean_html)
        .filter(|text| !text.is_empty())
}

/// `VKBaseIE._download_webpage_handle`'s JS challenge, which needs upstream's
/// `JSInterpreter`.
fn check_challenge(final_url: &Url, video_id: &str) -> Result<()> {
    if final_url.path() != "/challenge.html" {
        return Ok(());
    }
    crate::screen::screen(&format!(
        "[{NAME}] [{video_id}] Received a JS challenge response"
    ));
    Err(Error::UnsupportedHtml5(
        "VK JS challenges are not supported natively".into(),
    ))
}

/// `_download_payload('al_video', video_id, data)`.
fn download_payload(client: &Client, video_id: &str, list_id: Option<&str>) -> Result<Value> {
    use std::io::Read;
    let endpoint = Url::parse("https://vk.com/al_video.php")?;
    let mut body = url::form_urlencoded::Serializer::new(String::new());
    body.append_pair("act", "show")
        .append_pair("video", video_id);
    if let Some(list_id) = list_id {
        body.append_pair("list", list_id);
    }
    body.append_pair("al", "1");
    crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading JSON metadata"));
    let mut response = super::direct::post(
        client,
        &endpoint,
        &[
            ("Content-Type", "application/x-www-form-urlencoded"),
            ("Referer", endpoint.as_str()),
            ("X-Requested-With", "XMLHttpRequest"),
        ],
        body.finish(),
    )
    .map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor: NAME,
            id: video_id.to_owned(),
            what: "JSON metadata",
            source: Box::new(error),
        },
        error => error,
    })?;
    check_challenge(&response.url().clone(), video_id)?;
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    let json: Value = serde_json::from_slice(&bytes).map_err(|error| {
        fail(
            Some(video_id),
            &format!("Failed to parse JSON  (caused by {error})"),
        )
    })?;
    let bug = |key: &str| {
        fail(
            None,
            &format!(
                "An extractor error has occurred. (caused by {key}); {}",
                crate::BUG_REPORTS_MESSAGE
            ),
        )
    };
    let Some(pair) = json.get("payload") else {
        return Err(bug("KeyError('payload')"));
    };
    let Some([code, payload]) = pair.as_array().map(Vec::as_slice) else {
        return Err(bug("ValueError('not enough values to unpack')"));
    };
    match code.as_str() {
        Some("3") => {
            return Err(fail(
                None,
                &format!("This video is only available for registered users. {LOGIN_HINT}"),
            ));
        }
        Some("8") => {
            let text = payload.get(0).and_then(Value::as_str).unwrap_or_default();
            let mut chars = text.chars();
            chars.next();
            chars.next_back();
            return Err(fail(None, &crate::html::clean_html(chars.as_str())));
        }
        _ => {}
    }
    Ok(payload.clone())
}

fn url_follow(page: &Url, target: &Url) -> ExtractResult {
    let mut follow = super::direct::build_at(page.clone(), page, String::new());
    follow.formats.clear();
    follow.direct = None;
    follow
        .extra
        .insert(crate::FOLLOW_URL.into(), target.as_str().into());
    ExtractResult::Video(follow)
}

fn format_from(mut format: Map<String, Value>, id: &str) -> Result<crate::VideoFormat> {
    let keys: Vec<String> = format.keys().cloned().collect();
    format.retain(|_, value| !value.is_null());
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
        .map_err(|error| fail(Some(id), &error.to_string()))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    Ok(format)
}

impl Extractor for Vk {
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
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        // `get_temp_id`: the embed's `id` group, `None` for pages.
        let temp_id = found.name("id").map(|id| id.as_str().to_owned());
        let temp = temp_id.as_deref();
        let mut mv_data = json!({});
        let (video_id, info_page, player) = if let Some(video_id) = found.name("videoid") {
            let video_id = video_id.as_str().to_owned();
            let payload = download_payload(
                client,
                &video_id,
                found.name("list_id").map(|list| list.as_str()),
            )?;
            let items = payload.as_array().cloned().unwrap_or_default();
            let info_page = items
                .get(1)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let opts = items.last().cloned().unwrap_or_default();
            if let Some(data) = opts.get("mvData").filter(|data| truthy(data)) {
                mv_data = data.clone();
            }
            let player = opts
                .get("player")
                .filter(|player| truthy(player))
                .cloned()
                .unwrap_or_else(|| json!({}));
            (video_id, info_page, player)
        } else {
            let video_id = format!("{}_{}", &found["oid"], &found["id"]);
            let embed = Url::parse(&format!(
                "https://vk.com/video_ext.php?{}",
                &found["embed_query"]
            ))?;
            let (info_page, final_url) =
                super::download_webpage_handle(client, &embed, NAME, &video_id)?;
            check_challenge(&final_url, &video_id)?;
            if let Some(message) = ERROR_MESSAGES
                .iter()
                .find_map(|pattern| pattern.captures(&info_page))
                .map(|found| crate::html::clean_html(&found[1]))
                .filter(|message| !message.is_empty())
            {
                return Err(fail(temp, &message));
            }
            if SECURITY_CHECK.is_match(&info_page) {
                return Err(fail(
                    temp,
                    "You are trying to log in from an unusual location. You should confirm ownership at vk.com to log in with this IP.",
                ));
            }
            if let Some((_, message)) = ERRORS
                .iter()
                .find(|(pattern, _)| pattern.is_match(&info_page))
            {
                return Err(fail(temp, &message.replace("%s", &video_id)));
            }
            let Some(params) = PLAYER_PARAMS.captures(&info_page) else {
                return Err(fail(
                    temp,
                    &format!(
                        "Unable to extract player params; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ),
                ));
            };
            let player: Value = serde_json::from_str(&params[1]).map_err(|error| {
                fail(
                    Some(&video_id),
                    &format!("Failed to parse JSON  (caused by {error})"),
                )
            })?;
            (video_id, info_page, player)
        };

        // Embedded players: YouTube is followed, others are unported.
        if let Some(youtube) = super::youtube::embed_urls(page, &info_page)
            .into_iter()
            .next()
        {
            return Ok(Some(url_follow(page, &youtube)));
        }
        if let Some((_, name)) = OTHER_EMBEDS
            .iter()
            .find(|(pattern, _)| pattern.is_match(&info_page))
        {
            return Err(Error::UnsupportedHtml5(format!(
                "VK videos embedding a {name} player are not supported natively"
            )));
        }
        if let Some(opts) = OPTS.captures(&info_page)
            && let Some(target) = OPTS_URL.captures_iter(&opts[1]).find_map(|found| {
                // `(?!/\b)`: not a site-relative path.
                let url = found[1].to_owned();
                let mut chars = url.chars();
                let relative = chars.next() == Some('/')
                    && chars
                        .next()
                        .is_some_and(|c| c.is_alphanumeric() || c == '_');
                (!relative).then_some(url)
            })
        {
            let target = match target.strip_prefix("//") {
                Some(rest) => format!("https://{rest}"),
                None => target,
            };
            return Ok(Some(url_follow(page, &Url::parse(&target)?)));
        }

        let Some(data) = player
            .get("params")
            .and_then(|params| params.get(0))
            .cloned()
        else {
            return Err(fail(
                temp,
                &format!(
                    "An extractor error has occurred. (caused by KeyError('params')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ),
            ));
        };
        let is_live = int(data.get("live")) == Some(2);
        let timestamp = DATE
            .captures(&info_page)
            .map(|found| crate::html::clean_html(&found[1]))
            .and_then(|date| crate::dates::unified_timestamp(&date, true))
            .or_else(|| int(data.get("date")));
        let view_count = VIEWS
            .captures(&info_page)
            .and_then(|found| found[1].replace([',', '.', '+'], "").parse::<i64>().ok());

        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        for (format_id, value) in data.as_object().into_iter().flatten() {
            let Some(format_url) = value.as_str().and_then(super::json_ld::url_or_none) else {
                continue;
            };
            if !["http", "//", "rtmp"]
                .iter()
                .any(|prefix| format_url.starts_with(prefix))
            {
                continue;
            }
            let absolute = super::proto_relative(&format_url);
            if format_id.starts_with("url")
                || format_id.starts_with("cache")
                || ["extra_data", "live_mp4", "postlive_mp4"].contains(&format_id.as_str())
            {
                let mut format = Map::new();
                format.insert("format_id".into(), format_id.clone().into());
                format.insert("url".into(), absolute.into());
                format.insert("ext".into(), "mp4".into());
                format.insert("source_preference".into(), 1.into());
                format.insert(
                    "height".into(),
                    HEIGHT
                        .captures(format_id)
                        .and_then(|found| found[1].parse::<i64>().ok())
                        .into(),
                );
                formats.push(format_from(format, &video_id)?);
            } else if format_id.starts_with("hls") && format_id != "hls_live_playback" {
                if let Some(stream) = Url::parse(&absolute).ok().and_then(|manifest| {
                    super::extract_m3u8_optional_note(
                        client,
                        &manifest,
                        NAME,
                        &video_id,
                        Some(format_id),
                        true,
                        is_live,
                        "Downloading m3u8 information",
                    )
                }) {
                    formats.extend(stream.formats);
                    super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
                }
            } else if format_id.starts_with("dash")
                && !["dash_live_playback", "dash_uni"].contains(&format_id.as_str())
            {
                if let Some(stream) = Url::parse(&absolute).ok().and_then(|manifest| {
                    super::extract_mpd_optional_note(
                        client,
                        &manifest,
                        NAME,
                        &video_id,
                        Some(format_id),
                        "Downloading MPD manifest",
                    )
                }) {
                    formats.extend(stream.formats);
                    super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
                }
            } else if format_id == "rtmp" {
                let mut format = Map::new();
                format.insert("format_id".into(), format_id.clone().into());
                format.insert("url".into(), absolute.into());
                format.insert("ext".into(), "flv".into());
                formats.push(format_from(format, &video_id)?);
            }
        }
        for sub in data
            .get("subs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(url) = sub
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .and_then(|url| Url::parse(&super::proto_relative(&url)).ok())
            else {
                continue;
            };
            let ext = sub
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(".srt")
                .rsplit('.')
                .next()
                .unwrap_or_default()
                .to_owned();
            let language = sub
                .get("lang")
                .and_then(Value::as_str)
                .unwrap_or("en")
                .to_owned();
            let mut track = Map::new();
            track.insert("ext".into(), ext.into());
            track.insert("url".into(), url.as_str().into());
            let track: crate::SubtitleFormat = serde_json::from_value(Value::Object(track))
                .map_err(|error| fail(Some(&video_id), &error.to_string()))?;
            let mut single = crate::Subtitles::default();
            single.entries.push((Some(language), vec![track]));
            super::html5::merge_subtitles(&mut subtitles, single);
        }

        let mut dict = Map::new();
        dict.insert("id".into(), video_id.clone().into());
        dict.insert("formats".into(), json!([]));
        dict.insert("subtitles".into(), json!({}));
        let mut put = |key: &str, value: Option<Value>| {
            if let Some(value) = value {
                dict.insert(key.into(), value);
            }
        };
        put("title", unescaped(mv_data.get("title")).map(Value::from));
        put("description", cleaned(mv_data.get("desc")).map(Value::from));
        put("duration", int(mv_data.get("duration")).map(Value::from));
        put("like_count", int(mv_data.get("likes")).map(Value::from));
        put(
            "comment_count",
            int(mv_data.get("commcount")).map(Value::from),
        );
        put("title", unescaped(data.get("md_title")).map(Value::from));
        put(
            "description",
            cleaned(data.get("description")).map(Value::from),
        );
        put(
            "thumbnail",
            data.get("jpg")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .map(Value::from),
        );
        put(
            "uploader",
            unescaped(data.get("md_author")).map(Value::from),
        );
        put(
            "uploader_id",
            ["author_id", "authorId"]
                .iter()
                .find_map(|key| match data.get(*key)? {
                    Value::String(text) => Some(Value::from(text.clone())),
                    Value::Number(number) => Some(Value::from(number.to_string())),
                    _ => None,
                }),
        );
        put("duration", int(data.get("duration")).map(Value::from));
        let chapters: Vec<Value> = data
            .get("time_codes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|code| code.get("time").is_some_and(Value::is_i64))
            .map(|code| {
                let mut chapter = Map::new();
                if let Some(title) = unescaped(code.get("text")) {
                    chapter.insert("title".into(), title.into());
                }
                chapter.insert("start_time".into(), code["time"].clone());
                Value::Object(chapter)
            })
            .collect();
        if !chapters.is_empty() {
            dict.insert("chapters".into(), Value::Array(chapters));
        }
        dict.insert("timestamp".into(), timestamp.into());
        dict.insert("view_count".into(), view_count.into());
        dict.insert("is_live".into(), is_live.into());

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
        info.extractor_key = KEY.into();
        info.format_sort_fields = vec!["res".into(), "source".into()];
        let titled = dict.contains_key("title");
        if !titled {
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
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
        keys.push("_format_sort_fields".into());
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info,
            &format!(
                "dict{};{};subtitle=ext,url;chapter=title,start_time",
                if titled { "" } else { "-untitled" },
                keys.join(",")
            ),
        ))))
    }
}

fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::String(text) => !text.is_empty(),
        Value::Array(items) => !items.is_empty(),
        Value::Object(items) => !items.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        let video = VALID_URL
            .captures("https://vk.com/videos-77521?z=video-77521_162222515%2Fclub77521")
            .unwrap();
        assert_eq!(&video["videoid"], "-77521_162222515");
        let embed = VALID_URL
            .captures("https://vk.com/video_ext.php?oid=-77521&id=162222515&hash=87b046504ccd8bfa")
            .unwrap();
        assert_eq!((&embed["oid"], &embed["id"]), ("-77521", "162222515"));
        let listed = VALID_URL
            .captures("https://vk.com/video-93049196_456239755?list=ln-cBjJ7S4jYYx3ADnmDT")
            .unwrap();
        assert_eq!(&listed["list_id"], "ln-cBjJ7S4jYYx3ADnmDT");
        assert!(suitable("https://vkvideo.ru/clip-220265624_456239031"));
        assert!(!suitable("https://vk.com/videos-77521"));
    }
}

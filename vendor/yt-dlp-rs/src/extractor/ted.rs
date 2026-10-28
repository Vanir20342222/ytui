//! Upstream `TedTalkIE`, `TedPlaylistIE`, `TedSeriesIE` and `TedEmbedIE`
//! (`yt_dlp/extractor/ted.py`): a talk page's Next.js `videoData` (HLS,
//! H.264, RTMP and audio formats, an external video as a `url` result, and
//! metadata with OpenGraph fallbacks), playlists and series seasons as
//! playlists of talk URLs, and embeds rewritten to talk pages.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct TedTalk;
pub struct TedPlaylist;
pub struct TedSeries;
pub struct TedEmbed;

const TALK: &str = "TedTalk";
const PLAYLIST: &str = "TedPlaylist";
const SERIES: &str = "TedSeries";
const EMBED: &str = "TedEmbed";

fn base(kind: &str) -> String {
    format!(r"^https?://www\.ted\.com/(?:{kind})(?:/lang/[^/#?]+)?/(?P<id>[\w-]+)")
}

static TALK_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(&base("talks")).unwrap());
static PLAYLIST_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&base(r"playlists(?:/\d+)?")).unwrap());
static SERIES_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"{}(?:#season_(?P<season>\d+))?", base("series"))).unwrap()
});
static EMBED_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://embed(?:-ssl)?\.ted\.com/").unwrap());
static EMBED_HOST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"://embed(-ssl)?").unwrap());

/// `TedTalkIE.suitable`.
pub fn talk_suitable(url: &str) -> bool {
    TALK_URL.is_match(url)
}

/// `TedPlaylistIE.suitable`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// `TedSeriesIE.suitable`.
pub fn series_suitable(url: &str) -> bool {
    SERIES_URL.is_match(url)
}

/// `TedEmbedIE.suitable`.
pub fn embed_suitable(url: &str) -> bool {
    EMBED_URL.is_match(url)
}

fn bug(extractor: &str, id: &str, cause: &str) -> Error {
    Error::Reported(format!(
        "[{extractor}] {id}: An extractor error has occurred. (caused by {cause}); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn truthy_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

fn start(extractor: &'static str, page: &Url) {
    crate::screen::set_extractor(extractor);
    crate::screen::screen(&format!(
        "[{extractor}] Extracting URL: {}",
        crate::screen::extracting_url(page.as_str())
    ));
}

fn follow(page: &Url, target: &Url) -> ExtractResult {
    let mut follow = super::direct::build_at(page.clone(), page, String::new());
    follow.formats.clear();
    follow.direct = None;
    follow
        .extra
        .insert(crate::FOLLOW_URL.into(), target.as_str().into());
    ExtractResult::Video(follow)
}

/// `url_result(canonicalUrl, TedTalkIE)`.
fn talk_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": TALK,
            "extractor": "tedtalk",
            "entry": {"ie_key": TALK, "_type": "url", "url": target},
        }),
    );
    Ok(info)
}

/// `_parse_playlist(playlist)`.
fn parse_playlist(playlist: &Value, extractor: &str, id: &str) -> Result<Vec<VideoInfo>> {
    let Some(nodes) = playlist.pointer("/videos/nodes").and_then(Value::as_array) else {
        return Err(bug(
            extractor,
            id,
            "TypeError(\"'NoneType' object is not iterable\")",
        ));
    };
    nodes
        .iter()
        .filter(|entry| entry.get("__typename").and_then(Value::as_str) == Some("Video"))
        .filter_map(|entry| truthy_str(entry.get("canonicalUrl")))
        .map(talk_entry)
        .collect()
}

fn playlist_result(
    page: &Url,
    extractor: &'static str,
    id: Option<String>,
    title: Option<String>,
    description: Option<String>,
    entries: Vec<VideoInfo>,
    more: Map<String, Value>,
) -> ExtractResult {
    let mut extra = Map::new();
    // `playlist_result(..., **kwargs)`: the keyword fields come first.
    let mut keys: Vec<&str> = ["series", "season_number"]
        .into_iter()
        .filter(|key| more.contains_key(*key))
        .collect();
    keys.extend(["id", "title", "description"]);
    extra.insert(
        "description".into(),
        description.map_or(Value::Null, Value::from),
    );
    for (key, value) in more {
        extra.insert(key, value);
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    if id.is_none() {
        extra.insert("id".into(), Value::Null);
    }
    if title.is_none() {
        extra.insert("title".into(), Value::Null);
    }
    ExtractResult::Playlist(PlaylistInfo {
        id: id.unwrap_or_default(),
        title: title.unwrap_or_default(),
        webpage_url: page.clone(),
        extractor: extractor.into(),
        extractor_key: extractor.into(),
        entries,
        extra,
    })
}

impl Extractor for TedTalk {
    fn key(&self) -> &'static str {
        TALK
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = TALK_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        start(TALK, page);
        let webpage = super::download_webpage(client, page, TALK, &display_id)?;
        let next = super::search_nextjs_data(&webpage, TALK, &display_id)?;
        let talk = ["props", "pageProps", "videoData"]
            .iter()
            .try_fold(&next, |value, key| {
                value
                    .get(*key)
                    .ok_or_else(|| bug(TALK, &display_id, &format!("KeyError('{key}')")))
            })?
            .clone();
        let video_id = match talk.get("id") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Number(number)) => number.to_string(),
            _ => return Err(bug(TALK, &display_id, "KeyError('id')")),
        };
        let player = talk
            .get("videoPlayerData")
            .filter(|player| player.as_object().is_some_and(|map| !map.is_empty()))
            .cloned()
            .unwrap_or_else(|| json!({}));

        let mut formats: Vec<crate::VideoFormat> = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        let push = |formats: &mut Vec<crate::VideoFormat>, map: Map<String, Value>| -> Result<()> {
            let keys: Vec<String> = map.keys().cloned().collect();
            let mut map = map;
            map.retain(|_, value| !value.is_null());
            if !map.contains_key("ext") {
                let ext = map
                    .get("url")
                    .and_then(Value::as_str)
                    .and_then(super::direct::determine_ext)
                    .unwrap_or_else(|| "unknown_video".into());
                map.insert("ext".into(), ext.into());
            }
            let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
                .map_err(|error| bug(TALK, &display_id, &error.to_string()))?;
            crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
            Ok(())
        };
        for (format_id, resources) in player
            .get("resources")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            if format_id == "hls" {
                let Some(stream) = resources
                    .get("stream")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                    .and_then(|url| Url::parse(&url).ok())
                else {
                    continue;
                };
                if let Some(info) = super::extract_m3u8_optional(
                    client,
                    &stream,
                    TALK,
                    &video_id,
                    Some("hls"),
                    true,
                ) {
                    formats.extend(info.formats);
                    super::html5::merge_subtitles(&mut subtitles, info.subtitles);
                }
                continue;
            }
            let Some(resources) = resources.as_array() else {
                continue;
            };
            if format_id == "h264" {
                for resource in resources {
                    let Some(url) = truthy_str(resource.get("file")) else {
                        continue;
                    };
                    let bitrate = int(resource.get("bitrate"));
                    let mut map = Map::new();
                    map.insert("url".into(), url.into());
                    map.insert(
                        "format_id".into(),
                        format!(
                            "{format_id}-{}k",
                            bitrate.map_or("None".into(), |rate| rate.to_string())
                        )
                        .into(),
                    );
                    map.insert("tbr".into(), bitrate.into());
                    push(&mut formats, map)?;
                }
            } else if format_id == "rtmp" {
                let Some(streamer) = truthy_str(talk.get("streamer")) else {
                    continue;
                };
                for resource in resources {
                    let Some(file) = truthy_str(resource.get("file")) else {
                        continue;
                    };
                    let mut map = Map::new();
                    map.insert(
                        "format_id".into(),
                        format!(
                            "{format_id}-{}",
                            resource
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("None")
                        )
                        .into(),
                    );
                    map.insert("url".into(), streamer.into());
                    map.insert("play_path".into(), file.into());
                    map.insert("ext".into(), "flv".into());
                    map.insert("width".into(), int(resource.get("width")).into());
                    map.insert("height".into(), int(resource.get("height")).into());
                    map.insert("tbr".into(), int(resource.get("bitrate")).into());
                    push(&mut formats, map)?;
                }
            }
        }
        // Upstream's `http_url` bitrate formats only consider `protocol ==
        // 'm3u8'` formats; its HLS formats are `m3u8_native`, so none apply.
        if let Some(audio) = truthy_str(talk.get("audioDownload")) {
            let mut map = Map::new();
            map.insert("url".into(), audio.into());
            map.insert("format_id".into(), "audio".into());
            map.insert("vcodec".into(), "none".into());
            push(&mut formats, map)?;
        }
        if formats.is_empty() {
            let external = player
                .get("external")
                .filter(|external| external.as_object().is_some_and(|map| !map.is_empty()))
                .cloned()
                .unwrap_or_else(|| json!({}));
            let service = external
                .get("service")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let code = if service.eq_ignore_ascii_case("youtube") {
                truthy_str(external.get("code"))
            } else {
                None
            };
            let target = match code {
                Some(code) => code.to_owned(),
                None => match external.get("uri") {
                    Some(uri) => uri.as_str().unwrap_or_default().to_owned(),
                    None => return Err(bug(TALK, &display_id, "KeyError('uri')")),
                },
            };
            let Ok(target) = Url::parse(&target) else {
                return Err(Error::UnsupportedHtml5(format!(
                    "TED external video {target:?} is not a URL"
                )));
            };
            return Ok(Some(follow(page, &target)));
        }
        let thumbnail = truthy_str(player.get("thumb"))
            .map(str::to_owned)
            .or_else(|| crate::html::og_search_property(&webpage, "image"))
            .filter(|thumb| !thumb.is_empty())
            .map(|thumb| thumb.split('?').next().unwrap_or_default().to_owned());
        let title = truthy_str(talk.get("title"))
            .map(str::to_owned)
            .or_else(|| crate::html::og_search_property(&webpage, "title"));
        let description = truthy_str(talk.get("description"))
            .map(str::to_owned)
            .or_else(|| crate::html::og_search_property(&webpage, "description"));
        let duration = talk
            .get("duration")
            .filter(|duration| match duration {
                Value::Number(number) => number.as_f64() != Some(0.0),
                Value::String(text) => !text.is_empty(),
                _ => false,
            })
            .cloned()
            .or_else(|| {
                crate::html::og_search_property(&webpage, "video:duration")
                    .and_then(|text| crate::parse_duration(&text))
                    .and_then(|duration| serde_json::to_value(duration).ok())
            });
        let mut dict = Map::new();
        dict.insert("id".into(), video_id.clone().into());
        dict.insert(
            "title".into(),
            title.clone().map_or(Value::Null, Value::from),
        );
        dict.insert(
            "uploader".into(),
            talk.get("presenterDisplayName")
                .cloned()
                .unwrap_or(Value::Null),
        );
        dict.insert(
            "thumbnail".into(),
            thumbnail.map_or(Value::Null, Value::from),
        );
        dict.insert(
            "description".into(),
            description.map_or(Value::Null, Value::from),
        );
        dict.insert("subtitles".into(), json!({}));
        dict.insert("formats".into(), json!([]));
        dict.insert("duration".into(), duration.unwrap_or(Value::Null));
        dict.insert(
            "view_count".into(),
            match talk.get("viewedCount") {
                Some(Value::Number(number)) => number.as_i64().into(),
                Some(Value::String(text)) => text
                    .replace([',', '.', '+'], "")
                    .trim()
                    .parse::<i64>()
                    .ok()
                    .into(),
                _ => Value::Null,
            },
        );
        let date = |key: &str| {
            talk.get(key)
                .and_then(Value::as_str)
                .and_then(|text| crate::dates::unified_strdate(text, true))
                .map_or(Value::Null, Value::from)
        };
        dict.insert("upload_date".into(), date("publishedAt"));
        dict.insert("release_date".into(), date("recordedOn"));
        dict.insert(
            "tags".into(),
            player
                .pointer("/targeting/tag")
                .and_then(Value::as_str)
                .map_or(Value::Null, |tags| {
                    Value::Array(tags.split(',').map(Value::from).collect())
                }),
        );

        let first = formats
            .first()
            .map_or_else(|| page.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.direct = None;
        info.formats = formats;
        info.subtitles = subtitles;
        info.webpage_url = page.clone();
        info.extractor = TALK.into();
        info.extractor_key = TALK.into();
        if title.is_none() {
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
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info,
            &format!("dict;{};subtitle=ext,url", keys.join(",")),
        ))))
    }
}

impl Extractor for TedPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        start(PLAYLIST, page);
        let webpage = super::download_webpage(client, page, PLAYLIST, &display_id)?;
        let next = super::search_nextjs_data(&webpage, PLAYLIST, &display_id)?;
        let playlist = ["props", "pageProps", "playlist"]
            .iter()
            .try_fold(&next, |value, key| {
                value
                    .get(*key)
                    .ok_or_else(|| bug(PLAYLIST, &display_id, &format!("KeyError('{key}')")))
            })?
            .clone();
        let entries = parse_playlist(&playlist, PLAYLIST, &display_id)?;
        let id = match playlist.get("id") {
            Some(Value::String(text)) => Some(text.clone()),
            Some(Value::Number(number)) => Some(number.to_string()),
            _ => None,
        };
        let title = truthy_str(playlist.get("title"))
            .map(str::to_owned)
            .or_else(|| {
                crate::html::og_search_property(&webpage, "title")
                    .map(|title| title.replace(" | TED Talks", ""))
                    .filter(|title| !title.is_empty())
            });
        let description = crate::html::og_search_property(&webpage, "description");
        Ok(Some(playlist_result(
            page,
            PLAYLIST,
            id,
            title,
            description,
            entries,
            Map::new(),
        )))
    }
}

impl Extractor for TedSeries {
    fn key(&self) -> &'static str {
        SERIES
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SERIES_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        let season = found
            .name("season")
            .map(|season| season.as_str().to_owned());
        start(SERIES, page);
        let webpage = super::download_webpage_note(
            client,
            page,
            SERIES,
            &display_id,
            "Downloading series webpage",
        )?;
        let next = super::search_nextjs_data(&webpage, SERIES, &display_id)?;
        let info = ["props", "pageProps"]
            .iter()
            .try_fold(&next, |value, key| {
                value
                    .get(*key)
                    .ok_or_else(|| bug(SERIES, &display_id, &format!("KeyError('{key}')")))
            })?
            .clone();
        let Some(seasons) = info.get("seasons").and_then(Value::as_array) else {
            return Err(bug(SERIES, &display_id, "KeyError('seasons')"));
        };
        let mut entries = Vec::new();
        for item in seasons {
            // `season in [None, s.get('seasonNumber')]` (the URL's string).
            if season.is_none()
                || item.get("seasonNumber").and_then(Value::as_str) == season.as_deref()
            {
                entries.extend(parse_playlist(item, SERIES, &display_id)?);
            }
        }
        let series_id = match info.pointer("/series/id") {
            Some(Value::String(text)) => Some(text.clone()),
            Some(Value::Number(number)) => Some(number.to_string()),
            _ => None,
        };
        let series_name = truthy_str(info.pointer("/series/name"))
            .map(str::to_owned)
            .or_else(|| crate::html::og_search_property(&webpage, "title"));
        let id = match (&season, &series_id) {
            (Some(season), Some(series_id)) => Some(format!("{series_id}_{season}")),
            _ => series_id,
        };
        let title = match &season {
            Some(season) => Some(format!(
                "{} Season {season}",
                series_name.as_deref().unwrap_or("None")
            )),
            None => series_name.clone(),
        };
        let description = crate::html::og_search_property(&webpage, "description");
        let mut more = Map::new();
        more.insert(
            "series".into(),
            series_name.map_or(Value::Null, Value::from),
        );
        more.insert(
            "season_number".into(),
            season
                .as_deref()
                .and_then(|season| season.parse::<i64>().ok())
                .into(),
        );
        Ok(Some(playlist_result(
            page,
            SERIES,
            id,
            title,
            description,
            entries,
            more,
        )))
    }
}

impl Extractor for TedEmbed {
    fn key(&self) -> &'static str {
        EMBED
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        if !EMBED_URL.is_match(page.as_str()) {
            return Ok(None);
        }
        start(EMBED, page);
        let target = Url::parse(&EMBED_HOST.replace(page.as_str(), "://www"))?;
        Ok(Some(follow(page, &target)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_urls() {
        assert!(talk_suitable(
            "https://www.ted.com/talks/candace_parker_how_to_break_down_barriers"
        ));
        assert!(talk_suitable("https://www.ted.com/talks/lang/en/some_talk"));
        assert!(playlist_suitable(
            "https://www.ted.com/playlists/171/the_most_popular_talks_of_all"
        ));
        let series = SERIES_URL
            .captures("https://www.ted.com/series/small_thing_big_idea#season_2")
            .unwrap();
        assert_eq!(&series["season"], "2");
        assert!(embed_suitable("https://embed-ssl.ted.com/talks/x"));
        assert!(!talk_suitable("https://www.ted.com/playlists/171/x"));
    }
}

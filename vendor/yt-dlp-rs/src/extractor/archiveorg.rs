//! Upstream `ArchiveOrgIE` (`yt_dlp/extractor/archiveorg.py`): the embed
//! player's `<play-av>` playlist (entries and subtitle labels), the
//! metadata API's item and file fields, formats of the known media files
//! (private files skipped without a `logged-in-sig` cookie), thumbnails and
//! reviews as comments; one entry becomes the video, several a playlist.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct ArchiveOrg;

const NAME: &str = "archive.org";
const KEY: &str = "ArchiveOrg";
/// `KNOWN_EXTENSIONS`.
const KNOWN_EXTENSIONS: [&str; 42] = [
    "3g2", "3gp", "f4v", "mk3d", "divx", "mpg", "ogv", "m4v", "wmv", "avi", "flv", "mkv", "mov",
    "mp4", "webm", "aac", "ape", "asf", "f4a", "f4b", "m4b", "m4r", "oga", "ogx", "spx", "vorbis",
    "wma", "weba", "aiff", "alac", "flac", "m4a", "mka", "mp3", "ogg", "opus", "wav", "f4f", "f4m",
    "m3u8", "smil", "mpd",
];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?archive\.org/(?:details|embed)/(?P<id>[^?#]+)(?:[?].*)?$")
        .unwrap()
});

/// `ArchiveOrgIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

fn bug(id: &str, cause: &str) -> Error {
    fail(
        id,
        &format!(
            "An extractor error has occurred. (caused by {cause}); {}",
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

/// `urllib.parse.unquote_plus`.
fn unquote_plus(text: &str) -> String {
    percent_encoding::percent_decode_str(&text.replace('+', " "))
        .decode_utf8_lossy()
        .into_owned()
}

/// `urllib.parse.quote(name)` (`/` kept).
fn quote(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-' | b'~' | b'/' => {
                out.push(char::from(byte));
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// `('description', ({str}, (..., all, {' '.join})), {clean_html}, filter, any)`.
fn description(value: Option<&Value>) -> Option<Value> {
    let text = match value? {
        Value::String(text) => text.clone(),
        Value::Array(items) => {
            let parts: Option<Vec<&str>> = items.iter().map(Value::as_str).collect();
            parts?.join(" ")
        }
        _ => return None,
    };
    let text = crate::html::clean_html(&text);
    (!text.is_empty()).then(|| Value::from(text))
}

/// `('creator', (None, ...), {str}, filter, all, filter)`.
fn creators(value: Option<&Value>) -> Option<Value> {
    let items: Vec<Value> = match value? {
        Value::String(text) => vec![Value::from(text.clone())],
        Value::Array(items) => items.clone(),
        _ => Vec::new(),
    };
    let items: Vec<Value> = items
        .into_iter()
        .filter(|item| item.as_str().is_some_and(|text| !text.is_empty()))
        .collect();
    (!items.is_empty()).then_some(Value::Array(items))
}

/// `merge_dicts(*dicts)`.
fn merge_dicts(dicts: &[&Map<String, Value>]) -> Map<String, Value> {
    let mut merged = Map::new();
    for dict in dicts {
        for (key, value) in dict.iter() {
            let replace = match merged.get(key) {
                None => !value.is_null(),
                Some(Value::String(existing)) => existing.is_empty() && value.is_string(),
                Some(_) => false,
            };
            if replace {
                merged.insert(key.clone(), value.clone());
            }
        }
    }
    merged
}

fn put(map: &mut Map<String, Value>, key: &str, value: Option<Value>) {
    if let Some(value) = value.filter(|value| !value.is_null()) {
        map.insert(key.into(), value);
    }
}

/// An info dict (`formats`, `thumbnails`, `subtitles` as built) as a video.
fn video(page: &Url, dict: Map<String, Value>, id: &str) -> Result<VideoInfo> {
    let mut formats = Vec::new();
    for format in dict
        .get("formats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let mut map = format.as_object().cloned().unwrap_or_default();
        let keys: Vec<String> = map.keys().cloned().collect();
        map.retain(|_, value| !value.is_null());
        let ext = map
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::direct::determine_ext)
            .unwrap_or_else(|| "unknown_video".into());
        map.insert("ext".into(), ext.into());
        map.insert("format_id".into(), "".into());
        let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(map))
            .map_err(|error| fail(id, &error.to_string()))?;
        crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
        formats.push(format);
    }
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
    info.webpage_url = page.clone();
    info.extractor = NAME.into();
    info.extractor_key = KEY.into();
    info.format_sort_fields = vec!["source".into()];
    let titled = dict.get("title").is_some_and(Value::is_string);
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
            "formats" | "subtitles" | "_format_sort_fields" => {}
            "webpage_url" => {
                if let Some(url) = value.as_str().and_then(|url| Url::parse(url).ok()) {
                    info.webpage_url = url;
                }
            }
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    Ok(crate::info_order::mark_info(
        info,
        &format!(
            "dict{};{};thumbnail=id,url,width,height,filesize",
            if titled { "" } else { "-untitled" },
            keys.join(",")
        ),
    ))
}

impl Extractor for ArchiveOrg {
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
        let temp_id = found["id"].to_owned();
        let video_id = unquote_plus(&temp_id);
        let (identifier, entry_id) = match video_id.split_once('/') {
            Some((identifier, entry)) => (identifier.to_owned(), entry.to_owned()),
            None => (video_id.clone(), String::new()),
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        let embed = Url::parse(&format!("https://archive.org/embed/{identifier}"))?;
        let embed_page = super::download_webpage(client, &embed, NAME, &identifier)?;
        // `_playlist_data`: the `<play-av>` element's `playlist` JSON.
        let element = crate::html_elements::element_text_and_html_by_tag("play-av", &embed_page)
            .map_err(Error::Reported)?
            .1;
        let attrs = crate::html_elements::extract_attributes(&element);
        let Some(playlist) = attrs.get("playlist").and_then(Value::as_str) else {
            return Err(bug(&temp_id, "KeyError('playlist')"));
        };
        let playlist: Value =
            serde_json::from_str(playlist).map_err(|error| Error::Reported(format!("{error}")))?;
        let mut entries: Vec<(String, Map<String, Value>)> = Vec::new();
        for item in playlist.as_array().into_iter().flatten() {
            let Some(orig) = item.get("orig") else {
                return Err(bug(&temp_id, "KeyError('orig')"));
            };
            let orig = orig.as_str().unwrap_or_default().to_owned();
            if !entry_id.is_empty() && orig != entry_id {
                continue;
            }
            let mut entry = Map::new();
            entry.insert("formats".into(), json!([]));
            entry.insert("thumbnails".into(), json!([]));
            entry.insert(
                "artist".into(),
                item.get("artist").cloned().unwrap_or(Value::Null),
            );
            entry.insert(
                "track".into(),
                item.get("title").cloned().unwrap_or(Value::Null),
            );
            entry.insert("subtitles".into(), json!({}));
            for track in item
                .get("tracks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(kind) = track.get("kind") else {
                    return Err(bug(&temp_id, "KeyError('kind')"));
                };
                if kind.as_str() != Some("subtitles") {
                    continue;
                }
                // Upstream stores each track under its label, not `subtitles`.
                let (Some(label), Some(file)) = (
                    track.get("label").and_then(Value::as_str),
                    track.get("file").and_then(Value::as_str),
                ) else {
                    return Err(bug(&temp_id, "KeyError('label')"));
                };
                entry.insert(
                    label.into(),
                    json!({"url": format!("https://archive.org/{}", file.trim_start_matches('/'))}),
                );
            }
            match entries.iter_mut().find(|(name, _)| *name == orig) {
                Some((_, existing)) => *existing = entry,
                None => entries.push((orig, entry)),
            }
        }

        let api = Url::parse(&format!("https://archive.org/metadata/{identifier}"))?;
        let metadata = super::download_json(client, &api, NAME, &identifier)?;
        let Some(m) = metadata.get("metadata") else {
            return Err(bug(&temp_id, "KeyError('metadata')"));
        };
        let Some(identifier) = m.get("identifier").and_then(Value::as_str) else {
            return Err(bug(&temp_id, "KeyError('identifier')"));
        };
        let identifier = identifier.to_owned();
        let mut info = Map::new();
        put(
            &mut info,
            "title",
            m.get("title").filter(|t| t.is_string()).cloned(),
        );
        put(&mut info, "description", description(m.get("description")));
        put(
            &mut info,
            "uploader",
            ["uploader", "adder"]
                .iter()
                .find_map(|key| m.get(*key).filter(|value| value.is_string()).cloned()),
        );
        put(&mut info, "creators", creators(m.get("creator")));
        put(
            &mut info,
            "license",
            m.get("licenseurl")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .map(Value::from),
        );
        put(
            &mut info,
            "release_date",
            m.get("date")
                .and_then(Value::as_str)
                .and_then(|date| crate::dates::unified_strdate(date, true))
                .map(Value::from),
        );
        put(
            &mut info,
            "timestamp",
            ["publicdate", "addeddate"]
                .iter()
                .find_map(|key| {
                    m.get(*key)
                        .and_then(Value::as_str)
                        .and_then(|date| crate::dates::unified_timestamp(date, true))
                })
                .map(Value::from),
        );
        put(
            &mut info,
            "location",
            m.get("venue").filter(|v| v.is_string()).cloned(),
        );
        put(
            &mut info,
            "release_year",
            int(m.get("year")).map(Value::from),
        );
        info.insert("id".into(), identifier.clone().into());
        info.insert(
            "webpage_url".into(),
            format!("https://archive.org/details/{identifier}").into(),
        );

        let logged_in = crate::cookies::get_cookie("https://archive.org", "logged-in-sig")
            .is_some_and(|value| !value.is_empty());
        let Some(files) = metadata.get("files") else {
            return Err(bug(&temp_id, "KeyError('files')"));
        };
        for file in files.as_array().into_iter().flatten() {
            let Some(name) = file.get("name").and_then(Value::as_str) else {
                return Err(bug(&temp_id, "KeyError('name')"));
            };
            let index = if let Some(index) = entries.iter().position(|(entry, _)| entry == name) {
                let mut fields = Map::new();
                fields.insert("id".into(), format!("{identifier}/{name}").into());
                put(
                    &mut fields,
                    "title",
                    ["title", "name"]
                        .iter()
                        .find_map(|key| file.get(*key).filter(|value| value.is_string()).cloned()),
                );
                fields.insert("display_id".into(), name.into());
                put(
                    &mut fields,
                    "description",
                    description(file.get("description")),
                );
                put(&mut fields, "creators", creators(file.get("creator")));
                put(
                    &mut fields,
                    "duration",
                    file.get("length")
                        .and_then(Value::as_str)
                        .and_then(crate::parse_duration)
                        .and_then(|duration| serde_json::to_value(duration).ok()),
                );
                put(
                    &mut fields,
                    "track_number",
                    int(file.get("track")).map(Value::from),
                );
                put(
                    &mut fields,
                    "album",
                    file.get("album").filter(|v| v.is_string()).cloned(),
                );
                put(
                    &mut fields,
                    "discnumber",
                    int(file.get("disc")).map(Value::from),
                );
                put(
                    &mut fields,
                    "release_year",
                    int(file.get("year")).map(Value::from),
                );
                let merged = merge_dicts(&[&entries[index].1, &fields]);
                entries[index].1 = merged;
                index
            } else if let Some(index) = file
                .get("original")
                .and_then(Value::as_str)
                .and_then(|original| entries.iter().position(|(entry, _)| entry == original))
            {
                index
            } else {
                continue;
            };
            let entry = &mut entries[index].1;
            if file.get("format").and_then(Value::as_str) == Some("Thumbnail") {
                let thumbnail = json!({
                    "id": name,
                    "url": format!("https://archive.org/download/{identifier}/{name}"),
                    "width": int(file.get("width")),
                    "height": int(file.get("width")),
                    "filesize": int(file.get("size")),
                });
                if let Some(Value::Array(thumbnails)) = entry.get_mut("thumbnails") {
                    thumbnails.push(thumbnail);
                }
            }
            let extension = name.rsplit_once('.').map(|(_, ext)| ext);
            let private = match file.get("private") {
                None | Some(Value::Null) | Some(Value::Bool(false)) => false,
                Some(Value::String(text)) => !text.is_empty(),
                Some(_) => true,
            };
            if extension.is_some_and(|ext| KNOWN_EXTENSIONS.contains(&ext))
                && (!private || logged_in)
            {
                let source = file.get("source").cloned().unwrap_or(Value::Null);
                let format = json!({
                    "url": format!("https://archive.org/download/{identifier}/{}", quote(name)),
                    "format": file.get("format").cloned().unwrap_or(Value::Null),
                    "width": int(file.get("width")),
                    "height": int(file.get("height")),
                    "filesize": int(file.get("size")),
                    "protocol": "https",
                    "source_preference": if source.as_str() == Some("original") { 0 } else { -1 },
                    "format_note": source,
                });
                if let Some(Value::Array(formats)) = entry.get_mut("formats") {
                    formats.push(format);
                }
            }
        }
        for (_, entry) in &mut entries {
            entry.insert("_format_sort_fields".into(), json!(["source"]));
        }

        let comments: Option<Value> = metadata
            .get("reviews")
            .and_then(Value::as_array)
            .filter(|reviews| !reviews.is_empty())
            .map(|reviews| {
                Value::Array(
                    reviews
                        .iter()
                        .map(|review| {
                            let text: Vec<&str> = ["reviewtitle", "reviewbody"]
                                .iter()
                                .filter_map(|key| review.get(*key).and_then(Value::as_str))
                                .filter(|text| !text.is_empty())
                                .collect();
                            json!({
                                "id": review.get("review_id").cloned().unwrap_or(Value::Null),
                                "author": review.get("reviewer").cloned().unwrap_or(Value::Null),
                                "text": text.join("\n\n"),
                                "timestamp": review
                                    .get("createdate")
                                    .and_then(Value::as_str)
                                    .and_then(|date| crate::dates::unified_timestamp(date, true)),
                                "parent": "root",
                            })
                        })
                        .collect(),
                )
            });

        if entries.len() == 1 {
            let only = entries.pop().map(|(_, entry)| entry).unwrap_or_default();
            let mut dict = if entry_id.is_empty() {
                merge_dicts(&[&info, &only])
            } else {
                merge_dicts(&[&only, &info])
            };
            if let Some(comments) = comments {
                dict.insert("comments".into(), comments);
            }
            return Ok(Some(ExtractResult::Video(video(page, dict, &temp_id)?)));
        }
        let mut videos = Vec::new();
        for (_, entry) in entries {
            videos.push(video(page, entry, &temp_id)?);
        }
        let keys: Vec<String> = info.keys().cloned().collect();
        let mut extra = info;
        let id = extra
            .shift_remove("id")
            .and_then(|id| id.as_str().map(str::to_owned))
            .unwrap_or_default();
        let title = extra
            .shift_remove("title")
            .and_then(|title| title.as_str().map(str::to_owned))
            .unwrap_or_default();
        let webpage_url = extra
            .shift_remove("webpage_url")
            .and_then(|url| url.as_str().and_then(|url| Url::parse(url).ok()))
            .unwrap_or_else(|| page.clone());
        let mut after = Vec::new();
        if let Some(comments) = comments {
            extra.insert("comments".into(), comments);
            after.push("comments");
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{};after={}", keys.join(","), after.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id,
            title,
            webpage_url,
            extractor: NAME.into(),
            extractor_key: KEY.into(),
            entries: videos,
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://archive.org/details/XD300-23_68HighlightsAResearchCntAugHumanIntellect"
        ));
        assert!(suitable(
            "https://archive.org/embed/XD300-23_68HighlightsAResearchCntAugHumanIntellect"
        ));
        assert!(suitable(
            "https://archive.org/details/Election_Ads/Commercial-JFK1960ElectionAdCampaignJingle.mpg"
        ));
        assert!(!suitable("https://archive.org/search?query=x"));
    }

    #[test]
    fn names_are_quoted_like_urllib() {
        assert_eq!(quote("a b/ü.mp4"), "a%20b/%C3%BC.mp4");
        assert_eq!(unquote_plus("a+b%2Fc"), "a b/c");
    }
}

//! Upstream `BandcampIE` (`yt_dlp/extractor/bandcamp.py`): a track page's
//! `data-tralbum`/`data-embed` JSON, its streaming files and, for free
//! downloads, the download page's formats through their stat URLs.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Bandcamp;

const NAME: &str = "Bandcamp";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?P<uploader>[^/]+)\.bandcamp\.com/track/(?P<id>[^/?#&]+)").unwrap()
});
static ALBUM_ARTIST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<h3 class="albumTitle">[\S\s]*?by\s*<span>\s*<a href="[^>]+">\s*([^>]+?)\s*</a>"#)
        .unwrap()
});

/// `BandcampIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

/// `_extract_data_attr(webpage, id, attr)`: `data-ATTR="{...}"` JSON.
fn data_attr(webpage: &str, id: &str, attr: &str, fatal: bool) -> Result<Value> {
    let pattern = Regex::new(&format!(r#"data-{attr}=(?:"(\{{.+?\}})"|'(\{{.+?\}})')"#))
        .map_err(|error| Error::Reported(error.to_string()))?;
    // The search is fatal; `fatal` only covers the JSON parse.
    let Some(found) = pattern.captures(webpage) else {
        return Err(fail(
            id,
            &format!(
                "Unable to extract {attr} data; {}",
                crate::BUG_REPORTS_MESSAGE
            ),
        ));
    };
    let raw = found.get(1).or(found.get(2)).map_or("", |raw| raw.as_str());
    let text = crate::html::unescape_html(raw);
    serde_json::from_str(&text).or_else(|error| {
        if fatal {
            Err(fail(
                id,
                &format!("Failed to parse JSON  (caused by {error})"),
            ))
        } else {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Failed to parse JSON  (caused by {error})"
            ));
            Ok(json!({}))
        }
    })
}

/// `BandcampBaseIE._download_webpage`: a page without a title warns
/// (`_html_extract_title`), and a client challenge needs impersonation.
fn fetch_page(client: &Client, url: &Url, id: &str, note: &str) -> Result<String> {
    let page = super::download_webpage_note(client, url, NAME, id, note)?;
    match crate::html::html_extract_title(&page).as_deref() {
        Some("Client Challenge") => Err(Error::UnsupportedHtml5(
            "Bandcamp's client challenge needs browser impersonation, which is unavailable".into(),
        )),
        Some(_) => Ok(page),
        None => {
            super::search_warning(NAME, "title");
            Ok(page)
        }
    }
}

fn float(value: Option<&Value>) -> Option<f64> {
    let value = value?;
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn int(value: Option<&Value>) -> Option<i64> {
    float(value).map(|number| number as i64)
}

/// `_proto_relative_url(url, 'http:')`.
fn proto_relative(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("http://{rest}"),
        None => url.to_owned(),
    }
}

/// `parse_filesize('NUMBER UNIT')` for the download page's sizes.
fn parse_filesize(text: &str) -> Option<i64> {
    static SIZE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)^\s*([\d.,]+)\s*([kmgt]?i?b)\s*$").unwrap());
    let found = SIZE.captures(text)?;
    let number: f64 = found[1].replace(',', ".").parse().ok()?;
    let unit = found[2].to_lowercase();
    let factor = match unit.as_str() {
        "b" => 1.0,
        "kb" => 1000.0,
        "kib" => 1024.0,
        "mb" => 1000f64.powi(2),
        "mib" => 1024f64.powi(2),
        "gb" => 1000f64.powi(3),
        "gib" => 1024f64.powi(3),
        "tb" => 1000f64.powi(4),
        "tib" => 1024f64.powi(4),
        _ => return None,
    };
    Some((number * factor) as i64)
}

fn format_from(mut format: Map<String, Value>, id: &str) -> Result<crate::VideoFormat> {
    let keys: Vec<String> = format.keys().cloned().collect();
    for key in ["abr", "filesize", "format_note"] {
        if format.get(key).is_some_and(Value::is_null) {
            format.shift_remove(key);
        }
    }
    if format.get("ext").is_none_or(Value::is_null) {
        let ext = format
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::direct::determine_ext)
            .unwrap_or_else(|| "unknown_video".into());
        format.insert("ext".into(), ext.into());
    }
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
        .map_err(|error| fail(id, &error.to_string()))?;
    crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
    crate::info_order::mark_int_rates(&mut format);
    Ok(format)
}

impl Extractor for Bandcamp {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let (title_id, uploader) = (found["id"].to_owned(), found["uploader"].to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = fetch_page(client, url, &title_id, "Downloading webpage")?;
        let tralbum = data_attr(&webpage, &title_id, "tralbum", true)?;
        let mut thumbnail = crate::html::og_search_property(&webpage, "image");
        let mut formats = Vec::new();
        let mut track: Option<String> = None;
        let mut track_id: Option<String> = None;
        let mut track_number = None;
        let mut duration = None;
        if let Some(info) = tralbum
            .pointer("/trackinfo/0")
            .filter(|info| info.is_object())
        {
            if let Some(files) = info.get("file").and_then(Value::as_object) {
                for (format_id, format_url) in files {
                    let Some(format_url) =
                        format_url.as_str().and_then(super::json_ld::url_or_none)
                    else {
                        continue;
                    };
                    let (ext, abr) = format_id.split_once('-').unwrap_or((format_id, ""));
                    let mut format = Map::new();
                    format.insert("format_id".into(), format_id.clone().into());
                    format.insert("url".into(), proto_relative(&format_url).into());
                    format.insert("ext".into(), ext.into());
                    format.insert("vcodec".into(), "none".into());
                    format.insert("acodec".into(), ext.into());
                    format.insert("abr".into(), abr.parse::<i64>().ok().into());
                    formats.push(format_from(format, &title_id)?);
                }
            }
            track = info.get("title").and_then(Value::as_str).map(str::to_owned);
            track_id = ["track_id", "id"]
                .iter()
                .find_map(|key| match info.get(*key) {
                    Some(Value::Number(number)) if number.as_f64() != Some(0.0) => {
                        Some(number.to_string())
                    }
                    Some(Value::String(text)) if !text.is_empty() => Some(text.clone()),
                    _ => None,
                });
            track_number = int(info.get("track_num"));
            duration = float(info.get("duration"));
        }
        let embed = data_attr(&webpage, &title_id, "embed", false)?;
        let current = tralbum.get("current").cloned().unwrap_or_else(|| json!({}));
        let text_of = |value: Option<&Value>| {
            value
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        let mut artist = text_of(embed.get("artist"))
            .or_else(|| text_of(current.get("artist")))
            .or_else(|| text_of(tralbum.get("artist")));
        let album_artist = ALBUM_ARTIST
            .captures(&webpage)
            .map(|found| found[1].to_owned());
        if album_artist.is_none() {
            super::search_warning(NAME, "album artist");
        }
        let timestamp = text_of(current.get("publish_date"))
            .or_else(|| text_of(tralbum.get("album_publish_date")))
            .and_then(|date| crate::dates::unified_timestamp(&date, false));
        if let Some(download_link) = text_of(tralbum.get("freeDownloadPage")) {
            let id = match tralbum.get("id") {
                Some(Value::Number(number)) => number.to_string(),
                Some(Value::String(text)) => text.clone(),
                _ => {
                    return Err(fail(
                        &title_id,
                        "An extractor error has occurred. (caused by KeyError('id'))",
                    ));
                }
            };
            track_id = Some(id.clone());
            let download_url = Url::parse(&download_link)?;
            let download_page = fetch_page(
                client,
                &download_url,
                &id,
                "Downloading free downloads page",
            )?;
            let blob = data_attr(&download_page, &id, "blob", true)?;
            if let Some(info) = blob
                .pointer("/digital_items/0")
                .or_else(|| blob.pointer("/download_items/0"))
                .filter(|info| info.is_object())
                && let Some(downloads) = info.get("downloads").and_then(Value::as_object)
            {
                if track.is_none() {
                    track = text_of(info.get("title"));
                }
                if artist.is_none() {
                    artist = text_of(info.get("artist"));
                }
                if thumbnail.is_none() {
                    thumbnail = text_of(info.get("thumb_url"));
                }
                let mut extensions: Vec<(String, String)> = Vec::new();
                for entry in blob
                    .get("download_formats")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let (Some(name), Some(ext)) = (
                        entry.get("name").and_then(Value::as_str),
                        entry.get("file_extension").and_then(Value::as_str),
                    ) {
                        extensions.push((name.to_owned(), ext.trim_matches('.').to_owned()));
                    }
                }
                for (key, entry) in downloads {
                    let Some(format_url) = entry
                        .get("url")
                        .and_then(Value::as_str)
                        .filter(|u| !u.is_empty())
                    else {
                        continue;
                    };
                    let rand = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |time| time.as_millis() as u64 % 1_000_000_007);
                    let Ok(stat_base) = Url::parse(&proto_relative(
                        &format_url.replace("/download/", "/statdownload/"),
                    )) else {
                        continue;
                    };
                    let stat_url =
                        super::update_url_query(&stat_base, &[(".rand", &rand.to_string())]);
                    let format_id =
                        text_of(entry.get("encoding_name")).unwrap_or_else(|| key.clone());
                    let stat = super::download_json_transformed(
                        client,
                        &stat_url,
                        NAME,
                        &id,
                        &format!("Downloading {format_id} JSON"),
                        &|text| {
                            let start = text.find('{').unwrap_or(0);
                            let end = text.rfind('}').map_or(text.len(), |end| end + 1);
                            Ok(text[start..end].to_owned())
                        },
                    );
                    let stat = match stat {
                        Ok(stat) => stat,
                        Err(error) => {
                            crate::screen::warning(&error.to_string());
                            continue;
                        }
                    };
                    let Some(retry) = stat
                        .get("retry_url")
                        .and_then(Value::as_str)
                        .and_then(super::json_ld::url_or_none)
                    else {
                        continue;
                    };
                    let mut format = Map::new();
                    format.insert("url".into(), proto_relative(&retry).into());
                    format.insert(
                        "ext".into(),
                        extensions
                            .iter()
                            .find(|(name, _)| *name == format_id)
                            .map(|(_, ext)| ext.clone())
                            .into(),
                    );
                    format.insert("format_id".into(), format_id.clone().into());
                    format.insert(
                        "format_note".into(),
                        entry.get("description").cloned().unwrap_or(Value::Null),
                    );
                    format.insert(
                        "filesize".into(),
                        entry
                            .get("size_mb")
                            .and_then(Value::as_str)
                            .and_then(parse_filesize)
                            .into(),
                    );
                    format.insert("vcodec".into(), "none".into());
                    format.insert(
                        "acodec".into(),
                        format_id.split('-').next().unwrap_or_default().into(),
                    );
                    formats.push(format_from(format, &id)?);
                }
            }
        }
        let title = match &artist {
            Some(artist) => format!("{artist} - {}", track.as_deref().unwrap_or("None")),
            None => track.clone().unwrap_or_default(),
        };
        if duration.is_none_or(|duration| duration == 0.0) {
            duration = crate::html::html_search_meta(&webpage, "duration")
                .and_then(|text| text.trim().parse().ok());
        }
        let tags: Vec<Value> =
            crate::html_elements::elements_text_and_html_by_class("tag", &webpage)
                .unwrap_or_default()
                .into_iter()
                .map(|(content, _)| Value::from(crate::html::clean_html(&content)))
                .collect();
        let mut dict = Map::new();
        dict.insert("id".into(), track_id.clone().into());
        dict.insert("title".into(), title.into());
        dict.insert("thumbnail".into(), thumbnail.into());
        dict.insert("uploader".into(), artist.clone().into());
        dict.insert("uploader_id".into(), uploader.clone().into());
        dict.insert(
            "uploader_url".into(),
            format!("https://{uploader}.bandcamp.com").into(),
        );
        dict.insert("timestamp".into(), timestamp.into());
        dict.insert(
            "release_timestamp".into(),
            text_of(tralbum.get("album_release_date"))
                .and_then(|date| crate::dates::unified_timestamp(&date, false))
                .into(),
        );
        dict.insert(
            "duration".into(),
            duration.map_or(Value::Null, |d| json!(d)),
        );
        dict.insert("track".into(), track.into());
        dict.insert("track_number".into(), track_number.into());
        dict.insert("track_id".into(), track_id.into());
        dict.insert("artist".into(), artist.into());
        dict.insert(
            "album".into(),
            embed.get("album_title").cloned().unwrap_or(Value::Null),
        );
        dict.insert("album_artist".into(), album_artist.into());
        dict.insert("formats".into(), json!([]));
        dict.insert("tags".into(), Value::Array(tags));
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
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let mut keys = Vec::new();
        for (key, value) in dict {
            keys.push(key.clone());
            match key.as_str() {
                "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
                "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
                "formats" => {}
                _ if value.is_null() => {
                    info.extra.insert(key, Value::Null);
                }
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("dict;{}", keys.join(",")),
        )))
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://youtube-dl.bandcamp.com/track/youtube-dl-test-song"
        ));
        assert!(!suitable("https://benprunty.bandcamp.com/album/lanius"));
    }

    #[test]
    fn download_sizes_follow_parse_filesize() {
        assert_eq!(parse_filesize("7.2MB"), Some(7_200_000));
        assert_eq!(parse_filesize("1.5 MiB"), Some(1_572_864));
    }
}

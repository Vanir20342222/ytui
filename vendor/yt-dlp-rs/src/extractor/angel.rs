//! Upstream `AngelIE` (`yt_dlp/extractor/angel.py`): an episode page's
//! JSON-LD HLS stream with the OpenGraph title, description and source
//! thumbnail, merged with the remaining JSON-LD fields.
use super::{Extractor, download_webpage, hls, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Angel;

const NAME: &str = "Angel";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?angel\.com/watch/(?P<series>[^/?#]+)/episode/(?P<id>[\w-]+)/season-(?P<season_number>\d+)/episode-(?P<episode_number>\d+)/(?P<title>[^/?#]+)",
    )
    .unwrap()
});

static CLOUDINARY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(/upload)/.+(/angel-app/.+)$").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Angel {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &id)?;
        let mut json_ld = super::json_ld::search(&webpage);
        if json_ld.is_empty() {
            return Err(Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract JSON-LD; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        let manifest = json_ld.shift_remove("url").ok_or_else(|| {
            Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('url')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let manifest = Url::parse(manifest.as_str().unwrap_or_default())?;
        // `_extract_m3u8_formats_and_subtitles(..., note='Downloading HD
        // m3u8 information')` (fatal).
        crate::screen::expect_page(
            &id,
            Some(format!("[{NAME}] {id}: Downloading HD m3u8 information")),
            None,
        );
        let parsed = super::direct::request(client, &manifest, None)
            .map_err(|error| match error {
                Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                    extractor: NAME,
                    id: id.clone(),
                    what: "m3u8 information",
                    source: Box::new(error),
                },
                error => error,
            })
            .and_then(|response| hls::from_response(client, response, &manifest, None, true, None));
        crate::screen::clear_page();
        let stream = parsed?.ok_or_else(|| {
            Error::UnsupportedHtml5("the Angel stream is not an HLS playlist".into())
        })?;
        let search = |value: Option<String>, name: &str| {
            if value.is_none() {
                search_warning(NAME, name);
            }
            value
        };
        let title = search(
            crate::html::og_search_property(&webpage, "title"),
            "OpenGraph title",
        );
        let description = search(
            crate::html::og_search_property(&webpage, "description"),
            "OpenGraph description",
        );
        let og_thumbnail = search(
            crate::html::og_search_property(&webpage, "image"),
            "thumbnail URL",
        )
        .and_then(|thumbnail| super::json_ld::url_or_none(&thumbnail));
        let thumbnail = match og_thumbnail {
            Some(thumbnail) => Some(thumbnail),
            None => match json_ld.shift_remove("thumbnails") {
                Some(Value::String(thumbnail)) => Some(thumbnail),
                Some(Value::Array(_)) => {
                    return Err(Error::UnsupportedHtml5(
                        "Angel pages without an OpenGraph image are not supported natively".into(),
                    ));
                }
                _ => None,
            },
        }
        .map(|thumbnail| CLOUDINARY.replace(&thumbnail, "$1$2").into_owned());
        let mut info = stream;
        info.id = id;
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.description = description;
        let has_thumbnail = thumbnail.is_some();
        info.metadata.thumbnail = thumbnail;
        // `merge_dicts(info_dict, json_ld)`: the JSON-LD keys the info dict
        // lacks (or has as `None`), after its own.
        let own = [
            "id",
            "title",
            "description",
            "formats",
            "subtitles",
            "thumbnail",
        ];
        let mut keys = Vec::new();
        for (key, value) in json_ld {
            if value.is_null() {
                continue;
            }
            let filled = match key.as_str() {
                "title" => info.extra.contains_key(crate::info::TITLE_MISSING),
                "description" => info.metadata.description.is_none(),
                "thumbnail" => !has_thumbnail,
                _ => !own.contains(&key.as_str()),
            };
            if !filled {
                continue;
            }
            match key.as_str() {
                "title" => {
                    info.title = value.as_str().unwrap_or_default().to_owned();
                    info.extra.shift_remove(crate::info::TITLE_MISSING);
                }
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
            if !own.contains(&key.as_str()) {
                keys.push(key);
            }
        }
        let shape = format!("angel;{}", keys.join(","));
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.angel.com/watch/tuttle-twins/episode/2f3d0382-ba82-4f76-8b8c-89a0ee5ab5ed/season-1/episode-1/when-laws-go-bad"
        ));
        assert!(!suitable("https://www.angel.com/watch/tuttle-twins"));
        assert_eq!(
            CLOUDINARY.replace(
                "https://i.test/image/upload/c_fill/v1/angel-app/t/ep.jpg",
                "$1$2"
            ),
            "https://i.test/image/upload/angel-app/t/ep.jpg"
        );
    }
}

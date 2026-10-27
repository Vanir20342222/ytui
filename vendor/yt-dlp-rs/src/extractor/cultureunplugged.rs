//! Upstream `CultureUnpluggedIE` (`yt_dlp/extractor/cultureunplugged.py`):
//! the movie data JSON as an untyped result with its named thumbnails.
use super::{Extractor, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct CultureUnplugged;

const NAME: &str = "CultureUnplugged";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?cultureunplugged\.com/(?:documentary/watch-online/)?play/(?P<id>\d+)(?:/(?P<display_id>[^/#?]+))?",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for CultureUnplugged {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        let display_id = found
            .name("display_id")
            .map_or_else(|| id.clone(), |display_id| display_id.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = Url::parse(&format!(
            "http://www.cultureunplugged.com/movie-data/cu-{id}.json"
        ))?;
        let movie = download_json(client, &api, NAME, &display_id)?;
        let required = |name: &str| {
            let value = movie.get(name).ok_or_else(|| {
                Error::Reported(format!(
                    "{id}: An extractor error has occurred. (caused by KeyError('{name}')); {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
            value.as_str().map(str::to_owned).ok_or_else(|| {
                Error::UnsupportedHtml5(format!(
                    "Culture Unplugged {name} values that are not strings are not supported natively"
                ))
            })
        };
        let media = Url::parse(&required("url")?)?;
        let title = required("title")?;
        // `movie_data.get(f'{size}_thumb')` when truthy.
        let mut thumbnails = Vec::new();
        for (preference, size) in ["small", "large"].into_iter().enumerate() {
            match movie.get(format!("{size}_thumb")) {
                None | Some(Value::Null) => {}
                Some(Value::String(thumbnail)) if thumbnail.is_empty() => {}
                Some(Value::String(thumbnail)) => thumbnails.push(crate::info::Thumbnail {
                    url: thumbnail.clone(),
                    id: Some(size.into()),
                    preference: Some(preference as i64),
                    width: None,
                    height: None,
                    resolution: None,
                    extra: serde_json::Map::new(),
                }),
                Some(_) => {
                    return Err(Error::UnsupportedHtml5(
                        "Culture Unplugged thumbnails that are not strings are not supported natively"
                            .into(),
                    ));
                }
            }
        }
        let ext =
            super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, ext.clone()),
            "",
        );
        for format in &mut info.formats {
            format.ext = ext.clone();
            format.format_id = String::new();
        }
        info.format_id = None;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.thumbnails = thumbnails;
        crate::metadata_parser::set_field(&mut info, "display_id", display_id.into());
        let field = |name: &str| movie.get(name).cloned().unwrap_or(Value::Null);
        let duration = super::streamable::int_or_none(movie.get("duration"));
        for (name, value) in [
            ("description", field("synopsis")),
            ("creator", field("producer")),
            (
                "view_count",
                super::streamable::int_or_none(movie.get("views")),
            ),
        ] {
            if value.is_null() {
                info.extra.insert(name.into(), Value::Null);
            } else {
                crate::metadata_parser::set_field(&mut info, name, value);
            }
        }
        // Processing drops a `None` duration.
        if !duration.is_null() {
            crate::metadata_parser::set_field(&mut info, "duration", duration);
        }
        Ok(Some(crate::info_order::mark_info(info, "cultureunplugged")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.cultureunplugged.com/documentary/watch-online/play/53662/The-Next--Best-West"
        ));
        assert!(suitable("https://www.cultureunplugged.com/play/2833"));
        assert!(!suitable("https://www.cultureunplugged.com/play/"));
    }
}

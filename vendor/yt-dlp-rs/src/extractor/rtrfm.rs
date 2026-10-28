//! Upstream `RTRFMIE` (`yt_dlp/extractor/rtrfm.py`): a show page's
//! `playShow` details and the restream API's MP3 URL as an untyped result.
use super::{Extractor, download_json_note, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Rtrfm;

const NAME: &str = "RTRFM";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?rtrfm\.com\.au/(?:shows|show-episode)/(?P<id>[^/?\#&]+)")
        .unwrap()
});

static DETAILS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"\.playShow(?:From)?\(['"](?P<show>[^'"]+)['"],\s*['"](?P<date>[0-9]{4}-[0-9]{2}-[0-9]{2})['"],\s*['"](?P<title>[^'"]+)['"]"#,
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Rtrfm {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let details = DETAILS.captures(&webpage).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {display_id}: Unable to extract details; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let (show, date, title) = (&details["show"], &details["date"], &details["title"]);
        let api = super::update_url_query(
            &Url::parse("https://restreams.rtrfm.com.au/rzz")?,
            &[("n", show), ("d", date)],
        );
        let data = download_json_note(client, &api, NAME, show, "Downloading MP3 URL")?;
        let media = match data.get("u") {
            Some(Value::String(media)) => media.clone(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "RTRFM stream URLs that are not strings are not supported natively".into(),
                ));
            }
            None => {
                return Err(Error::Reported(format!(
                    "{display_id}: An extractor error has occurred. (caused by KeyError('u')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        // An MP4 URL is the only sign of an expired episode.
        if media.contains(".mp4") {
            return Err(Error::Reported(format!(
                "[{NAME}] {display_id}: Expired or no episode on this date"
            )));
        }
        let description = crate::html::og_search_property(&webpage, "description");
        if description.is_none() {
            search_warning(NAME, "OpenGraph description");
        }
        let media = Url::parse(&media)?;
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
        info.id = format!("{show}-{date}");
        info.title = format!("{title} {date}");
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        info.metadata.description = description;
        crate::metadata_parser::set_field(&mut info, "series", title.into());
        crate::metadata_parser::set_field(&mut info, "release_date", date.into());
        Ok(Some(crate::info_order::mark_info(info, "rtrfm")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://rtrfm.com.au/shows/breakfast/"));
        assert!(suitable(
            "https://rtrfm.com.au/show-episode/breakfast-2021-11-11/"
        ));
        assert!(!suitable("https://rtrfm.com.au/"));
    }
}

//! Upstream `HarpodeonIE` (`yt_dlp/extractor/harpodeon.py`): the page's
//! `hpBase`/`hpInjectVideo` MP4 as an untyped result with its `videoInfo`
//! title, creator and year.
use super::{Extractor, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Harpodeon;

const NAME: &str = "Harpodeon";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?harpodeon\.com/(?:video|preview)/\w+/(?P<id>\d+)").unwrap()
});
static INFO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"<div[^>]+videoInfo[^<]*<h2[^>]*>(?P<title>[^>]+)</h2>(?:\s*<p[^>]*>\((?P<creator>.+),\s*)?(?P<release_year>\d{4})?",
    )
    .unwrap()
});
static BASE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"hpBase\(\s*["']([^"']+)"#).unwrap());
static INJECT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"hpInjectVideo\(['"](?P<hp_inject_video>\w+)['"],['"](?P<hp_resolution>\d+)['"]"#)
        .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Harpodeon {
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
        let (title, creator, release_year) = match INFO.captures(&webpage) {
            Some(found) => (
                Some(found["title"].to_owned()),
                found
                    .name("creator")
                    .map(|creator| creator.as_str().to_owned()),
                found
                    .name("release_year")
                    .map(|year| year.as_str().to_owned()),
            ),
            None => {
                search_warning(NAME, "title");
                (None, None, None)
            }
        };
        let fatal = |what: &str| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract {what}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let base = BASE
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
            .ok_or_else(|| fatal("hp_base"))?;
        let inject = INJECT
            .captures(&webpage)
            .ok_or_else(|| fatal("hp_inject_video"))?;
        let media = Url::parse(&format!(
            "{base}{}_{}.mp4",
            &inject["hp_inject_video"], &inject["hp_resolution"]
        ))?;
        let description = crate::html::html_search_meta(&webpage, "description");
        if description.is_none() {
            search_warning(NAME, "description");
        }
        let referer =
            serde_json::Map::from_iter([("Referer".to_owned(), Value::from(url.as_str()))]);
        let ext =
            super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, ext.clone()),
            "",
        );
        for format in &mut info.formats {
            format.ext = ext.clone();
            format.format_id = String::new();
            format.http_headers = Some(referer.clone());
        }
        info.format_id = None;
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
        info.extra
            .insert("http_headers".into(), Value::Object(referer));
        info.extra
            .insert("creator".into(), creator.map_or(Value::Null, Value::from));
        info.extra.insert(
            "release_year".into(),
            release_year
                .and_then(|year| year.parse::<i64>().ok())
                .map_or(Value::Null, Value::from),
        );
        Ok(Some(crate::info_order::mark_info(info, "harpodeon")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.harpodeon.com/video/The_Smoking_Out_of_Bella_Butts/268068288"
        ));
        assert!(suitable(
            "https://www.harpodeon.com/preview/Behind_the_Screen/421838710"
        ));
        assert!(!suitable("https://www.harpodeon.com/video/x"));
    }
}

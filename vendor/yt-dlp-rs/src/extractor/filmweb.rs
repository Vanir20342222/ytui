//! Upstream `FilmwebIE` (`yt_dlp/extractor/filmweb.py`): the article's
//! trailer embed code, a `url_transparent` result of its 23video player.
use super::{Extractor, download_json, download_webpage, update_url_query};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Filmweb;

const NAME: &str = "Filmweb";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?filmweb\.no/(?P<type>trailere|filmnytt)/article(?P<id>\d+)\.ece",
    )
    .unwrap()
});
static VIDEO_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"data-videoid="(\d+)""#).unwrap());
static IFRAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<iframe[^>]+src="([^"]+)"#).unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn fatal(id: &str, what: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Unable to extract {what}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for Filmweb {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let mut article = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        if &found["type"] == "filmnytt" {
            let webpage = download_webpage(client, url, NAME, &article)?;
            article = VIDEO_ID
                .captures(&webpage)
                .map(|found| found[1].to_owned())
                .ok_or_else(|| fatal(&article, "article id"))?;
        }
        let api = update_url_query(
            &Url::parse("https://www.filmweb.no/template_v2/ajax/json_trailerEmbed.jsp")?,
            &[("articleId", &article)],
        );
        let data = download_json(client, &api, NAME, &article)?;
        let embed = data.get("embedCode").and_then(Value::as_str).ok_or_else(|| {
            Error::Reported(format!(
                "{article}: An extractor error has occurred. (caused by KeyError('embedCode')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let iframe = IFRAME
            .captures(embed)
            .map(|found| super::proto_relative(&found[1]))
            .ok_or_else(|| fatal(&article, "iframe url"))?;
        let target = Url::parse(&iframe)?;
        // `url_transparent` with `ie_key='TwentyThreeVideo'`; its only field
        // (`id`) is exempt from the update.
        let Some(mut info) = super::twentythreevideo::TwentyThreeVideo.extract(client, &target)?
        else {
            return Err(Error::UnsupportedHtml5(
                "Filmweb embeds other than 23video players are not supported natively".into(),
            ));
        };
        info.webpage_url = url.clone();
        let inner_shape = match info.extra.get(crate::info_order::INFO_SHAPE) {
            Some(Value::String(shape)) => shape.clone(),
            _ => String::new(),
        };
        let shape = format!("{inner_shape}|transparent:");
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info, &shape,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.filmweb.no/trailere/article1264921.ece"
        ));
        assert!(suitable(
            "http://www.filmweb.no/filmnytt/article1265000.ece"
        ));
        assert!(!suitable("http://www.filmweb.no/kino/article1.ece"));
    }
}

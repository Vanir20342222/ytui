//! Upstream `PlayerFmIE` (`yt_dlp/extractor/playerfm.py`): an episode's
//! `.json` document as an untyped audio result.
use super::{Extractor, download_json};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct PlayerFm;

const NAME: &str = "PlayerFm";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<url>https?://(?:www\.)?player\.fm/(?:series/)?[\w-]+/(?P<id>[\w-]+))")
        .unwrap()
});

/// Upstream `clean_podcast_url`'s tracking prefixes.
static TRACKING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:(?:chtbl\.com/track|media\.blubrry\.com|play\.podtrac\.com|chrt\.fm/track|mgln\.ai/e)(?:/[^/.]+)?|(?:dts|www)\.podtrac\.com/(?:pts/)?redirect\.[0-9a-z]{3,4}|flex\.acast\.com|pd(?:cn\.co|st\.fm)/e|[0-9]\.gum\.fm|pscrb\.fm/rss/p)/",
    )
    .unwrap()
});
static DOUBLE_SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\w+://(\w+://)").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Upstream `clean_podcast_url`.
pub(crate) fn clean_podcast_url(url: &str) -> String {
    let url = TRACKING.replace_all(url, "");
    DOUBLE_SCHEME.replace(&url, "$1").into_owned()
}

impl Extractor for PlayerFm {
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
        let api = Url::parse(&format!("{}.json", &found["url"]))?;
        let data = download_json(client, &api, NAME, &display_id)?;
        // `('id', {int}, {str_or_none})`: only an integer (not a bool).
        let Some(id) = data
            .get("id")
            .filter(|id| id.is_i64() || id.is_u64())
            .map(Value::to_string)
        else {
            return Err(Error::Reported(format!(
                "[{NAME}] Missing \"id\" field in extractor result; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let media = match data.get("url") {
            None | Some(Value::Null) => None,
            Some(Value::String(media)) => Some(clean_podcast_url(media)),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "Player.fm URLs that are not strings are not supported natively".into(),
                ));
            }
        };
        let Some(media) = media else {
            return Err(Error::Reported(format!(
                "[{NAME}] {id}: No video formats found!; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let media = Url::parse(&media)?;
        let ext =
            super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, ext.clone()),
            "",
        );
        for format in &mut info.formats {
            format.ext = ext.clone();
            format.vcodec = Some("none".into());
            format.format_id = String::new();
        }
        info.vcodec = Some("none".into());
        info.format_id = None;
        info.id = id;
        let title = data.get("title").and_then(Value::as_str);
        // Without one, the template has no `title` key.
        let shape = if title.is_some() {
            "playerfm"
        } else {
            "playerfm-untitled"
        };
        match title {
            Some(title) => info.title = title.to_owned(),
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
        crate::metadata_parser::set_field(&mut info, "display_id", display_id.into());
        info.metadata.description = data
            .get("description")
            .and_then(Value::as_str)
            .map(crate::html::clean_html);
        let int = |key: &str| super::streamable::int_or_none(data.get(key));
        // `(('image', ('series', 'image')), 'url', {url_or_none}, any)`.
        info.metadata.thumbnail = [
            data.get("image"),
            data.get("series").and_then(|series| series.get("image")),
        ]
        .into_iter()
        .flatten()
        .filter_map(|image| image.get("url")?.as_str())
        .find_map(super::json_ld::url_or_none);
        // The untyped result's own format has its `filesize`.
        let filesize = int("size").as_u64();
        for format in &mut info.formats {
            format.filesize = filesize;
        }
        for (key, value) in [
            ("duration", int("duration")),
            ("filesize", int("size")),
            ("timestamp", int("publishedAt")),
        ] {
            if !value.is_null() {
                crate::metadata_parser::set_field(&mut info, key, value);
            }
        }
        // `('series', 'author', {str}, filter, all, filter)`.
        if let Some(author) = data
            .get("series")
            .and_then(|series| series.get("author"))
            .and_then(Value::as_str)
            .filter(|author| !author.is_empty())
        {
            info.extra
                .insert("creators".into(), serde_json::json!([author]));
        }
        Ok(Some(crate::info_order::mark_info(info, shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://player.fm/series/chapo-trap-house/movie-mindset-33-casino-feat-felix"
        ));
        assert!(suitable("https://player.fm/1fK4hK1/x"));
        assert!(!suitable("https://player.fm/series"));
    }

    #[test]
    fn podcast_urls_are_cleaned_like_upstream() {
        // Values from the pinned upstream `clean_podcast_url`.
        for (input, expected) in [
            (
                "https://www.podtrac.com/pts/redirect.mp3/chtbl.com/track/5899E/traffic.megaphone.fm/HSW7835899191.mp3",
                "https://traffic.megaphone.fm/HSW7835899191.mp3",
            ),
            (
                "https://play.podtrac.com/npr-344098539/edge1.pod.npr.org/anon.npr-podcasts/podcast/npr/waitwait/2020/10/20201003_waitwait_wwdtmpodcast201003-015621a5-f035-4eca-a9a1-7c118d90bc3c.mp3",
                "https://edge1.pod.npr.org/anon.npr-podcasts/podcast/npr/waitwait/2020/10/20201003_waitwait_wwdtmpodcast201003-015621a5-f035-4eca-a9a1-7c118d90bc3c.mp3",
            ),
            (
                "https://pdst.fm/e/2.gum.fm/chtbl.com/track/chrt.fm/track/34D33/pscrb.fm/rss/p/traffic.megaphone.fm/ITLLC7765286967.mp3?updated=1687282661",
                "https://traffic.megaphone.fm/ITLLC7765286967.mp3?updated=1687282661",
            ),
            (
                "https://pdst.fm/e/https://mgln.ai/e/441/www.buzzsprout.com/1121972/13019085-ep-252-the-deep-life-stack.mp3",
                "https://www.buzzsprout.com/1121972/13019085-ep-252-the-deep-life-stack.mp3",
            ),
        ] {
            assert_eq!(clean_podcast_url(input), expected);
        }
    }
}

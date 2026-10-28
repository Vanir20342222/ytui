//! Upstream `YoutubeSearchIE` (`ytsearch[N|all]:QUERY`) and
//! `YoutubeSearchURLIE` (`/results?search_query=`), from
//! `yt_dlp/extractor/youtube/_search.py` and `_tab.py`'s `_search_results`.
use super::tab::{at, entry_info, extract_entries};
use super::watch;
use crate::{ExtractResult, Result};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub(crate) const NAME: &str = "youtube:search";
pub(crate) const URL_NAME: &str = "youtube:search_url";
/// `_SEARCH_PARAMS`: videos only.
const SEARCH_PARAMS: &str = "EgIQAfABAQ==";

static SEARCH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^ytsearch(?P<prefix>|[1-9][0-9]*|all):(?P<query>[\s\S]+)$").unwrap()
});
static URL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?youtube\.com/(?:results|search)\?([^#]+&)?(?:search_query|q)=(?:[^&]+)(?:[&#]|$)")
        .unwrap()
});

/// The input text of a `ytsearch` pseudo-URL (the URL parser
/// percent-encodes non-ASCII characters, which are decoded again).
fn input_text(url: &Url) -> String {
    let text = url.as_str();
    let mut bytes = Vec::with_capacity(text.len());
    let raw = text.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        if raw[index] == b'%'
            && let Some(byte) = text
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                .filter(|byte| *byte >= 0x80)
        {
            bytes.push(byte);
            index += 3;
            continue;
        }
        bytes.push(raw[index]);
        index += 1;
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `YoutubeSearchIE.suitable`.
pub fn suitable(url: &str) -> bool {
    SEARCH_RE.is_match(url)
}

/// `YoutubeSearchURLIE.suitable`.
pub fn url_suitable(url: &str) -> bool {
    URL_RE.is_match(url)
}

/// `_search_results(query, params)`: the search API pages until `limit`
/// entries (upstream's `islice` of its generator) or the last page.
fn search_results(
    client: &Client,
    extractor: &'static str,
    query: &str,
    params: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<Map<String, Value>>> {
    let display_id = format!("query \"{query}\"");
    // `_download_ytcfg('web', display_id)`.
    let home = Url::parse("https://www.youtube.com")?;
    let ytcfg = match super::super::download_webpage_note(
        client,
        &home,
        extractor,
        &display_id,
        "Downloading web client config",
    ) {
        Ok(webpage) => watch::extract_ytcfg(&webpage),
        Err(error) => {
            crate::screen::warning(&error.to_string());
            Map::new()
        }
    };
    let mut data = Map::new();
    data.insert("query".into(), query.into());
    if let Some(params) = params.filter(|params| !params.is_empty()) {
        data.insert("params".into(), params.into());
    }
    let mut entries = Vec::new();
    let mut previous: Option<Value> = None;
    for page in 1.. {
        let visitor = previous
            .as_ref()
            .and_then(watch::visitor_from)
            .or_else(|| watch::visitor_from(&Value::Object(ytcfg.clone())));
        let search = watch::innertube_api(
            client,
            "search",
            extractor,
            &format!("{display_id} page {page}"),
            &ytcfg,
            visitor.as_deref(),
            data.clone(),
            "Downloading API JSON",
        )?;
        let contents = [
            &[
                "contents",
                "twoColumnSearchResultsRenderer",
                "primaryContents",
                "sectionListRenderer",
                "contents",
            ][..],
            &[
                "onResponseReceivedCommands",
                "0",
                "appendContinuationItemsAction",
                "continuationItems",
            ][..],
            &[
                "contents",
                "tabbedSearchResultsRenderer",
                "tabs",
                "0",
                "tabRenderer",
                "content",
                "sectionListRenderer",
                "contents",
            ][..],
            &["continuationContents"][..],
        ]
        .iter()
        .find_map(|path| at(&search, path).filter(|value| !value.is_null()))
        .cloned()
        .unwrap_or(Value::Null);
        // `variadic`: a list stays a list.
        let contents = match contents {
            Value::Array(items) => items,
            Value::Null => Vec::new(),
            other => vec![other],
        };
        let mut found = Vec::new();
        let continuation =
            extract_entries(&serde_json::json!({ "contents": contents }), &mut found);
        for entry in found {
            if limit.is_some_and(|limit| entries.len() >= limit) {
                break;
            }
            entries.push(entry);
        }
        if limit.is_some_and(|limit| entries.len() >= limit) {
            break;
        }
        let Some(continuation) = continuation else {
            break;
        };
        data.extend(continuation);
        previous = Some(search);
    }
    Ok(entries)
}

/// `playlist_result(entries, query, query)`.
fn search_playlist(
    entries: Vec<Map<String, Value>>,
    query: &str,
    webpage_url: Url,
    extractor: &'static str,
    key: &'static str,
) -> Result<ExtractResult> {
    let entries = entries
        .into_iter()
        .map(entry_info)
        .collect::<Result<Vec<_>>>()?;
    let mut extra = Map::new();
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        "youtube-tab-plain;id,title".into(),
    );
    Ok(ExtractResult::Playlist(crate::PlaylistInfo {
        id: query.to_owned(),
        title: query.to_owned(),
        webpage_url,
        extractor: extractor.into(),
        extractor_key: key.into(),
        entries,
        extra,
    }))
}

fn announce(extractor: &'static str, url: &str) {
    crate::screen::set_extractor(extractor);
    crate::screen::screen(&format!(
        "[{extractor}] Extracting URL: {}",
        crate::screen::extracting_url(url)
    ));
}

pub struct YoutubeSearch;

impl super::super::Extractor for YoutubeSearch {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<crate::VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let text = input_text(url);
        let Some(found) = SEARCH_RE.captures(&text) else {
            return Ok(None);
        };
        announce(NAME, &text);
        let query = found["query"].to_owned();
        // `SearchInfoExtractor._real_extract` (`_MAX_RESULTS` is unbounded).
        let limit = match &found["prefix"] {
            "" => Some(1),
            "all" => None,
            count => count.parse().ok().or(Some(usize::MAX)),
        };
        let entries = search_results(client, NAME, &query, Some(SEARCH_PARAMS), limit)?;
        search_playlist(entries, &query, url.clone(), NAME, "YoutubeSearch").map(Some)
    }
}

pub struct YoutubeSearchUrl;

impl super::super::Extractor for YoutubeSearchUrl {
    fn key(&self) -> &'static str {
        URL_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<crate::VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !url_suitable(url.as_str()) {
            return Ok(None);
        }
        announce(URL_NAME, url.as_str());
        // `parse_qs`: blank values are dropped.
        let first = |key: &str| {
            url.query_pairs()
                .find(|(name, value)| name == key && !value.is_empty())
                .map(|(_, value)| value.into_owned())
        };
        let query = first("search_query")
            .or_else(|| first("q"))
            .unwrap_or_default();
        let params = first("sp");
        let entries = search_results(client, URL_NAME, &query, params.as_deref(), None)?;
        search_playlist(entries, &query, url.clone(), URL_NAME, "YoutubeSearchURL").map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_inputs_follow_upstream_patterns() {
        assert!(suitable("ytsearch:cats"));
        assert!(suitable("ytsearch10:cats and dogs"));
        assert!(suitable("ytsearchall:cats"));
        assert!(!suitable("ytsearch0:cats"));
        assert!(!suitable("ytsearch:"));
        assert!(url_suitable(
            "https://www.youtube.com/results?search_query=cats"
        ));
        assert!(url_suitable(
            "https://youtube.com/search?x=1&q=cats&sp=EgIQAg%253D%253D"
        ));
        assert!(!url_suitable(
            "https://www.youtube.com/results?search_query="
        ));
        let url = Url::parse("ytsearch5:caf\u{e9} %41").unwrap();
        assert_eq!(input_text(&url), "ytsearch5:caf\u{e9} %41");
    }
}

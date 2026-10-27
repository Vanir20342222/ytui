//! Upstream `BlueskyIE` (`yt_dlp/extractor/bluesky.py`): a post's thread
//! JSON, its video embeds' HLS playlists and PDS blobs (with captions),
//! external links as `url` results, quoted posts and the post metadata.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Bluesky;

const NAME: &str = "Bluesky";

static WEB_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?(?:bsky\.app|main\.bsky\.dev)/profile/(?P<handle>[\w.:%-]+)/post/(?P<id>\w+)")
        .unwrap()
});
static AT_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^at://(?P<handle>[\w.:%-]+)/app\.bsky\.feed\.post/(?P<id>\w+)").unwrap()
});

/// `BlueskyIE.suitable`.
pub fn suitable(url: &str) -> bool {
    WEB_URL.is_match(url) || AT_URL.is_match(url)
}

fn at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = match current {
            Value::Array(items) => items.get(key.parse::<usize>().ok()?)?,
            _ => current.get(*key)?,
        };
    }
    Some(current).filter(|value| !value.is_null())
}

fn int(value: Option<&Value>) -> Option<Value> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
        .map(Value::from)
}

fn text(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| value.is_string()).cloned()
}

fn url(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

fn profile(value: Option<&Value>) -> Option<Value> {
    value
        .and_then(Value::as_str)
        .map(|path| Value::from(format!("https://bsky.app/profile/{path}")))
}

/// A traversal path with more keys.
fn join<'a>(base: &[&'a str], suffix: &[&'a str]) -> Vec<&'a str> {
    let mut full = base.to_vec();
    full.extend(suffix);
    full
}

/// One `_extract_videos` entry.
enum Entry {
    Url(String),
    Video(
        Map<String, Value>,
        Vec<crate::VideoFormat>,
        crate::Subtitles,
    ),
}

/// `_get_service_endpoint(did)`: the account's PDS, else bsky.social.
fn service_endpoint(client: &Client, did: &str, id: &str) -> String {
    let address = match did.strip_prefix("did:web:") {
        Some(host) => format!("https://{host}/.well-known/did.json"),
        None => format!("https://plc.directory/{did}"),
    };
    let services = Url::parse(&address)
        .ok()
        .and_then(|url| {
            match super::download_json_note(client, &url, NAME, id, "Fetching service endpoint") {
                Ok(value) => Some(value),
                Err(error) => {
                    crate::screen::warning(&format!(
                        "[{NAME}] {id}: Falling back to bsky.social: {}",
                        crate::upstream_error_text(&error)
                    ));
                    None
                }
            }
        })
        .unwrap_or(Value::Null);
    services
        .get("service")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|service| {
            service.get("type").and_then(Value::as_str) == Some("AtprotoPersonalDataServer")
        })
        .find_map(|service| url(service.get("serviceEndpoint")))
        .unwrap_or_else(|| "https://bsky.social".into())
}

/// `_extract_videos(root, video_id, embed_path, record_path, record_subpath)`.
fn extract_videos(
    client: &Client,
    root: &Value,
    video_id: &str,
    embed_path: &[&str],
    record_path: &[&str],
    record_subpath: &[&str],
) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let record_embed: Vec<&str> = record_path.iter().chain(record_subpath).copied().collect();
    if let Some(external) = [record_embed.as_slice(), embed_path]
        .iter()
        .find_map(|path| {
            let mut full: Vec<&str> = path.to_vec();
            full.extend(["external", "uri"]);
            url(at(root, &full))
        })
    {
        entries.push(Entry::Url(external));
    }
    let mut playlist_path: Vec<&str> = embed_path.to_vec();
    playlist_path.push("playlist");
    let Some(playlist) = url(at(root, &playlist_path)) else {
        return Ok(entries);
    };
    let (mut formats, mut subtitles) = match Url::parse(&playlist).ok().and_then(|manifest| {
        super::extract_m3u8_optional(client, &manifest, NAME, video_id, Some("hls"), true)
    }) {
        Some(stream) => (stream.formats, stream.subtitles),
        None => (Vec::new(), crate::Subtitles::default()),
    };
    let path = |suffix: &[&'static str]| join(embed_path, suffix);
    let record = |suffix: &[&'static str]| join(&record_embed, suffix);
    let video_cid = text(at(root, &path(&["cid"])))
        .or_else(|| text(at(root, &record(&["video", "ref", "$link"]))))
        .and_then(|cid| cid.as_str().map(str::to_owned));
    let did = at(root, &["author", "did"])
        .and_then(Value::as_str)
        .map(str::to_owned);
    if let (Some(did), Some(cid)) = (&did, &video_cid) {
        let endpoint = service_endpoint(client, did, video_id);
        let blob_url = |cid: &str| -> Result<String> {
            Ok(super::update_url_query(
                &Url::parse(&format!("{endpoint}/xrpc/com.atproto.sync.getBlob"))?,
                &[("did", did), ("cid", cid)],
            )
            .to_string())
        };
        let mut format = Map::new();
        format.insert("format_id".into(), "blob".into());
        format.insert("quality".into(), 1.into());
        format.insert("url".into(), blob_url(cid)?.into());
        for (key, source) in [("width", "width"), ("height", "height")] {
            if let Some(value) = int(at(root, &path(&["aspectRatio", source]))) {
                format.insert(key.into(), value);
            }
        }
        if let Some(size) = int(at(root, &record(&["video", "size"]))) {
            format.insert("filesize".into(), size);
        }
        let mime_ext = at(root, &record(&["video", "mimeType"]))
            .and_then(Value::as_str)
            .map(|mime| super::direct::mimetype2ext_or(mime, None))
            .filter(|ext| !ext.is_empty());
        if let Some(ext) = &mime_ext {
            format.insert("ext".into(), ext.clone().into());
        }
        let keys: Vec<String> = format.keys().cloned().collect();
        if !format.contains_key("ext") {
            format.insert("ext".into(), "unknown_video".into());
        }
        let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
            .map_err(|error| Error::Reported(error.to_string()))?;
        crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
        for caption in at(root, &record(&["captions"]))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(caption_cid) = at(caption, &["file", "ref", "$link"]).and_then(Value::as_str)
            else {
                continue;
            };
            let language = caption
                .get("lang")
                .and_then(Value::as_str)
                .filter(|lang| !lang.is_empty())
                .unwrap_or("und")
                .to_owned();
            let ext = at(caption, &["file", "mimeType"])
                .and_then(Value::as_str)
                .map(|mime| super::direct::mimetype2ext_or(mime, None))
                .filter(|ext| !ext.is_empty())
                .unwrap_or_else(|| "unknown_video".into());
            let track: crate::SubtitleFormat =
                serde_json::from_value(json!({"url": blob_url(caption_cid)?, "ext": ext}))
                    .map_err(|error| Error::Reported(error.to_string()))?;
            let language = Some(language);
            match subtitles
                .entries
                .iter_mut()
                .find(|(key, _)| *key == language)
            {
                Some((_, tracks)) => tracks.push(track),
                None => subtitles.entries.push((language, vec![track])),
            }
        }
    }
    let labels: Vec<&str> = root
        .get("labels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|label| label.get("val").and_then(Value::as_str))
        .collect();
    let mut tags: Vec<Value> = Vec::new();
    for label in &labels {
        if !tags.iter().any(|tag| tag == label) {
            tags.push((*label).into());
        }
    }
    let mut dict = Map::new();
    dict.insert("id".into(), video_id.into());
    dict.insert("formats".into(), json!([]));
    dict.insert("subtitles".into(), json!({}));
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            dict.insert(key.to_owned(), value);
        }
    };
    put(
        "id",
        at(root, &["uri"])
            .and_then(Value::as_str)
            .map(|uri| Value::from(uri.rsplit('/').next().unwrap_or(uri))),
    );
    put(
        "thumbnail",
        url(at(root, &path(&["thumbnail"]))).map(Value::from),
    );
    put(
        "alt_title",
        text(at(root, &path(&["alt"]))).filter(|alt| alt != ""),
    );
    put("uploader", text(at(root, &["author", "displayName"])));
    put("uploader_id", text(at(root, &["author", "handle"])));
    put("uploader_url", profile(at(root, &["author", "handle"])));
    put("channel_id", text(at(root, &["author", "did"])));
    put("channel_url", profile(at(root, &["author", "did"])));
    put("like_count", int(root.get("likeCount")));
    put("repost_count", int(root.get("repostCount")));
    put("comment_count", int(root.get("replyCount")));
    put(
        "timestamp",
        root.get("indexedAt")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
            .map(Value::from),
    );
    put("tags", Some(Value::Array(tags)));
    put(
        "age_limit",
        labels
            .iter()
            .any(|label| matches!(*label, "sexual" | "porn" | "graphic-media"))
            .then(|| Value::from(18)),
    );
    let mut record_text: Vec<&str> = record_path.to_vec();
    record_text.push("text");
    let body = at(root, &record_text).and_then(Value::as_str);
    put(
        "description",
        body.filter(|text| !text.is_empty()).map(Value::from),
    );
    put(
        "title",
        body.map(|text| {
            let text = text.replace('\n', " ");
            let chars: Vec<char> = text.chars().collect();
            Value::from(if chars.len() <= 72 {
                text
            } else {
                format!("{}...", chars[..69].iter().collect::<String>())
            })
        }),
    );
    entries.push(Entry::Video(dict, formats, subtitles));
    Ok(entries)
}

fn video(
    page: &Url,
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
    subtitles: crate::Subtitles,
) -> VideoInfo {
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
    info.subtitles = subtitles;
    info.webpage_url = page.clone();
    info.extractor = NAME.into();
    info.extractor_key = NAME.into();
    let titled = dict.get("title").is_some_and(|title| !title.is_null());
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
            "formats" | "subtitles" => {}
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    crate::info_order::mark_info(
        info,
        &format!(
            "dict{};{};subtitle=url,ext",
            if titled { "" } else { "-untitled" },
            keys.join(",")
        ),
    )
}

fn follow(target: &str) -> Result<ExtractResult> {
    let target = Url::parse(target)?;
    let mut info = super::direct::build_at(target.clone(), &target, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra
        .insert(crate::FOLLOW_URL.into(), target.as_str().into());
    Ok(ExtractResult::Video(info))
}

impl Extractor for Bluesky {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = WEB_URL
            .captures(page.as_str())
            .or_else(|| AT_URL.captures(page.as_str()))
        else {
            return Ok(None);
        };
        let (handle, video_id) = (found["handle"].to_owned(), found["id"].to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        let mut api = Url::parse("https://public.api.bsky.app/xrpc/app.bsky.feed.getPostThread")?;
        api.query_pairs_mut()
            .append_pair(
                "uri",
                &format!("at://{handle}/app.bsky.feed.post/{video_id}"),
            )
            .append_pair("depth", "0")
            .append_pair("parentHeight", "0");
        let thread = super::download_json(client, &api, NAME, &video_id)?;
        let post = thread.pointer("/thread/post").cloned().ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: An extractor error has occurred. (caused by KeyError('thread')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let mut entries = extract_videos(
            client,
            &post,
            &video_id,
            &["embed"],
            &["record"],
            &["embed"],
        )?;
        entries.extend(extract_videos(
            client,
            &post,
            &video_id,
            &["embed", "media"],
            &["record"],
            &["embed", "media"],
        )?);
        if let Some(nested) = [
            at(&post, &["embed", "record", "record"]),
            at(&post, &["embed", "record"]),
        ]
        .into_iter()
        .flatten()
        .find(|value| value.is_object())
        .cloned()
        {
            entries.extend(extract_videos(
                client,
                &nested,
                &video_id,
                &["embeds", "0"],
                &["value"],
                &["embed"],
            )?);
        }
        if entries.is_empty() {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: No video could be found in this post"
            )));
        }
        if entries.len() == 1 {
            return match entries.pop() {
                Some(Entry::Url(target)) => follow(&target).map(Some),
                Some(Entry::Video(dict, formats, subtitles)) => Ok(Some(ExtractResult::Video(
                    video(page, dict, formats, subtitles),
                ))),
                None => unreachable!("one entry"),
            };
        }
        let mut videos = Vec::new();
        for entry in entries {
            match entry {
                Entry::Video(dict, formats, subtitles) => {
                    videos.push(video(page, dict, formats, subtitles))
                }
                Entry::Url(_) => {
                    return Err(Error::UnsupportedHtml5(
                        "Bluesky posts mixing videos and external links are not supported natively"
                            .into(),
                    ));
                }
            }
        }
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: video_id,
            title: String::new(),
            webpage_url: page.clone(),
            extractor: NAME.into(),
            extractor_key: NAME.into(),
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
            "https://bsky.app/profile/blu3blue.bsky.social/post/3l4omssdl632g"
        ));
        assert!(suitable(
            "https://main.bsky.dev/profile/did:plc:abc/post/3l4omssdl632g"
        ));
        assert!(suitable(
            "at://did:plc:abc/app.bsky.feed.post/3l4omssdl632g"
        ));
        assert!(!suitable("https://bsky.app/profile/blu3blue.bsky.social"));
    }
}

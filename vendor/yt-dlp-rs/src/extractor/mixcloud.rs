//! Upstream `MixcloudIE` (`yt_dlp/extractor/mixcloud.py`): the GraphQL
//! cloudcast lookup (requested with upstream's impersonation warning), its
//! XOR-encrypted stream URLs (progressive, HLS, DASH), comments and
//! metadata.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Mixcloud;

const NAME: &str = "mixcloud";
const KEY: &str = "Mixcloud";
const DECRYPTION_KEY: &[u8] = b"IFYOUWANTTHEARTISTSTOGETPAIDDONOTDOWNLOADFROMMIXCLOUD";
const CLOUDCAST_FIELDS: &str = "audioLength
    comments(first: 100) {
      edges {
        node {
          comment
          created
          user {
            displayName
            username
          }
        }
      }
      totalCount
    }
    description
    favorites {
      totalCount
    }
    featuringArtistList
    isExclusive
    name
    owner {
      displayName
      url
      username
    }
    picture(width: 1024, height: 1024) {
        url
    }
    plays
    publishDate
    reposts {
      totalCount
    }
    streamInfo {
      dashUrl
      hlsUrl
      url
    }
    tags {
      tag {
        name
      }
    }
    restrictedReason
    id";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:www|beta|m)\.)?mixcloud\.com/(?P<user>[^/]+)/(?P<slug>[^/]+)")
        .unwrap()
});

/// `MixcloudIE._match_valid_url` (`(?!stream|uploads|...)` checked apart).
fn match_url(url: &str) -> Option<(String, String)> {
    let found = VALID_URL.captures(url)?;
    let slug = &found["slug"];
    if ["stream", "uploads", "favorites", "listens", "playlists"]
        .iter()
        .any(|reserved| slug.starts_with(reserved))
    {
        return None;
    }
    let decode = |text: &str| {
        percent_encoding::percent_decode_str(text)
            .decode_utf8_lossy()
            .into_owned()
    };
    Some((decode(&found["user"]), decode(slug)))
}

/// `MixcloudIE.suitable`.
pub fn suitable(url: &str) -> bool {
    match_url(url).is_some()
}

/// `_decrypt_xor_cipher(key, base64.b64decode(url))`.
fn decrypt(encoded: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    Some(
        bytes
            .iter()
            .zip(DECRYPTION_KEY.iter().cycle())
            .map(|(byte, key)| char::from(byte ^ key))
            .collect(),
    )
}

/// An `ExtractorError`: the URL pattern has no `id` group, so there is no
/// temporary ID before the message.
fn fail(_id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {message}"))
}

fn int(value: Option<&Value>) -> Option<Value> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
        .map(Value::from)
}

impl Extractor for Mixcloud {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some((username, slug)) = match_url(url.as_str()) else {
            return Ok(None);
        };
        let track_id = format!("{username}_{slug}");
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        crate::screen::warning_once(&format!(
            "[{NAME}] The extractor is attempting impersonation, but no impersonate target is available. If you encounter errors, then see  https://github.com/yt-dlp/yt-dlp#impersonation  for information on installing the required dependencies"
        ));
        let query = format!(
            "{{\n  cloudcastLookup(lookup: {{username: \"{username}\", slug: \"{slug}\"}}) {{\n    {CLOUDCAST_FIELDS}\n  }}\n}}"
        );
        let mut api = Url::parse("https://app.mixcloud.com/graphql")?;
        api.query_pairs_mut().append_pair("query", &query);
        let response = super::download_json(client, &api, NAME, &track_id)?;
        let cloudcast = response
            .pointer("/data/cloudcastLookup")
            .filter(|value| !value.is_null())
            .cloned()
            .ok_or_else(|| fail(&track_id, "Track not found"))?;
        match cloudcast.get("restrictedReason").and_then(Value::as_str) {
            Some("tracklist") => {
                return Err(fail(
                    &track_id,
                    "Track unavailable in your country due to licensing restrictions",
                ));
            }
            Some("repeat_play") => {
                return Err(fail(
                    &track_id,
                    "You have reached your play limit for this track",
                ));
            }
            Some(reason) if !reason.is_empty() => {
                return Err(fail(&track_id, "Track is restricted"));
            }
            _ => {}
        }
        let stream_info = cloudcast.get("streamInfo").cloned().unwrap_or(Value::Null);
        let mut formats = Vec::new();
        for key in ["url", "hlsUrl", "dashUrl"] {
            let Some(encoded) = stream_info
                .get(key)
                .and_then(Value::as_str)
                .filter(|u| !u.is_empty())
            else {
                continue;
            };
            let Some(decrypted) = decrypt(encoded) else {
                continue;
            };
            match key {
                "hlsUrl" => {
                    if let Some(stream) = Url::parse(&decrypted).ok().and_then(|manifest| {
                        super::extract_m3u8_optional(
                            client,
                            &manifest,
                            NAME,
                            &track_id,
                            Some("hls"),
                            false,
                        )
                    }) {
                        formats.extend(stream.formats);
                    }
                }
                "dashUrl" => {
                    if let Some(stream) = Url::parse(&decrypted).ok().and_then(|manifest| {
                        super::extract_mpd_optional_note(
                            client,
                            &manifest,
                            NAME,
                            &track_id,
                            Some("dash"),
                            "Downloading MPD manifest",
                        )
                    }) {
                        let mut stream = stream;
                        if !stream.subtitles.is_empty() {
                            crate::screen::warning_once(&format!(
                                "[{NAME}] Ignoring subtitle tracks found in the DASH manifest; if any subtitle tracks are missing, {}",
                                crate::BUG_REPORTS_MESSAGE
                            ));
                            stream.subtitles = Default::default();
                        }
                        formats.extend(stream.formats);
                    }
                }
                _ => {
                    let ext = super::direct::determine_ext(&decrypted)
                        .unwrap_or_else(|| "unknown_video".into());
                    let mut format: crate::VideoFormat = serde_json::from_value(json!({
                        "format_id": "http",
                        "url": decrypted,
                        "vcodec": "none",
                        "downloader_options": {"http_chunk_size": 5242880},
                        "ext": ext,
                    }))
                    .map_err(|error| fail(&track_id, &error.to_string()))?;
                    crate::info_order::mark_format(
                        &mut format,
                        "dict:format_id,url,vcodec,downloader_options",
                    );
                    formats.push(format);
                }
            }
        }
        if formats.is_empty() && cloudcast.get("isExclusive").and_then(Value::as_bool) == Some(true)
        {
            return Err(fail(
                &track_id,
                "This video is only available for registered users. Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies",
            ));
        }
        let mut comments = Vec::new();
        for node in cloudcast
            .pointer("/comments/edges")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|edge| edge.get("node").filter(|node| node.is_object()))
        {
            let Some(text) = node
                .get("comment")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
            else {
                continue;
            };
            let mut comment = Map::new();
            comment.insert("text".into(), text.into());
            if let Some(author) = node.pointer("/user/displayName").filter(|v| v.is_string()) {
                comment.insert("author".into(), author.clone());
            }
            if let Some(author_id) = node.pointer("/user/username").filter(|v| v.is_string()) {
                comment.insert("author_id".into(), author_id.clone());
            }
            if let Some(timestamp) = node
                .get("created")
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
            {
                comment.insert("timestamp".into(), timestamp.into());
            }
            comments.push(Value::Object(comment));
        }
        let mut dict = Map::new();
        dict.insert("id".into(), track_id.clone().into());
        dict.insert("formats".into(), json!([]));
        dict.insert("comments".into(), Value::Array(comments));
        let text = |path: &str| {
            cloudcast
                .pointer(path)
                .filter(|value| value.is_string())
                .cloned()
        };
        let list = |values: Vec<Value>| (!values.is_empty()).then_some(Value::Array(values));
        let tags: Vec<Value> = cloudcast
            .get("tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tag| tag.pointer("/tag/name").and_then(Value::as_str))
            .filter(|tag| !tag.is_empty())
            .map(Value::from)
            .collect();
        let artists: Vec<Value> = cloudcast
            .get("featuringArtistList")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|artist| !artist.is_empty())
            .map(Value::from)
            .collect();
        for (key, value) in [
            ("title", text("/name")),
            ("description", text("/description")),
            (
                "thumbnail",
                cloudcast
                    .pointer("/picture/url")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                    .map(Value::from),
            ),
            (
                "timestamp",
                cloudcast
                    .get("publishDate")
                    .and_then(Value::as_str)
                    .and_then(crate::dates::parse_iso8601)
                    .map(Value::from),
            ),
            ("duration", int(cloudcast.get("audioLength"))),
            ("uploader", text("/owner/displayName")),
            ("uploader_id", text("/owner/username")),
            (
                "uploader_url",
                cloudcast
                    .pointer("/owner/url")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                    .map(Value::from),
            ),
            ("view_count", int(cloudcast.get("plays"))),
            (
                "like_count",
                int(cloudcast.pointer("/favorites/totalCount")),
            ),
            (
                "repost_count",
                int(cloudcast.pointer("/reposts/totalCount")),
            ),
            (
                "comment_count",
                int(cloudcast.pointer("/comments/totalCount")),
            ),
            ("tags", list(tags)),
            ("artists", list(artists)),
        ] {
            if let Some(value) = value {
                dict.insert(key.into(), value);
            }
        }
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
        info.extractor_key = KEY.into();
        let titled = dict.contains_key("title");
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
                "formats" => {}
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!(
                "dict{};{}",
                if titled { "" } else { "-untitled" },
                keys.join(",")
            ),
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
        assert_eq!(
            match_url("http://www.mixcloud.com/dholbach/cryptkeeper/"),
            Some(("dholbach".into(), "cryptkeeper".into()))
        );
        assert!(suitable(
            "https://beta.mixcloud.com/RedLightRadio/nosedrip-15-red-light-radio-01-18-2016/"
        ));
        assert!(!suitable("https://www.mixcloud.com/dholbach/uploads/"));
        assert!(!suitable("https://www.mixcloud.com/dholbach/"));
    }

    #[test]
    fn stream_urls_are_xor_decrypted() {
        let plain = "https://stream.example/a.m4a";
        let cipher: Vec<u8> = plain
            .bytes()
            .zip(DECRYPTION_KEY.iter().cycle())
            .map(|(byte, key)| byte ^ key)
            .collect();
        let encoded = base64::engine::general_purpose::STANDARD.encode(cipher);
        assert_eq!(decrypt(&encoded).as_deref(), Some(plain));
    }
}

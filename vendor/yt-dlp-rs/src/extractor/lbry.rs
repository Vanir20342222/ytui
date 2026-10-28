//! Upstream `LBRYIE` (`yt_dlp/extractor/lbry.py`, `lbry`/odysee.com): the
//! API proxy's `resolve` and `get` (streaming URL), the original file when
//! the v3 API serves one, the streaming redirect's HLS formats, livestreams
//! and the claim's metadata.
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Lbry;

const NAME: &str = "lbry";
const KEY: &str = "LBRY";
const CLAIM: &str = "[0-9a-f]{1,40}";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    let opt = format!(r"[^$@:/?#&]+(?:[:#]{CLAIM})?");
    Regex::new(&format!(
        r"^(?:https?://(?:www\.)?(?:lbry\.tv|odysee\.com)/|lbry://)(?:\$/(?:download|embed)/)?(?P<id>[^$@:/?#]+/{CLAIM}|(?:@{opt}/)?{opt})"
    ))
    .unwrap()
});
static API_VERSION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/api/v\d+/").unwrap());

/// `LBRYIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

fn int(value: Option<&Value>) -> Option<Value> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
        .map(Value::from)
}

/// `_call_api_proxy(method, display_id, params, resource)`.
/// Errors carry `temp_id`, extract's temporary ID (the URL's raw match).
fn call_api(
    client: &Client,
    method: &str,
    id: &str,
    temp_id: &str,
    params: Value,
    resource: &str,
) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!(
        "[{NAME}] {id}: Downloading {resource} JSON metadata"
    ));
    let mut headers = vec![("Content-Type", "application/json-rpc".to_owned())];
    if let Some(token) = crate::cookies::get_cookie("https://odysee.com", "auth_token") {
        headers.push(("x-lbry-auth-token", token));
    }
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let body = crate::output::python_json_dumps(&json!({"method": method, "params": params}));
    let url = Url::parse("https://api.lbry.tv/api/v1/proxy")?;
    let mut response =
        super::direct::post(client, &url, &headers, body).map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: NAME,
                id: id.to_owned(),
                what: "JSON metadata",
                source: Box::new(error),
            },
            error => error,
        })?;
    let mut text = Vec::new();
    response
        .read_to_end(&mut text)
        .map_err(Error::ExtractionRead)?;
    let response: Value = serde_json::from_slice(&text)
        .map_err(|error| fail(id, &format!("Failed to parse JSON  (caused by {error})")))?;
    if let Some(error) = response
        .get("error")
        .filter(|error| !error.is_null() && error.as_object().is_none_or(|e| !e.is_empty()))
    {
        let field = |key: &str| match error.get(key) {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Null) | None => "None".into(),
            Some(other) => other.to_string(),
        };
        return Err(fail(
            temp_id,
            &format!("{NAME} said: {} - {}", field("code"), field("message")),
        ));
    }
    Ok(response.get("result").cloned().unwrap_or(Value::Null))
}

/// `_permanent_url(url, claim_name, claim_id)`.
fn permanent_url(url: &str, name: &str, claim: &str) -> Option<String> {
    let base = Url::parse(&url.replace("lbry://", "https://lbry.tv/")).ok()?;
    base.join(&format!("/{name}:{claim}"))
        .ok()
        .map(String::from)
}

/// `_parse_stream(stream, url)`.
fn parse_stream(stream: &Value, url: &str) -> Map<String, Value> {
    let stream_type = stream.pointer("/value/stream_type").and_then(Value::as_str);
    let text = |path: &str| {
        stream
            .pointer(path)
            .filter(|value| value.is_string())
            .cloned()
    };
    let mut info = Map::new();
    let mut put = |key: &str, value: Option<Value>| {
        if let Some(value) = value {
            info.insert(key.to_owned(), value);
        }
    };
    put("title", text("/value/title"));
    put(
        "thumbnail",
        stream
            .pointer("/value/thumbnail/url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map(Value::from),
    );
    put("description", text("/value/description"));
    put("license", text("/value/license"));
    put("timestamp", int(stream.get("timestamp")));
    put(
        "release_timestamp",
        int(stream.pointer("/value/release_time")),
    );
    let tags: Vec<Value> = stream
        .pointer("/value/tags")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|tag| match tag {
            Value::String(text) => !text.is_empty(),
            Value::Null => false,
            _ => true,
        })
        .cloned()
        .collect();
    put("tags", (!tags.is_empty()).then_some(Value::Array(tags)));
    put(
        "duration",
        stream_type.and_then(|kind| int(stream.pointer(&format!("/value/{kind}/duration")))),
    );
    put("channel", text("/signing_channel/value/title"));
    put("channel_id", text("/signing_channel/claim_id"));
    put("uploader_id", text("/signing_channel/name"));
    if let (Some(uploader), Some(channel)) = (
        info.get("uploader_id")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty()),
        info.get("channel_id")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty()),
    ) && let Some(channel_url) = permanent_url(url, uploader, channel)
    {
        info.insert("channel_url".into(), channel_url.into());
    }
    info
}

/// `urlhandle_detect_ext(urlh)`.
fn response_ext(response: &reqwest::blocking::Response) -> Option<String> {
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    if let Some(disposition) = header("content-disposition") {
        static FILENAME: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"filename\s*=\s*"?([^";]+)"?"#).unwrap());
        if let Some(ext) = FILENAME
            .captures(&disposition)
            .and_then(|found| super::direct::determine_ext(&found[1]))
        {
            return Some(ext);
        }
    }
    if let Some(ext) =
        header("x-amz-meta-name").and_then(|name| super::direct::determine_ext(&name))
    {
        return Some(ext);
    }
    if let Some(kind) = header("x-amz-meta-file-type") {
        return Some(kind);
    }
    header("content-type")
        .map(|mime| super::direct::mimetype2ext_or(&mime, None))
        .filter(|ext| !ext.is_empty())
}

impl Extractor for Lbry {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, page: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let temp_id = found["id"].to_owned();
        let mut display_id = temp_id.clone();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        display_id = if display_id.starts_with('@') {
            display_id.replace(':', "#")
        } else {
            display_id.replace('/', ":")
        };
        display_id = percent_encoding::percent_decode_str(&display_id)
            .decode_utf8_lossy()
            .into_owned();
        let uri = format!("lbry://{display_id}");
        let resolved = call_api(
            client,
            "resolve",
            &display_id,
            &temp_id,
            json!({"urls": uri}),
            "stream",
        )?;
        let result = resolved.get(&uri).cloned().ok_or_else(|| {
            fail(
                &display_id,
                &format!(
                    "An extractor error has occurred. (caused by KeyError({uri:?})); {}",
                    crate::BUG_REPORTS_MESSAGE
                ),
            )
        })?;
        let headers = [("Referer", "https://odysee.com/")];
        let stream_type = result.pointer("/value/stream_type").and_then(Value::as_str);
        let mut formats = Vec::new();
        let mut final_url: Option<String> = None;
        let (claim_id, is_live) = if matches!(stream_type, Some("video" | "audio")) {
            let claim_id = result
                .get("claim_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let mut params = Map::new();
            params.insert("uri".into(), uri.clone().into());
            for key in ["signature", "signature_ts"] {
                if let Some((_, value)) = page
                    .query_pairs()
                    .find(|(name, value)| name == key && !value.is_empty())
                {
                    params.insert(key.into(), value.into_owned().into());
                }
            }
            let streaming = call_api(
                client,
                "get",
                &claim_id,
                &temp_id,
                Value::Object(params),
                "streaming url",
            )?;
            let streaming_url = streaming
                .get("streaming_url")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    fail(
                        &claim_id,
                        "An extractor error has occurred. (caused by KeyError('streaming_url'))",
                    )
                })?
                .to_owned();
            // The v3 API answers with the original file when available.
            let direct_url = API_VERSION.replace(&streaming_url, "/api/v3/").into_owned();
            crate::screen::screen(&format!(
                "[{NAME}] {display_id}: Checking for original quality"
            ));
            let original = Url::parse(&direct_url).ok().and_then(|direct| {
                super::direct::request_with_headers(client, &direct, None, &headers).ok()
            });
            if let Some(response) = original
                && response_ext(&response).as_deref() != Some("m3u8")
            {
                let value = result.get("value").cloned().unwrap_or(Value::Null);
                let ext = value
                    .pointer("/source/name")
                    .and_then(Value::as_str)
                    .and_then(super::direct::determine_ext)
                    .or_else(|| {
                        value
                            .pointer("/source/media_type")
                            .and_then(Value::as_str)
                            .map(|mime| super::direct::mimetype2ext_or(mime, None))
                            .filter(|ext| !ext.is_empty())
                    });
                let mut format = Map::new();
                format.insert("url".into(), direct_url.clone().into());
                format.insert("format_id".into(), "original".into());
                format.insert("quality".into(), 1.into());
                let mut keys = vec!["url", "format_id", "quality"];
                if let Some(ext) = &ext {
                    format.insert("ext".into(), ext.clone().into());
                    keys.push("ext");
                }
                for (key, path) in [
                    ("filesize", "/source/size"),
                    ("width", "/video/width"),
                    ("height", "/video/height"),
                ] {
                    if let Some(number) = int(value.pointer(path)) {
                        format.insert(key.into(), number);
                        keys.push(key);
                    }
                }
                if stream_type == Some("audio") {
                    format.insert("vcodec".into(), "none".into());
                }
                keys.push("vcodec");
                if !format.contains_key("ext") {
                    format.insert(
                        "ext".into(),
                        super::direct::determine_ext(&direct_url)
                            .unwrap_or_else(|| "unknown_video".into())
                            .into(),
                    );
                }
                let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
                    .map_err(|error| fail(&display_id, &error.to_string()))?;
                crate::info_order::mark_format(&mut format, &format!("dict:{}", keys.join(",")));
                crate::info_order::mark_int_rates(&mut format);
                formats.push(format);
            }
            crate::screen::screen(&format!(
                "[{NAME}] {display_id}: Downloading streaming redirect url info"
            ));
            if let Ok(response) = Url::parse(&streaming_url)
                .map_err(Error::from)
                .and_then(|stream| super::direct::head_with_headers(client, &stream, &headers))
            {
                final_url = Some(response.url().to_string());
            }
            (claim_id, false)
        } else if result.get("value_type").and_then(Value::as_str) == Some("stream")
            && stream_type != Some("binary")
        {
            let claim_id = result
                .pointer("/signing_channel/claim_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let mut api = Url::parse("https://api.odysee.live/livestream/is_live")?;
            api.query_pairs_mut()
                .append_pair("channel_claim_id", &claim_id);
            let live = super::download_json_note(
                client,
                &api,
                NAME,
                &claim_id,
                "Downloading livestream JSON metadata",
            )?;
            let data = live.get("data").cloned().unwrap_or(Value::Null);
            if data.get("Live").and_then(Value::as_bool) != Some(true) {
                return Err(fail(&claim_id, "This stream is not live"));
            }
            final_url = data
                .get("VideoURL")
                .and_then(Value::as_str)
                .map(str::to_owned);
            (claim_id, true)
        } else {
            return Err(Error::Reported(format!("Unsupported URL: {page}")));
        };
        if let Some(final_url) = &final_url
            && super::direct::determine_ext(final_url).as_deref() == Some("m3u8")
        {
            let manifest = Url::parse(final_url)?;
            let mut stream = super::extract_m3u8_fatal(
                client,
                &manifest,
                NAME,
                &display_id,
                Some("hls"),
                false,
            )?;
            if is_live {
                super::hls::live_format_ids(&mut stream.formats, Some("hls"));
            }
            formats.extend(stream.formats);
        }
        let mut dict = parse_stream(&result, page.as_str());
        dict.insert("id".into(), claim_id.into());
        dict.insert("formats".into(), json!([]));
        dict.insert("is_live".into(), is_live.into());
        dict.insert(
            "http_headers".into(),
            json!({"Referer": "https://odysee.com/"}),
        );
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
        info.webpage_url = page.clone();
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
        assert!(suitable("https://lbry.tv/@Mantega:1/First-day-LBRY:1"));
        assert!(suitable(
            "https://odysee.com/@gardeningincanada:b/plants-i-will-never-grow-again.-the:e"
        ));
        assert!(suitable(
            "https://odysee.com/$/embed/@Mantega:1/First-day-LBRY:1"
        ));
        assert!(suitable("lbry://@lbry#3f/odysee#7"));
        assert!(suitable(
            "https://odysee.com/Episode-1:e7d93d772bd87e2b62d5ab993c1c3ced86ebb396"
        ));
        assert!(!suitable("https://example.com/@x"));
    }
}

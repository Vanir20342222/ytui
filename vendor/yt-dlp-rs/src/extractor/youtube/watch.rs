//! Upstream `YoutubeIE._real_extract`'s acquisition: the watch page, its
//! `ytcfg`, initial data and player response, then one player API request
//! per requested InnerTube client (`_extract_player_responses`), whose
//! responses feed [`super::streams`].
use super::{challenge, innertube, streams};
use crate::{Error, Result};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

/// `_DEFAULT_CLIENTS`, `_DEFAULT_JSLESS_CLIENTS` and `_DEFAULT_AUTHED_CLIENTS`.
const DEFAULT_CLIENTS: [&str; 3] = ["tv_samsung", "visionos", "web"];
const DEFAULT_JSLESS_CLIENTS: [&str; 1] = ["visionos"];
const DEFAULT_AUTHED_CLIENTS: [&str; 3] = ["web_embedded", "tv_samsung", "web"];
const WEBPAGE_CLIENT: &str = "web";
const EJS_WIKI_URL: &str = "https://github.com/yt-dlp/yt-dlp/wiki/EJS";
/// `_PLAYER_JS_VARIANT_MAP['main']`.
const MAIN_PLAYER_VARIANT: &str = "player_ias.vflset/en_US/base.js";

/// Whether `_report_pot_format_skipped` only writes a debug message.
pub(super) fn is_default_client(client: &str) -> bool {
    DEFAULT_CLIENTS.contains(&client) || DEFAULT_AUTHED_CLIENTS.contains(&client)
}

/// `extract_ytcfg`: `ytcfg\.set\s*\(\s*({.+?})\s*\)\s*;`.
pub(super) fn extract_ytcfg(webpage: &str) -> Map<String, Value> {
    static YTCFG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?s)ytcfg\.set\s*\(\s*(\{.+?\})\s*\)\s*;").unwrap());
    YTCFG
        .captures(webpage)
        .and_then(|found| serde_json::from_str::<Value>(&found[1]).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default()
}

/// `_search_json(START, webpage, NAME, video_id, fatal=False)`.
fn search_initial(webpage: &str, start: &Regex, name: &str) -> Option<Value> {
    match super::super::search_json(start, webpage, false) {
        Some(Some(value)) => Some(value),
        Some(None) => {
            crate::screen::warning(&format!(
                "[youtube] Unable to extract {name} - Failed to parse JSON"
            ));
            None
        }
        None => {
            super::super::search_warning("youtube", name);
            None
        }
    }
}

/// The default `ytcfg` of a client (`_get_default_ytcfg`) as JSON.
fn default_ytcfg(client: &innertube::ClientConfig) -> Map<String, Value> {
    let mut ytcfg = Map::new();
    let context: Value = serde_json::from_str(client.context).unwrap_or(Value::Null);
    ytcfg.insert("INNERTUBE_CONTEXT".into(), context);
    ytcfg.insert(
        "INNERTUBE_CONTEXT_CLIENT_NAME".into(),
        client.client_name_id.into(),
    );
    ytcfg
}

/// `_ytcfg_get_safe`: a `ytcfg` value, else the client default's.
fn ytcfg_value<'a>(
    ytcfg: &'a Map<String, Value>,
    default: &'a Map<String, Value>,
    path: &[&str],
) -> Option<&'a Value> {
    let get = |map: &'a Map<String, Value>| {
        let mut value = map.get(path[0])?;
        for key in &path[1..] {
            value = value.get(*key)?;
        }
        Some(value).filter(|value| match value {
            Value::Null => false,
            Value::String(text) => !text.is_empty(),
            Value::Number(number) => number.as_f64() != Some(0.0),
            _ => true,
        })
    };
    get(ytcfg).or_else(|| get(default))
}

/// `_extract_player_url`: `PLAYER_JS_URL` (else a `jsUrl`) under
/// `_construct_player_url`'s `main` variant.
fn player_url(ytcfgs: &[&Map<String, Value>]) -> Option<String> {
    let found = ytcfgs
        .iter()
        .find_map(|ytcfg| ytcfg.get("PLAYER_JS_URL").and_then(Value::as_str))
        .or_else(|| {
            ytcfgs.iter().find_map(|ytcfg| {
                ytcfg
                    .get("WEB_PLAYER_CONTEXT_CONFIGS")?
                    .as_object()?
                    .values()
                    .find_map(|config| config.get("jsUrl")?.as_str())
            })
        })?;
    static PLAYER_ID: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"/s/player/(?P<id>[a-fA-F0-9]{8,})/").unwrap());
    let id = PLAYER_ID.captures(found)?["id"].to_owned();
    let base = Url::parse("https://www.youtube.com").ok()?;
    let path = found
        .split_once(&format!("/s/player/{id}/"))
        .map_or("", |(_, path)| path);
    if path == MAIN_PLAYER_VARIANT {
        return base.join(found).ok().map(String::from);
    }
    base.join(&format!("/s/player/{id}/{MAIN_PLAYER_VARIANT}"))
        .ok()
        .map(String::from)
}

/// `_extract_response(ep=ENDPOINT)` for the web client (tab pages).
#[allow(clippy::too_many_arguments)]
pub(super) fn innertube_api(
    client: &Client,
    endpoint: &str,
    extractor: &'static str,
    item_id: &str,
    ytcfg: &Map<String, Value>,
    visitor: Option<&str>,
    query: Map<String, Value>,
    note: &str,
) -> Result<Value> {
    let config = innertube::client(WEBPAGE_CLIENT).expect("the web client is defined");
    call_api(
        client, config, endpoint, extractor, item_id, ytcfg, visitor, query, note,
    )
}

/// `_extract_visitor_data` of one source.
pub(super) fn visitor_from(source: &Value) -> Option<String> {
    visitor_data(&[source])
}

/// `_extract_visitor_data`.
fn visitor_data(sources: &[&Value]) -> Option<String> {
    for source in sources {
        for path in [
            &["VISITOR_DATA"][..],
            &["INNERTUBE_CONTEXT", "client", "visitorData"][..],
            &["responseContext", "visitorData"][..],
        ] {
            let mut value = Some(*source);
            for key in path {
                value = value.and_then(|value| value.get(*key));
            }
            if let Some(Value::String(text)) = value {
                return Some(text.clone());
            }
        }
    }
    None
}

/// `_extract_context`: the client context with `hl`, `timeZone` and
/// `utcOffsetMinutes` enforced.
fn api_context(ytcfg: &Map<String, Value>, default: &Map<String, Value>) -> Value {
    let mut context = ytcfg
        .get("INNERTUBE_CONTEXT")
        .filter(|context| context.is_object())
        .or_else(|| default.get("INNERTUBE_CONTEXT"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    if let Some(client) = context.get_mut("client").and_then(Value::as_object_mut) {
        client.insert("hl".into(), "en".into());
        client.insert("timeZone".into(), "UTC".into());
        client.insert("utcOffsetMinutes".into(), 0.into());
    }
    context
}

/// `_extract_player_response` through `_call_api`: one POST to
/// `/youtubei/v1/player`.
#[allow(clippy::too_many_arguments)]
fn player_response(
    client: &Client,
    config: &innertube::ClientConfig,
    video_id: &str,
    webpage_ytcfg: &Map<String, Value>,
    player_ytcfg: &Map<String, Value>,
    player_url: Option<&str>,
    visitor: Option<&str>,
) -> Result<Value> {
    let mut query = Map::new();
    query.insert("videoId".into(), video_id.into());
    if let Some(params) = config.player_params {
        query.insert("params".into(), params.into());
    }
    // `_extract_signature_timestamp`: the page's `STS`.
    let sts = player_url.and_then(|_| {
        webpage_ytcfg
            .get("STS")
            .and_then(|sts| sts.as_i64().or_else(|| sts.as_str()?.parse().ok()))
            .filter(|sts| *sts != 0)
    });
    let mut playback = Map::new();
    playback.insert("html5Preference".into(), "HTML5_PREF_WANTS".into());
    if let Some(sts) = sts {
        playback.insert("signatureTimestamp".into(), sts.into());
    }
    query.insert(
        "playbackContext".into(),
        json!({"contentPlaybackContext": Value::Object(playback)}),
    );
    query.insert("contentCheckOk".into(), true.into());
    query.insert("racyCheckOk".into(), true.into());
    let note = format!(
        "Downloading {} player API JSON",
        config.name.replace('_', " ").trim()
    );
    call_api(
        client,
        config,
        "player",
        "youtube",
        video_id,
        player_ytcfg,
        visitor,
        query,
        &note,
    )
}

/// `_call_api`: one POST to `/youtubei/v1/ENDPOINT` with the client's
/// `generate_api_headers`.
#[allow(clippy::too_many_arguments)]
fn call_api(
    client: &Client,
    config: &innertube::ClientConfig,
    endpoint: &str,
    extractor: &'static str,
    item_id: &str,
    player_ytcfg: &Map<String, Value>,
    visitor: Option<&str>,
    query: Map<String, Value>,
    note: &str,
) -> Result<Value> {
    let default = default_ytcfg(config);
    let mut body = Map::new();
    body.insert("context".into(), api_context(player_ytcfg, &default));
    body.extend(query);
    let origin = format!("https://{}", config.host);
    let name_id = ytcfg_value(player_ytcfg, &default, &["INNERTUBE_CONTEXT_CLIENT_NAME"])
        .map(|value| match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    let version = ytcfg_value(player_ytcfg, &default, &["INNERTUBE_CLIENT_VERSION"])
        .or_else(|| {
            ytcfg_value(
                player_ytcfg,
                &default,
                &["INNERTUBE_CONTEXT", "client", "clientVersion"],
            )
        })
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let user_agent = ytcfg_value(
        player_ytcfg,
        &default,
        &["INNERTUBE_CONTEXT", "client", "userAgent"],
    )
    .and_then(Value::as_str)
    .map(str::to_owned);
    let mut headers: Vec<(&str, String)> = vec![
        ("X-YouTube-Client-Name", name_id),
        ("X-YouTube-Client-Version", version),
        ("Origin", origin),
    ];
    if let Some(visitor) = visitor {
        headers.push(("X-Goog-Visitor-Id", visitor.to_owned()));
    }
    if let Some(user_agent) = user_agent {
        headers.push(("User-Agent", user_agent));
    }
    headers.push(("content-type", "application/json".into()));
    let url = Url::parse(&format!(
        "https://{}/youtubei/v1/{endpoint}?prettyPrint=false",
        config.host
    ))?;
    crate::screen::screen(&format!("[{extractor}] {item_id}: {note}"));
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let body = serde_json::to_string(&Value::Object(body))
        .map_err(|error| Error::Reported(error.to_string()))?;
    let response =
        super::super::direct::post(client, &url, &headers, body).map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor,
                id: item_id.to_owned(),
                what: "API page",
                source: Box::new(error),
            },
            error => error,
        })?;
    let text = response.text().map_err(Error::Http)?;
    serde_json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{extractor}] {item_id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// `_download_player_url`: the player version in the iframe API JS.
fn iframe_player_url(client: &Client, video_id: &str) -> Option<String> {
    static VERSION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"player\\?/([0-9a-fA-F]{8})\\?/").unwrap());
    crate::screen::screen(&format!("[youtube] {video_id}: Downloading iframe API JS"));
    let url = Url::parse("https://www.youtube.com/iframe_api").ok()?;
    let page = match super::super::direct::request(client, &url, None)
        .and_then(|response| response.text().map_err(Error::Http))
    {
        Ok(page) => page,
        Err(error) => {
            crate::screen::warning(&format!(
                "[youtube] {video_id}: Unable to download webpage: {}",
                crate::upstream_error_text(&error)
            ));
            return None;
        }
    };
    let Some(found) = VERSION.captures(&page) else {
        super::super::search_warning("youtube", "player version");
        return None;
    };
    Some(format!(
        "https://www.youtube.com/s/player/{}/{MAIN_PLAYER_VARIANT}",
        &found[1]
    ))
}

/// `_get_requested_clients` for an anonymous session.
fn requested_clients(js_available: bool) -> Vec<&'static str> {
    let defaults: &[&'static str] = if js_available {
        &DEFAULT_CLIENTS
    } else {
        &DEFAULT_JSLESS_CLIENTS
    };
    let mut requested: Vec<&'static str> = Vec::new();
    let mut excluded: Vec<String> = Vec::new();
    let arguments = crate::manifest_args::configuration_arg("youtube", "player_client", false)
        .unwrap_or_default();
    let mut allowed: Vec<&'static innertube::ClientConfig> = innertube::CLIENTS
        .iter()
        .filter(|client| !client.name.starts_with('_'))
        .collect();
    allowed.sort_by_key(|client| std::cmp::Reverse(client.priority));
    for argument in &arguments {
        match argument.as_str() {
            "default" => requested.extend(defaults),
            "all" => requested.extend(allowed.iter().map(|client| client.name)),
            name if name.starts_with('-') => excluded.push(name[1..].to_owned()),
            name => match innertube::client(name) {
                Some(client) => requested.push(client.name),
                None => crate::screen::warning(&format!(
                    "[youtube] Skipping unsupported client \"{name}\""
                )),
            },
        }
    }
    if requested.is_empty() && excluded.is_empty() && !js_available {
        crate::screen::warning_once(&format!(
            "[youtube] No supported JavaScript runtime could be found. Only deno is enabled by default; to use another runtime add  --js-runtimes RUNTIME[:PATH]  to your command/config. YouTube extraction without a JS runtime has been deprecated, and some formats may be missing. See  {EJS_WIKI_URL}  for details on installing one"
        ));
    }
    if requested.is_empty() {
        requested.extend(defaults);
    }
    requested.retain(|client| !excluded.iter().any(|excluded| excluded == client));
    let mut ordered = Vec::new();
    for client in requested {
        if !ordered.contains(&client) {
            ordered.push(client);
        }
    }
    ordered
}

/// The acquisition result `_real_extract` continues with.
pub(super) struct Acquired {
    pub webpage: String,
    pub initial_data: Option<Value>,
    pub responses: Vec<streams::ClientResponse>,
    /// Every player response in upstream's order (the stripped initial one
    /// first), for metadata.
    pub player_responses: Vec<Value>,
    pub player_url: Option<String>,
}

/// `_initial_extract` for an anonymous session.
pub(super) fn acquire(
    client: &Client,
    video_id: &str,
    runtime: Option<&challenge::Runtime>,
) -> Result<Acquired> {
    let webpage_url = Url::parse(&format!(
        "https://www.youtube.com/watch?v={video_id}&bpctr=9999999999&has_verified=1"
    ))?;
    let webpage = super::super::download_webpage(client, &webpage_url, "youtube", video_id)?;
    let web = innertube::client(WEBPAGE_CLIENT).expect("web client");
    let mut webpage_ytcfg = extract_ytcfg(&webpage);
    if webpage_ytcfg.is_empty() {
        webpage_ytcfg = default_ytcfg(web);
    }
    static INITIAL_DATA: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?:window\s*\[\s*["']ytInitialData["']\s*\]|ytInitialData)\s*="#).unwrap()
    });
    static INITIAL_PLAYER_RESPONSE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"ytInitialPlayerResponse\s*=").unwrap());
    let initial_data = search_initial(&webpage, &INITIAL_DATA, "yt initial data").filter(|data| {
        data.get("contents")
            .is_some_and(|contents| !contents.is_null())
    });
    let initial_pr = search_initial(
        &webpage,
        &INITIAL_PLAYER_RESPONSE,
        &format!("{WEBPAGE_CLIENT} client initial player response"),
    );
    let clients = requested_clients(runtime.is_some());

    let mut player_responses: Vec<Value> = Vec::new();
    let mut responses: Vec<streams::ClientResponse> = Vec::new();
    let valid = |response: &Value| {
        response
            .get("videoDetails")
            .and_then(|details| details.get("videoId"))
            .and_then(Value::as_str)
            == Some(video_id)
    };
    if let Some(initial) = initial_pr.as_ref().filter(|initial| valid(initial)) {
        let mut stripped = initial.clone();
        if let Some(object) = stripped.as_object_mut() {
            object.insert("streamingData".into(), Value::Null);
        }
        player_responses.push(stripped);
    }
    let mut player_url_found: Option<String> = None;
    let mut tried_iframe_fallback = false;
    let mut visitor: Option<String> = None;
    let mut skipped: Vec<(&'static str, String)> = Vec::new();
    let empty = Map::new();
    let webpage_ytcfg_value = Value::Object(webpage_ytcfg.clone());
    for name in clients {
        let Some(config) = innertube::client(name) else {
            continue;
        };
        let player_ytcfg = if name == WEBPAGE_CLIENT {
            &webpage_ytcfg
        } else {
            &empty
        };
        if player_url_found.is_none() {
            player_url_found = player_url(&[&webpage_ytcfg, player_ytcfg]);
        }
        if player_url_found.is_none() && !tried_iframe_fallback && config.require_js_player {
            player_url_found = iframe_player_url(client, video_id);
            tried_iframe_fallback = true;
        }
        let initial_value = initial_pr.clone().unwrap_or(Value::Null);
        if visitor.is_none() {
            visitor = visitor_data(&[
                &webpage_ytcfg_value,
                &initial_value,
                &Value::Object(player_ytcfg.clone()),
            ]);
        }
        let reused = (name == WEBPAGE_CLIENT)
            .then(|| initial_pr.clone())
            .flatten();
        let response = match reused {
            Some(response) => response,
            None => match player_response(
                client,
                config,
                video_id,
                if player_ytcfg.is_empty() {
                    &webpage_ytcfg
                } else {
                    player_ytcfg
                },
                player_ytcfg,
                player_url_found.as_deref(),
                visitor.as_deref(),
            ) {
                Ok(response) => response,
                Err(error) => {
                    crate::screen::warning(&crate::upstream_error_text(&error));
                    continue;
                }
            },
        };
        let id = response
            .get("videoDetails")
            .and_then(|details| details.get("videoId"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        if id.as_deref() != Some(video_id) {
            skipped.push((name, id.unwrap_or_else(|| "None".into())));
            continue;
        }
        let status = response
            .get("playabilityStatus")
            .and_then(|status| status.get("status"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        player_responses.push(response.clone());
        responses.push(streams::ClientResponse {
            client: config.name,
            response,
            player_token_provided: false,
            available_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs() as i64),
        });
        if !matches!(
            status.as_deref(),
            Some("OK" | "LIVE_STREAM_OFFLINE" | "AGE_CHECK_REQUIRED" | "AGE_VERIFICATION_REQUIRED")
        ) {
            crate::screen::debug(&format!(
                "{video_id}: {name} player response playability status: {}",
                status.as_deref().unwrap_or("None")
            ));
        }
    }
    if !skipped.is_empty() {
        let clients: Vec<&str> = skipped.iter().map(|(client, _)| *client).collect();
        let mut ids: Vec<&str> = skipped.iter().map(|(_, id)| id.as_str()).collect();
        ids.dedup();
        crate::screen::warning(&format!(
            "[youtube] Skipping player responses from {} clients (got player responses for video \"{}\" instead of \"{video_id}\")",
            clients.join("/"),
            ids.join("/")
        ));
        if player_responses.is_empty() {
            return Err(Error::Reported(format!(
                "[youtube] {video_id}: All player responses are invalid. Your IP is likely being blocked by Youtube"
            )));
        }
    } else if player_responses.is_empty() {
        return Err(Error::Reported(format!(
            "[youtube] {video_id}: Failed to extract any player response; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    }
    Ok(Acquired {
        webpage,
        initial_data,
        responses,
        player_responses,
        player_url: player_url_found,
    })
}

/// A single direct player call on the existing account/proxy-scoped HTTP client.
/// VisionOS does not need a watch page, signature timestamp or player JavaScript.
pub(super) fn acquire_direct(client: &Client, video_id: &str) -> Result<Acquired> {
    let config = innertube::client("visionos").expect("visionos client");
    let empty = Map::new();
    let response = player_response(client, config, video_id, &empty, &empty, None, None)?;
    direct_response(video_id, response)
}

fn direct_response(video_id: &str, response: Value) -> Result<Acquired> {
    if response
        .pointer("/videoDetails/videoId")
        .and_then(Value::as_str)
        != Some(video_id)
        || response
            .pointer("/playabilityStatus/status")
            .and_then(Value::as_str)
            != Some("OK")
        || !response.get("streamingData").is_some_and(Value::is_object)
    {
        return Err(super::unsupported(
            "direct player response requires full acquisition",
        ));
    }
    Ok(Acquired {
        webpage: String::new(),
        initial_data: None,
        player_responses: vec![response.clone()],
        responses: vec![streams::ClientResponse {
            client: "visionos",
            response,
            player_token_provided: false,
            available_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_secs() as i64),
        }],
        player_url: None,
    })
}

pub(super) fn rate_limited(error: &Error) -> bool {
    match error {
        Error::Http(source) | Error::HttpRetryAfter { source, .. } => {
            source.status() == Some(reqwest::StatusCode::TOO_MANY_REQUESTS)
        }
        Error::ExtractorRequest { source, .. } | Error::Webpage { source, .. } => {
            rate_limited(source)
        }
        _ => false,
    }
}

#[cfg(test)]
mod direct_tests {
    use super::*;
    #[test]
    fn direct_response_rejects_wrong_id_restrictions_and_missing_streams() {
        let valid = json!({"videoDetails":{"videoId":"BaW_jenozKc"},
            "playabilityStatus":{"status":"OK"},"streamingData":{}});
        assert!(direct_response("BaW_jenozKc", valid.clone()).is_ok());
        assert!(direct_response("ABCDEFGHIJK", valid.clone()).is_err());
        for status in [
            "LOGIN_REQUIRED",
            "UNPLAYABLE",
            "ERROR",
            "LIVE_STREAM_OFFLINE",
        ] {
            let mut response = valid.clone();
            response["playabilityStatus"]["status"] = status.into();
            assert!(direct_response("BaW_jenozKc", response).is_err());
        }
        let mut response = valid;
        response["streamingData"] = Value::Null;
        assert!(direct_response("BaW_jenozKc", response).is_err());
    }
}

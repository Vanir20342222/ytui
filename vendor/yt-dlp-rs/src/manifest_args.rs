//! GenericIE's manifest extractor arguments (`--extractor-args
//! "generic:fragment_query=...;variant_query=...;key_query=...;hls_key=...;is_live=..."`),
//! upstream `_extra_manifest_info`, and how HlsFD/DashSegmentsFD apply the
//! resulting `extra_param_to_segment_url`, `extra_param_to_key_url` and
//! `hls_aes` at download time.
use crate::{Error, Fragment, Result, VideoInfo};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{LazyLock, Mutex},
};
use url::Url;

/// Parsed `--extractor-args`: per lowercased extractor key, argument lists.
pub type ExtractorArgs = BTreeMap<String, BTreeMap<String, Vec<String>>>;

static EXTRACTOR_ARGS: LazyLock<Mutex<ExtractorArgs>> = LazyLock::new(Default::default);

/// Set the parsed `--extractor-args` (see `parse_extractor_args`).
pub fn set_extractor_args(args: ExtractorArgs) {
    *EXTRACTOR_ARGS.lock().unwrap() = args;
}

/// Upstream `_configuration_arg(key, ie_key=...)` (lowercased unless
/// `casesense`).
pub(crate) fn configuration_arg(ie_key: &str, key: &str, casesense: bool) -> Option<Vec<String>> {
    let args = EXTRACTOR_ARGS.lock().unwrap();
    let values = args.get(&ie_key.to_lowercase())?.get(key)?;
    Some(if casesense {
        values.clone()
    } else {
        values.iter().map(|value| value.to_lowercase()).collect()
    })
}

/// Upstream `--extractor-args IE_KEY:ARGS` (`_dict_from_options_callback`
/// with `_extractor_arg_parser`): a later value for the same extractor
/// replaces its arguments.
pub fn parse_extractor_args(values: &[String]) -> std::result::Result<ExtractorArgs, String> {
    static OPTION: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?is)^(?P<key>[\w-]+):(?P<val>.*)$").unwrap());
    let mut parsed = BTreeMap::new();
    for value in values {
        let Some(found) = OPTION.captures(value) else {
            return Err(format!(
                "wrong --extractor-args formatting; it should be IE_KEY:ARGS, not \"{value}\""
            ));
        };
        let mut arguments = BTreeMap::new();
        for argument in found["val"].split(';') {
            let (key, values) = argument.split_once('=').unwrap_or((argument, ""));
            let key = key.trim().to_lowercase().replace('-', "_");
            // `re.split(r'(?<!\\),', vals)`, unescaping `\,`.
            let mut items = Vec::new();
            let mut current = String::new();
            let mut previous = None;
            for c in values.chars() {
                if c == ',' && previous != Some('\\') {
                    items.push(std::mem::take(&mut current));
                } else {
                    current.push(c);
                }
                previous = Some(c);
            }
            items.push(current);
            let items = items
                .into_iter()
                .map(|item| item.replace("\\,", ",").trim().to_owned())
                .collect();
            arguments.insert(key, items);
        }
        parsed.insert(found["key"].to_lowercase(), arguments);
    }
    Ok(parsed)
}

/// Python `urllib.parse.parse_qs` (blank values dropped).
fn parse_qs(text: &str) -> Vec<(String, Vec<String>)> {
    let mut query: Vec<(String, Vec<String>)> = Vec::new();
    for (key, value) in url::form_urlencoded::parse(text.as_bytes()) {
        if value.is_empty() {
            continue;
        }
        match query.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, values)) => values.push(value.into_owned()),
            None => query.push((key.into_owned(), vec![value.into_owned()])),
        }
    }
    query
}

/// Upstream `update_url_query(url, query)`: the parsed query updated with
/// `query`, re-encoded like `urlencode(..., True)`.
fn update_url_query(url: &Url, extra: &[(String, Vec<String>)]) -> Url {
    let mut query = parse_qs(url.query().unwrap_or(""));
    for (key, values) in extra {
        match query.iter_mut().find(|(existing, _)| existing == key) {
            Some((_, existing)) => *existing = values.clone(),
            None => query.push((key.clone(), values.clone())),
        }
    }
    let mut encoded = url::form_urlencoded::Serializer::new(String::new());
    for (key, values) in &query {
        for value in values {
            encoded.append_pair(key, value);
        }
    }
    let mut updated = url.clone();
    let encoded = encoded.finish();
    updated.set_query((!encoded.is_empty()).then_some(encoded.as_str()));
    updated
}

/// `urllib.parse.urlparse(value).query or value`.
fn query_of(value: &str) -> String {
    Url::parse(value)
        .ok()
        .and_then(|url| url.query().map(str::to_owned))
        .filter(|query| !query.is_empty())
        .unwrap_or_else(|| value.to_owned())
}

/// Upstream `GenericIE._extra_manifest_info`: the query and key arguments
/// as info keys, `variant_query` on every format URL; returns the
/// `is_live` argument (which replaces the live-status check).
pub(crate) fn extra_manifest_info(info: &mut VideoInfo, manifest_url: &Url) -> Option<String> {
    let manifest_query = manifest_url.query().filter(|query| !query.is_empty());
    for (argument, key) in [
        ("fragment_query", "extra_param_to_segment_url"),
        ("key_query", "extra_param_to_key_url"),
    ] {
        if let Some(value) = configuration_arg("generic", argument, true)
            .and_then(|values| values.into_iter().next())
        {
            let value = Some(query_of(&value))
                .filter(|value| !value.is_empty())
                .or_else(|| manifest_query.map(str::to_owned));
            info.extra
                .insert(key.into(), value.map_or(Value::Null, Value::from));
        }
    }
    // `hls_key`: `URI_OR_HEX_KEY[,HEX_IV]`.
    let hex = |value: &str| {
        let digits = value
            .strip_prefix("0x")
            .or_else(|| value.strip_prefix("0X"))
            .unwrap_or(value);
        (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| value.to_owned())
    };
    let hls_key = configuration_arg("generic", "hls_key", true).unwrap_or_default();
    let mut aes = serde_json::Map::new();
    if let Some(first) = hls_key.first() {
        if Url::parse(first).is_ok_and(|url| matches!(url.scheme(), "http" | "https")) {
            aes.insert("uri".into(), first.clone().into());
        }
        if let Some(key) = hex(first) {
            aes.insert("key".into(), key.into());
        }
    }
    if let Some(iv) = hls_key.get(1).and_then(|iv| hex(iv)) {
        aes.insert("iv".into(), iv.into());
    }
    if !aes.is_empty() {
        info.extra.insert("hls_aes".into(), Value::Object(aes));
    }
    if let Some(variant) = configuration_arg("generic", "variant_query", true)
        .and_then(|values| values.into_iter().next())
    {
        let text = Some(query_of(&variant))
            .filter(|value| !value.is_empty())
            .or_else(|| manifest_query.map(str::to_owned))
            .unwrap_or_default();
        let query = parse_qs(&text);
        for format in &mut info.formats {
            format.url = update_url_query(&format.url, &query);
        }
        if let Some(first) = info.formats.first() {
            info.url = first.url.clone();
        }
    }
    configuration_arg("generic", "is_live", false).and_then(|values| values.into_iter().next())
}

/// An external `hls_aes` key (16 bytes for AES-128).
pub(crate) fn external_key(info: &VideoInfo) -> Result<Option<[u8; 16]>> {
    let Some(key) = info
        .extra
        .get("hls_aes")
        .and_then(|aes| aes.get("key"))
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let digits = key.strip_prefix("0x").unwrap_or(key);
    let bytes = hex_bytes(digits)
        .ok_or_else(|| Error::UnsupportedManifest("invalid HLS AES-128 key".into()))?;
    <[u8; 16]>::try_from(bytes.as_slice())
        .map(Some)
        .map_err(|_| Error::UnsupportedManifest("Invalid length for HLS AES-128 key".into()))
}

fn hex_bytes(digits: &str) -> Option<Vec<u8>> {
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    (0..digits.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&digits[index..index + 2], 16).ok())
        .collect()
}

/// The fragments as HlsFD/DashSegmentsFD request them: the segment query on
/// every fragment, the key query (else the segment query) on key URLs, and
/// `hls_aes`'s URI and IV; `None` when nothing changes.
pub(crate) fn download_fragments(info: &VideoInfo) -> Option<Vec<Fragment>> {
    let text = |key: &str| info.extra.get(key).and_then(Value::as_str).map(parse_qs);
    let segment = text("extra_param_to_segment_url").filter(|query| !query.is_empty());
    let key = text("extra_param_to_key_url").filter(|query| !query.is_empty());
    let aes = info.extra.get("hls_aes");
    let uri = aes
        .and_then(|aes| aes.get("uri"))
        .and_then(Value::as_str)
        .and_then(|uri| Url::parse(uri).ok());
    let iv = aes
        .and_then(|aes| aes.get("iv"))
        .and_then(Value::as_str)
        .and_then(|iv| {
            let digits = iv.strip_prefix("0x").unwrap_or(iv);
            hex_bytes(&format!("{digits:0>32}"))
        })
        .and_then(|bytes| <[u8; 16]>::try_from(bytes.as_slice()).ok());
    if segment.is_none() && key.is_none() && uri.is_none() && iv.is_none() {
        return None;
    }
    let hls = info
        .protocol
        .as_deref()
        .is_some_and(|protocol| protocol.starts_with("m3u8"));
    Some(
        info.fragments
            .iter()
            .map(|fragment| {
                let mut fragment = fragment.clone();
                if let Some(segment) = &segment {
                    fragment.url = update_url_query(&fragment.url, segment);
                }
                if hls && let Some(encryption) = &mut fragment.encryption {
                    if let Some(iv) = iv {
                        encryption.iv = iv;
                    }
                    if let Some(query) = key.as_ref().or(segment.as_ref()) {
                        encryption.key_url = update_url_query(&encryption.key_url, query);
                    }
                    if let Some(uri) = &uri {
                        encryption.key_url = uri.clone();
                    }
                }
                fragment
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extractor_args_and_queries_follow_upstream() {
        let parsed = parse_extractor_args(&[
            "Generic:fragment_query=a=1;Variant-Query= x ,y\\,z".into(),
            "youtube:player_client=web".into(),
        ])
        .unwrap();
        assert_eq!(parsed["generic"]["fragment_query"], ["a=1"]);
        assert_eq!(parsed["generic"]["variant_query"], ["x", "y,z"]);
        assert!(parse_extractor_args(&["noargs".into()]).is_err());
        let url = Url::parse("https://e.invalid/s.ts?a=0&b=2").unwrap();
        assert_eq!(
            update_url_query(&url, &parse_qs("a=1&c=3")).as_str(),
            "https://e.invalid/s.ts?a=1&b=2&c=3"
        );
        assert_eq!(query_of("https://e.invalid/?t=1"), "t=1");
        assert_eq!(query_of("t=1"), "t=1");
    }
}

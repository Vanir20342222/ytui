//! GenericIE's KVS player step: the `kt_player` version, `_extract_kvs`'
//! flashvars, and `_kvs_get_real_url`'s license-token hash unscrambling.
use regex::Regex;
use serde_json::{Map, Value};
use std::sync::LazyLock;

/// The `kt_player.js`/`kt_player.swf` version (`group='ver'`), trying the
/// script pattern before the SWF call like `_search_regex` with a tuple.
pub(crate) fn player_version(html: &str) -> Option<String> {
    static SCRIPT: LazyLock<Regex> = LazyLock::new(|| {
        let quoted = |q: &str| {
            format!(
                r#"{q}https?://[^?#{q}]+/kt_player\.js\?v=(?P<v{}>\d+(?:\.\d+)+){q}"#,
                if q == "\"" { 1 } else { 2 }
            )
        };
        Regex::new(&format!(
            r#"<script\b[^>]+?\bsrc\s*=\s*(?:{}|{})[^>]*>"#,
            quoted("\""),
            quoted("'")
        ))
        .expect("static pattern")
    });
    static SWF: LazyLock<Regex> = LazyLock::new(|| {
        let mut alternatives = Vec::new();
        for (index, (first, second)) in [("\"", "\""), ("\"", "'"), ("'", "\""), ("'", "'")]
            .into_iter()
            .enumerate()
        {
            alternatives.push(format!(
                r#"{first}[^{first}]+{first}\s*,\s*{second}https?://[^?#{second}]+/kt_player\.swf\?v=(?P<v{index}>\d+(?:\.\d+)+){second}\s*,"#
            ));
        }
        Regex::new(&format!(r"kt_player\s*\(\s*(?:{})", alternatives.join("|")))
            .expect("static pattern")
    });
    for regex in [&*SCRIPT, &*SWF] {
        if let Some(found) = regex.captures(html) {
            return found
                .iter()
                .skip(1)
                .flatten()
                .next()
                .map(|version| version.as_str().to_owned());
        }
    }
    None
}

/// `_search_json(r'(?s:<script\b[^>]*>.*?var\s+flashvars\s*=)', webpage,
/// 'flashvars', transform_source=js_to_json)`: the greedy `{...}` after the
/// start, converted whole, and its first JSON value (`ignore_extra=True`).
pub(crate) fn flashvars(html: &str) -> Option<Option<Map<String, Value>>> {
    static START: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?s:<script\b[^>]*>.*?var\s+flashvars\s*=)\s*(?s)(\{.+\})")
            .expect("static pattern")
    });
    let found = START.captures(html)?;
    let converted = crate::js_to_json::js_to_json(&found[1]);
    let value = serde_json::Deserializer::from_str(&converted)
        .into_iter::<Value>()
        .next()
        .and_then(Result::ok);
    Some(match value {
        Some(Value::Object(map)) => Some(map),
        _ => None,
    })
}

/// `_kvs_get_license_token(license_code)`; `None` where upstream raises.
fn license_token(code: &str) -> Option<Vec<u32>> {
    let code = code.replace('$', "");
    let values: Vec<u32> = code
        .chars()
        .map(|c| c.to_digit(10))
        .collect::<Option<_>>()?;
    let modified = code.replace('0', "1");
    let center = modified.len() / 2;
    let front: i128 = modified.get(..center + 1)?.parse().ok()?;
    let back: i128 = modified.get(center..)?.parse().ok()?;
    let digits = (4 * (front - back).abs()).to_string();
    let digits: Vec<u32> = digits
        .chars()
        .take(center + 1)
        .filter_map(|c| c.to_digit(10))
        .collect();
    let mut token = Vec::new();
    for (index, current) in digits.into_iter().enumerate() {
        for offset in 0..4 {
            token.push((values.get(index + offset)? + current) % 10);
        }
    }
    Some(token)
}

/// `_kvs_get_real_url(video_url, license_code)`; `None` where upstream
/// raises.
pub(crate) fn real_url(video_url: &str, license_code: &str) -> Option<String> {
    let Some(rest) = video_url.strip_prefix("function/0/") else {
        return Some(video_url.to_owned());
    };
    let token = license_token(license_code)?;
    // `urlparse`: the path between the netloc and `;`/`?`/`#`.
    let (scheme_netloc, path_and_rest) = match rest.find("://") {
        Some(index) => {
            let after = index + 3;
            let path_start = rest[after..]
                .find(['/', '?', '#'])
                .map_or(rest.len(), |offset| after + offset);
            rest.split_at(path_start)
        }
        None => ("", rest),
    };
    let path_end = path_and_rest
        .find(['?', '#', ';'])
        .unwrap_or(path_and_rest.len());
    let (path, tail) = path_and_rest.split_at(path_end);
    let mut parts: Vec<String> = path.split('/').map(str::to_owned).collect();
    const HASH_LENGTH: usize = 32;
    let part = parts.get(3)?;
    let hash: Vec<char> = part.chars().take(HASH_LENGTH).collect();
    if token.len() < HASH_LENGTH || hash.len() < HASH_LENGTH {
        return None;
    }
    let mut indices: Vec<usize> = (0..HASH_LENGTH).collect();
    let mut accum = 0usize;
    for src in (0..HASH_LENGTH).rev() {
        accum += token[src] as usize;
        let dest = (src + accum) % HASH_LENGTH;
        indices.swap(src, dest);
    }
    let unscrambled: String = indices.iter().map(|&index| hash[index]).collect();
    let remainder: String = part.chars().skip(HASH_LENGTH).collect();
    parts[3] = unscrambled + &remainder;
    Some(format!("{scheme_netloc}{}{tail}", parts.join("/")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_urls_match_pinned_upstream() {
        // `GenericIE._kvs_get_real_url` of the pinned upstream.
        assert_eq!(
            real_url(
                "function/0/https://www.example.test/get_file/1/0123456789abcdef0123456789abcdefextra/10000/10000_720p.mp4/?br=900",
                "$535155614031502"
            )
            .as_deref(),
            Some(
                "https://www.example.test/get_file/1/b9d4545ef0f23690821a8cecd767ab31extra/10000/10000_720p.mp4/?br=900"
            )
        );
        assert_eq!(
            real_url("https://h/x.mp4", "$1").as_deref(),
            Some("https://h/x.mp4")
        );
    }

    #[test]
    fn versions_follow_upstream_patterns() {
        assert_eq!(
            player_version(
                r#"<script type="text/javascript" src="https://h/p/kt_player.js?v=6.2.0"></script>"#
            )
            .as_deref(),
            Some("6.2.0")
        );
        assert_eq!(
            player_version("kt_player('kt_player', 'https://h/kt_player.swf?v=3.1.2', '100%');")
                .as_deref(),
            Some("3.1.2")
        );
        assert!(player_version("<script src='https://h/kt_player.js'></script>").is_none());
    }
}

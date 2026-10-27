//! Proxy policy shared by normal and bounded player HTTP clients.
use crate::{Error, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use percent_encoding::percent_decode_str;
use reqwest::blocking::ClientBuilder;
use url::Url;

#[derive(Clone, Default)]
pub(crate) enum ProxyConfig {
    #[default]
    Environment,
    Direct,
    Explicit {
        url: Url,
        credentials: Option<(String, Option<String>)>,
    },
}

fn error() -> Error {
    // Proxy URLs can contain credentials. Never include input or parser errors.
    Error::HttpConfiguration("invalid or unsupported proxy URL".into())
}

impl ProxyConfig {
    pub(crate) fn parse(raw: Option<&str>) -> Result<Self> {
        let Some(raw) = raw else {
            return Ok(Self::Environment);
        };
        if matches!(raw, "" | "__noproxy__") {
            return Ok(Self::Direct);
        }
        if raw.len() > 16 * 1024
            || raw.chars().any(|c| c.is_whitespace() || c.is_control())
            || raw.contains('\\')
        {
            return Err(error());
        }
        let normalized;
        let raw = if raw.contains("://") {
            raw
        } else {
            normalized = format!("http://{}", raw.strip_prefix("//").unwrap_or(raw));
            &normalized
        };
        let mut url = Url::parse(raw).map_err(|_| error())?;
        let authority = raw
            .split_once("://")
            .map_or("", |(_, tail)| tail)
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("");
        let credentials = authority
            .rsplit_once('@')
            .map(|(userinfo, _)| {
                let username = percent_decode_str(url.username())
                    .decode_utf8()
                    .map_err(|_| error())?
                    .into_owned();
                let password = if userinfo.contains(':') {
                    Some(
                        percent_decode_str(url.password().unwrap_or(""))
                            .decode_utf8()
                            .map_err(|_| error())?
                            .into_owned(),
                    )
                } else {
                    None
                };
                Ok::<_, Error>((username, password))
            })
            .transpose()?;
        match url.scheme() {
            "http" | "https" | "socks4" | "socks4a" | "socks5h" => {}
            "socks" => {
                url.set_scheme("socks4").map_err(|_| error())?;
            }
            // Pinned yt-dlp preserves its historical remote-DNS SOCKS5 behavior.
            "socks5" => {
                url.set_scheme("socks5h").map_err(|_| error())?;
            }
            _ => return Err(error()),
        }
        if url.host_str().is_none()
            || url.port() == Some(0)
            || !matches!(url.path(), "" | "/")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(error());
        }
        for value in std::iter::once(url.username()).chain(url.password()) {
            let bytes = value.as_bytes();
            for (index, &byte) in bytes.iter().enumerate() {
                if byte == b'%'
                    && (index + 2 >= bytes.len()
                        || !bytes[index + 1].is_ascii_hexdigit()
                        || !bytes[index + 2].is_ascii_hexdigit())
                {
                    return Err(error());
                }
            }
            let decoded = percent_decode_str(value)
                .decode_utf8()
                .map_err(|_| error())?;
            if decoded.chars().any(char::is_control) {
                return Err(error());
            }
            if url.scheme().starts_with("socks") && decoded.len() > 255 {
                return Err(error());
            }
        }
        if matches!(url.scheme(), "socks4" | "socks4a") && credentials.is_some() {
            // The current transport cannot carry SOCKS4 USERID. Do not silently
            // discard credentials that upstream would send.
            return Err(error());
        }
        // Explicit SOCKS ports avoid depending on backend-specific URL defaults.
        if url.scheme().starts_with("socks") && url.port().is_none() {
            url.set_port(Some(1080)).map_err(|_| error())?;
        }
        reqwest::Proxy::all(url.clone()).map_err(|_| error())?;
        Ok(Self::Explicit { url, credentials })
    }

    pub(crate) fn apply(&self, builder: ClientBuilder) -> Result<ClientBuilder> {
        match self {
            Self::Environment => Ok(builder),
            Self::Direct => Ok(builder.no_proxy()),
            Self::Explicit { url, credentials } => {
                let mut endpoint = url.clone();
                let http = matches!(url.scheme(), "http" | "https");
                let incomplete_socks_auth = url.scheme() == "socks5h"
                    && credentials.as_ref().is_none_or(|(username, password)| {
                        username.is_empty() || password.as_deref().is_none_or(str::is_empty)
                    });
                if http || incomplete_socks_auth {
                    endpoint.set_username("").map_err(|_| error())?;
                    endpoint.set_password(None).map_err(|_| error())?;
                }
                let mut proxy = reqwest::Proxy::all(endpoint).map_err(|_| error())?;
                if http && let Some((username, password)) = credentials {
                    let encoded = STANDARD
                        .encode(format!("{username}:{}", password.as_deref().unwrap_or("")));
                    let mut value =
                        reqwest::header::HeaderValue::from_str(&format!("Basic {encoded}"))
                            .map_err(|_| error())?;
                    value.set_sensitive(true);
                    proxy = proxy.custom_http_auth(value);
                }
                // Explicit configuration overrides ambient proxies and NO_PROXY.
                Ok(builder.no_proxy().proxy(proxy))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_explicit_proxy_endpoints_match_without_using_ambient_environment() {
        use serde_json::{Value, json};
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/proxies/cases.json")).unwrap();
        let mut compared = 0;
        for case in fixture["cases"].as_array().unwrap() {
            let raw = case["expected"]["proxy_option"].as_str();
            if raw.is_none() {
                assert!(matches!(
                    ProxyConfig::parse(None).unwrap(),
                    ProxyConfig::Environment
                ));
                continue;
            }
            if case["name"].as_str().unwrap().starts_with("socks4-auth") {
                assert!(ProxyConfig::parse(raw).is_err());
                continue;
            }
            let actual = match ProxyConfig::parse(raw).unwrap() {
                ProxyConfig::Direct => Value::Null,
                ProxyConfig::Explicit { url, credentials } => {
                    let username = credentials.as_ref().map(|(username, _)| username);
                    let password = credentials
                        .as_ref()
                        .and_then(|(_, password)| password.as_ref());
                    let host = url
                        .host_str()
                        .unwrap()
                        .trim_start_matches('[')
                        .trim_end_matches(']');
                    json!({"scheme": url.scheme(), "host": host, "port": url.port_or_known_default().unwrap(), "username": username, "password": password, "rdns": match url.scheme() { "socks4" => Some(false), "socks4a" | "socks5h" => Some(true), _ => None }})
                }
                ProxyConfig::Environment => panic!("explicit proxy became environment"),
            };
            for expected in case["expected"]["selected"].as_array().unwrap() {
                assert_eq!(&actual, expected, "{}", case["name"]);
            }
            compared += 1;
        }
        assert_eq!(compared, 39);
    }

    #[test]
    fn malformed_proxy_values_fail_without_exposing_credentials() {
        for raw in [
            "ftp://user:secret@example.test",
            "http://user:secret@example.test/path",
            "http://user:secret@example.test?query",
            "http://user:secret@example.test/#fragment",
            "http://user:secret@example.test:0",
            "http://user:secret@",
            "http://user:secret%XX@example.test",
            "http://user:secret%0A@example.test",
            "http://user:secret%FF@example.test",
            " http://user:secret@example.test",
            "http://user:secret@exam\\ple.test",
        ] {
            let message = ProxyConfig::parse(Some(raw)).err().unwrap().to_string();
            assert!(!message.contains("secret"));
            assert!(!message.contains("user"));
        }
        assert!(ProxyConfig::parse(Some(&"x".repeat(16 * 1024 + 1))).is_err());
    }
}

#[cfg(test)]
mod tls;

//! Validated global HTTP defaults, separate from extractor-owned request headers.
mod proxy;
use crate::{CookieJar, Error, Result};
pub(crate) use proxy::ProxyConfig;
use reqwest::blocking::ClientBuilder;
use reqwest::header::{COOKIE, HeaderMap, HeaderName, HeaderValue, REFERER, USER_AGENT};
use std::sync::Arc;
use std::{net::IpAddr, time::Duration};

/// Global request defaults. Extractor/transport headers take precedence.
/// Raw Cookie headers are unsupported; use the scoped cookie jar instead.
#[derive(Clone, Default)]
pub struct HttpOptions {
    /// None inherits system routing; an empty string forces direct connections.
    pub proxy: Option<String>,
    /// Shared cookie state; None creates a fresh in-memory jar.
    pub cookies: Option<Arc<CookieJar>>,
    pub headers: Vec<(String, String)>,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
    /// `--socket-timeout` seconds for connect and each read/write; None or 0
    /// keeps the native defaults (15 s connect, 300 s operations).
    pub socket_timeout: Option<f64>,
    /// `--no-check-certificates`: accept invalid HTTPS certificates.
    pub no_check_certificates: bool,
    /// `--source-address`/`-4`/`-6`: client-side IP address to bind to.
    pub source_address: Option<String>,
    /// `--client-certificate` (PEM, optionally including the key),
    /// `--client-certificate-key` and `--client-certificate-password`.
    pub client_certificate: Option<std::path::PathBuf>,
    pub client_certificate_key: Option<std::path::PathBuf>,
    pub client_certificate_password: Option<String>,
    /// `--print-traffic`: report the HTTP bytes sent and response headers.
    pub print_traffic: bool,
}

/// Connection policy shared by the normal and bounded player clients.
#[derive(Clone, Default)]
pub(crate) struct TransportConfig {
    pub(crate) proxy: ProxyConfig,
    timeout: Option<Duration>,
    insecure: bool,
    source_address: Option<IpAddr>,
    identity: Option<reqwest::Identity>,
}

impl From<ProxyConfig> for TransportConfig {
    fn from(proxy: ProxyConfig) -> Self {
        Self {
            proxy,
            ..Default::default()
        }
    }
}

impl TransportConfig {
    pub(crate) fn apply(&self, builder: ClientBuilder) -> Result<ClientBuilder> {
        let (connect, operation) = match self.timeout {
            Some(timeout) => (timeout, timeout),
            None => (Duration::from_secs(15), Duration::from_secs(300)),
        };
        let mut builder = self
            .proxy
            .apply(builder)?
            .connect_timeout(connect)
            .timeout(operation);
        if self.insecure {
            builder = builder.danger_accept_invalid_certs(true);
        }
        if let Some(address) = self.source_address {
            builder = builder.local_address(address);
        }
        if let Some(identity) = &self.identity {
            builder = builder.identity(identity.clone());
        }
        Ok(builder)
    }
}

fn error(message: &str) -> Error {
    // Never include supplied values: headers can contain credentials.
    Error::HttpConfiguration(message.into())
}

impl HttpOptions {
    /// Parse repeated FIELD:VALUE options; split only the first colon.
    /// CLI names use the pinned option callback's ASCII word/hyphen subset.
    pub fn from_cli(
        user_agent: Option<String>,
        referer: Option<String>,
        headers: &[String],
    ) -> Result<Self> {
        if headers.len() > 256 {
            return Err(error("more than 256 custom headers"));
        }
        let mut options = Self {
            headers: Vec::with_capacity(headers.len()),
            user_agent,
            referer,
            ..Default::default()
        };
        for raw in headers {
            let (name, value) = raw
                .split_once(':')
                .ok_or_else(|| error("expected FIELD:VALUE in --add-headers"))?;
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
            {
                return Err(error("invalid field name in --add-headers"));
            }
            options.headers.push((name.into(), value.into()));
        }
        // Validate before any client/network/filesystem operation.
        options.header_map()?;
        Ok(options)
    }

    /// Validate request configuration without network requests or DNS resolution.
    pub fn validate(&self) -> Result<()> {
        self.header_map()?;
        self.transport_config()?;
        Ok(())
    }

    /// Upstream `client_certificate*`: the certificate file (which may hold
    /// the key) plus an optional separate key file, as a TLS client identity.
    fn client_identity(&self) -> Result<Option<reqwest::Identity>> {
        let Some(certificate) = &self.client_certificate else {
            if self.client_certificate_key.is_some() || self.client_certificate_password.is_some() {
                return Err(error(
                    "--client-certificate-key and --client-certificate-password need --client-certificate",
                ));
            }
            return Ok(None);
        };
        let read = |path: &std::path::Path| {
            std::fs::read(path)
                .map_err(|_| error("unable to read the client certificate or key file"))
        };
        let mut pem = read(certificate)?;
        if let Some(key) = &self.client_certificate_key {
            pem.push(b'\n');
            pem.extend(read(key)?);
        }
        if pem
            .windows(b"ENCRYPTED PRIVATE KEY".len())
            .any(|window| window == b"ENCRYPTED PRIVATE KEY")
            || self.client_certificate_password.is_some()
        {
            return Err(error(
                "encrypted client certificate keys (--client-certificate-password) are not supported natively",
            ));
        }
        reqwest::Identity::from_pem(&pem).map(Some).map_err(|_| {
            error("the client certificate must be PEM with a certificate and its private key")
        })
    }

    pub(crate) fn transport_config(&self) -> Result<TransportConfig> {
        let timeout = match self.socket_timeout {
            // Upstream treats 0 as unset (`socket_timeout or 20`).
            None | Some(0.0) => None,
            Some(seconds) if seconds.is_finite() && seconds > 0.0 && seconds <= 86_400.0 => {
                Some(Duration::from_secs_f64(seconds))
            }
            Some(_) => return Err(error("socket timeout must be a positive number of seconds")),
        };
        let source_address = self
            .source_address
            .as_deref()
            .map(|address| {
                address
                    .parse::<IpAddr>()
                    .map_err(|_| error("source address must be an IP address"))
            })
            .transpose()?;
        Ok(TransportConfig {
            proxy: ProxyConfig::parse(self.proxy.as_deref())?,
            timeout,
            insecure: self.no_check_certificates,
            source_address,
            identity: self.client_identity()?,
        })
    }

    pub(crate) fn header_map(&self) -> Result<HeaderMap> {
        if self.headers.len() > 256 {
            return Err(error("more than 256 custom headers"));
        }
        let mut headers = HeaderMap::new();
        let mut total = 0usize;
        let mut insert = |name: HeaderName, raw: &str| -> Result<()> {
            total = total
                .saturating_add(name.as_str().len())
                .saturating_add(raw.len());
            if raw.len() > 16 * 1024 || total > 64 * 1024 {
                return Err(error("header values exceed 16 KiB each or 64 KiB total"));
            }
            // Check before trimming: outer CR/LF must not hide injection attempts.
            if raw.chars().any(|c| c.is_control() && c != '\t') {
                return Err(error("header value contains unsupported controls"));
            }
            if matches!(name.as_str(), "ytdl-request-proxy" | "ytdl-socks-proxy") {
                return Err(error(
                    "legacy per-request proxy headers are unsupported; use --proxy",
                ));
            }
            if name == COOKIE {
                return Err(error(
                    "raw Cookie headers are unsupported; use a scoped cookie jar",
                ));
            }
            let mut value = HeaderValue::from_str(raw.trim())
                .map_err(|_| error("invalid HTTP header value"))?;
            if matches!(name.as_str(), "authorization" | "proxy-authorization") {
                value.set_sensitive(true);
            }
            headers.insert(name, value);
            Ok(())
        };
        for (name, value) in &self.headers {
            if name.len() > 256 {
                return Err(error("HTTP header name exceeds 256 bytes"));
            }
            let name = HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| error("invalid HTTP header name"))?;
            insert(name, value)?;
        }
        // Pinned CLI normalization makes convenience values win independently
        // of argument order; preserve that result for the library API too.
        if let Some(value) = &self.user_agent {
            insert(USER_AGENT, value)?;
        }
        if let Some(value) = &self.referer {
            insert(REFERER, value)?;
        }
        Ok(headers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_controls_cookies_and_resource_bounds_fail_without_values_in_errors() {
        for (name, value) in [
            ("bad name", "secret"),
            ("Cookie", "secret"),
            ("Ytdl-Request-Proxy", "secret"),
            ("Ytdl-Socks-Proxy", "secret"),
            ("x-key", "secret\r\nInjected: yes"),
            ("x-key", "secret\0"),
        ] {
            let options = HttpOptions {
                headers: vec![(name.into(), value.into())],
                ..Default::default()
            };
            let message = options.header_map().unwrap_err().to_string();
            assert!(!message.contains("secret"));
        }
        for options in [
            HttpOptions {
                headers: vec![("x-key".into(), "a".repeat(16 * 1024 + 1))],
                ..Default::default()
            },
            HttpOptions {
                headers: vec![("x-key".into(), "a".repeat(16 * 1024)); 5],
                ..Default::default()
            },
            HttpOptions {
                headers: vec![("x-key".into(), "a".into()); 257],
                ..Default::default()
            },
        ] {
            assert!(options.header_map().is_err());
        }
    }

    #[test]
    fn library_accepts_http_token_names_and_cli_rejects_unsupported_callback_names() {
        let options = HttpOptions {
            headers: vec![("x!key".into(), "\tvalue\twith\ttabs\t".into())],
            ..Default::default()
        };
        assert_eq!(options.header_map().unwrap()["x!key"], "value\twith\ttabs");
        for raw in ["x!key:v", "bad name:v", "missing", ":value"] {
            assert!(HttpOptions::from_cli(None, None, &[raw.into()]).is_err());
        }
    }
}

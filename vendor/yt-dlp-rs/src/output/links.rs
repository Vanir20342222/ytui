//! Upstream internet shortcut files (`--write-link` and friends).
use url::Url;

const URL_TEMPLATE: &str = "[InternetShortcut]\nURL=%(url)s\n";
const WEBLOC_TEMPLATE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n\t<key>URL</key>\n\t<string>%(url)s</string>\n</dict>\n</plist>\n";
const DESKTOP_TEMPLATE: &str =
    "[Desktop Entry]\nEncoding=UTF-8\nName=%(filename)s\nType=Link\nURL=%(url)s\nIcon=text-html\n";

/// Python `urllib.parse.quote_plus(text, safe)`.
fn quote_plus(text: &str, safe: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        let c = byte as char;
        if byte.is_ascii_alphanumeric()
            || "_.-~".contains(c)
            || (byte.is_ascii() && safe.contains(c))
        {
            out.push(c);
        } else if c == ' ' {
            out.push('+');
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// Upstream `iri_to_uri`: `None` for other schemes or IPv6 hosts.
pub fn iri_to_uri(url: &Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?;
    if host.starts_with('[') {
        return None;
    }
    let mut out = format!("{}://", url.scheme());
    let userinfo_safe = "!$%&'()*+,~";
    if !url.username().is_empty() {
        out.push_str(&quote_plus(url.username(), userinfo_safe).replace('+', "%20"));
        if let Some(password) = url.password() {
            out.push(':');
            out.push_str(&quote_plus(password, userinfo_safe).replace('+', "%20"));
        }
        out.push('@');
    }
    out.push_str(host);
    if let Some(port) = url.port().filter(|&port| port != 80) {
        out.push_str(&format!(":{port}"));
    }
    out.push_str(&quote_plus(url.path(), "!$%&'()*+,/:;=@|~"));
    if let Some(query) = url.query().filter(|query| !query.is_empty()) {
        out.push('?');
        out.push_str(&quote_plus(query, "!$%&'()*+,/:;=?@{|}~"));
    }
    if let Some(fragment) = url.fragment().filter(|fragment| !fragment.is_empty()) {
        out.push('#');
        out.push_str(&quote_plus(fragment, "!#$%&'()*+,/:;=?@{|}~"));
    }
    Some(out)
}

/// `_desktop_entry_localestring`.
fn localestring(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            ' ' => out.push_str("\\s"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out
}

/// The shortcut file text for `kind` (`url`, `webloc` or `desktop`), with
/// `.url` files using CRLF line endings as upstream writes them.
pub fn link_file(kind: &str, uri: &str, path: &str) -> String {
    let text = match kind {
        "url" => URL_TEMPLATE.replace("%(url)s", uri),
        "webloc" => WEBLOC_TEMPLATE.replace("%(url)s", uri),
        _ => {
            let name = path.strip_suffix(".desktop").unwrap_or(path);
            DESKTOP_TEMPLATE
                .replace("%(filename)s", &localestring(name))
                .replace("%(url)s", uri)
        }
    };
    if kind == "url" {
        text.replace('\n', "\r\n")
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_upstream_iri_to_uri_cases() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/links/cases.json")).unwrap();
        for case in fixture["iri_to_uri"].as_array().unwrap() {
            let url = Url::parse(case["input"].as_str().unwrap()).unwrap();
            assert_eq!(
                iri_to_uri(&url).as_deref(),
                case["output"].as_str(),
                "{url}"
            );
        }
        for case in fixture["files"].as_array().unwrap() {
            assert_eq!(
                link_file(
                    case["kind"].as_str().unwrap(),
                    case["url"].as_str().unwrap(),
                    case["path"].as_str().unwrap()
                ),
                case["text"].as_str().unwrap()
            );
        }
    }
}

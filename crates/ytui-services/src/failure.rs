//! Read-only recovery guidance. Never changes sessions, tools or networking.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recovery {
    pub title: &'static str,
    pub instruction: &'static str,
}
pub fn recovery(message: &str) -> Option<Recovery> {
    let text = message.to_lowercase();
    let has = |patterns: &[&str]| patterns.iter().any(|p| text.contains(p));
    let (title, instruction) = if has(&["not a bot", "bot confirmation", "bot-confirmation"]) {
        (
            "YouTube requires verification",
            "Open Account in Actions. Complete YouTube verification in your browser, then reconnect the signed-in session.",
        )
    } else if has(&[
        "all player responses are invalid",
        "ip is likely being blocked",
    ]) {
        (
            "YouTube rejected player responses",
            "Check this video in your browser on the same network. The extractor reports possible IP blocking; verify account/network access before retrying.",
        )
    } else if has(&[
        "connect tunnel",
        "tunnel connection failed",
        "tunnel failed",
        "tunnel error",
        "connect to proxy",
        "proxy connection",
        "proxy error",
        "proxyerror",
    ]) {
        (
            "Proxy connection failed",
            "Check the proxy in Settings → Network & account and your network connection before retrying.",
        )
    } else if has(&["no space left", "disk quota exceeded"]) {
        (
            "Storage is full",
            "Free space on the filesystem shown in Diagnostics before retrying. Existing completed files are kept.",
        )
    } else if has(&["permission denied", "read-only file system"]) {
        (
            "Access denied",
            "Check the path or device shown in Diagnostics: download folder permissions, tool executables or mpv audio/video access.",
        )
    } else if has(&[
        "http error 429",
        "http 429",
        "too many requests",
        "rate limit",
    ]) {
        (
            "YouTube rate limit",
            "Wait for the service cooldown before retrying. Repeated refreshes will not help.",
        )
    } else if has(&[
        "native youtube solving requires",
        "configured native javascript runtime was not found",
    ]) {
        (
            "Native JavaScript runtime needs attention",
            "Use installed Node.js 22+ or Deno 2.9.7. Clear the native runtime field in Settings → Tools for automatic detection.",
        )
    } else if has(&[
        "no supported javascript runtime",
        "no valid javascript runtime",
        "javascript runtime could not",
        "deno was not found",
    ]) {
        (
            "YouTube JavaScript runtime is missing",
            "Install a supported Deno runtime and update yt-dlp, then check executable paths in Settings → Tools.",
        )
    } else if has(&[
        "cannot start ",
        "yt-dlp was not found",
        "yt-dlp is required",
    ]) {
        (
            "A required tool could not start",
            "Check installed executable paths and permissions in Settings → Tools.",
        )
    } else if has(&[
        "sign in",
        "session has expired",
        "cookies are no longer valid",
        "login required",
        "authentication required",
    ]) {
        (
            "YouTube sign-in needs attention",
            "Open Account in Actions and reconnect the selected signed-in browser session.",
        )
    } else if has(&[
        "requested format",
        "no eligible media",
        "signature/n",
        "url challenges",
    ]) {
        (
            "YouTube extraction needs attention",
            "Open Playback settings and check the native JavaScript runtime and quality. Native mode never switches to external yt-dlp.",
        )
    } else if has(&[
        "video is unavailable",
        "video unavailable",
        "private video",
        "video is private",
        "video has been removed",
    ]) {
        (
            "Video is unavailable to this session",
            "Check the video in your browser and use content available to your current account.",
        )
    } else if has(&[
        "timed out",
        "timeout",
        "connection refused",
        "network is unreachable",
        "connection reset",
        "stream ended before its expected duration",
        "error sending request",
    ]) {
        (
            "Network request failed",
            "Check your connection and retry. Keep the existing download job to retain its source and completed files.",
        )
    } else {
        return None;
    };
    Some(Recovery { title, instruction })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_distinguishes_verification_proxy_runtime_and_local_failures() {
        for (error, title) in [
            (
                "ERROR: Sign in to confirm you’re not a bot",
                "YouTube requires verification",
            ),
            (
                "All player responses are invalid. Your IP is likely being blocked by Youtube",
                "YouTube rejected player responses",
            ),
            (
                "Unable to connect to proxy: Tunnel connection failed: 403 Forbidden",
                "Proxy connection failed",
            ),
            (
                "No supported JavaScript runtime could be found",
                "YouTube JavaScript runtime is missing",
            ),
            ("HTTP Error 429: Too Many Requests", "YouTube rate limit"),
            (
                "Browser session has expired; import it again",
                "YouTube sign-in needs attention",
            ),
            ("No space left on device", "Storage is full"),
            ("Permission denied", "Access denied"),
            (
                "Cannot start mpv; install it or configure its path",
                "A required tool could not start",
            ),
            (
                "This video is unavailable",
                "Video is unavailable to this session",
            ),
            (
                "no eligible media formats; signature/n URL challenges",
                "YouTube extraction needs attention",
            ),
            ("Connection reset by peer", "Network request failed"),
        ] {
            assert_eq!(recovery(error).unwrap().title, title, "{error}");
        }
        assert_eq!(recovery("unknown fixture failure"), None);
    }
}

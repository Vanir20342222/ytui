//! Upstream-style informational messages (`to_screen`) and warnings from
//! library code, delivered to a process-wide sink installed by the CLI.
//! Without a sink, messages are discarded.
use std::{
    cell::Cell,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

/// One message: a `[extractor] …` screen line, a warning text (without
/// the `WARNING: ` prefix), or a complete line upstream writes to stderr
/// with `write_string`, independently of quiet/no-warnings options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message<'a> {
    Screen(&'a str),
    Warning(&'a str),
    Stderr(&'a str),
    /// A `write_debug` text (without the `[debug] ` prefix); only sent with
    /// `--verbose`.
    Debug(&'a str),
    /// A line Python's `print()` writes to stdout (`--print-traffic`),
    /// independently of quiet modes.
    Stdout(&'a str),
    /// A postprocessor progress hook (`PostProcessor._hook_progress`):
    /// `started` before its run, `finished` after a successful one.
    Postprocess {
        postprocessor: &'a str,
        status: &'a str,
    },
}

type Sink = Box<dyn Fn(Message<'_>) + Send + Sync>;

static SINK: OnceLock<Sink> = OnceLock::new();

/// The thread that installed the sink, which may hold the output streams.
static SINK_THREAD: OnceLock<std::thread::ThreadId> = OnceLock::new();

/// Install the sink once; later calls are ignored.
pub fn set_sink(sink: impl Fn(Message<'_>) + Send + Sync + 'static) {
    if SINK.set(Box::new(sink)).is_ok() {
        let _ = SINK_THREAD.set(std::thread::current().id());
    }
}

/// Messages upstream reports when it first loads its cookie jar (lazily,
/// during the first request): `--cookies-from-browser` extraction.
static AT_FIRST_REQUEST: std::sync::Mutex<Vec<crate::browser_cookies::Message>> =
    std::sync::Mutex::new(Vec::new());

static COOKIE_ERROR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
static COOKIE_REPORTED_ERROR: AtomicBool = AtomicBool::new(false);

/// Whether loading cookies reported a (non-fatal) error, which upstream's
/// `report_error` counts toward the exit status.
pub fn cookies_reported_error() -> bool {
    COOKIE_REPORTED_ERROR.load(Ordering::Relaxed)
}

pub(crate) fn defer_until_request(
    messages: Vec<crate::browser_cookies::Message>,
    error: Option<String>,
) {
    AT_FIRST_REQUEST.lock().unwrap().extend(messages);
    *COOKIE_ERROR.lock().unwrap() = error;
}

/// Upstream's lazy cookie-jar load, before a request (`--verbose` loads it
/// for the debug header, saving cookies at the end): the deferred messages,
/// then a deferred load error. Other threads leave both for the sink's.
pub fn load_cookies() -> crate::Result<()> {
    if SINK_THREAD.get() != Some(&std::thread::current().id()) {
        return Ok(());
    }
    flush_traffic();
    // A failed load is retried (and fails again) at every later access.
    let error = COOKIE_ERROR.lock().unwrap().clone();
    let messages = if error.is_some() {
        AT_FIRST_REQUEST.lock().unwrap().clone()
    } else {
        std::mem::take(&mut *AT_FIRST_REQUEST.lock().unwrap())
    };
    for message in messages {
        match message {
            crate::browser_cookies::Message::Info(text) => screen(&text),
            crate::browser_cookies::Message::Debug(text) => debug(&text),
            crate::browser_cookies::Message::Warning(text) => warning(&text),
            crate::browser_cookies::Message::Error(text) => {
                COOKIE_REPORTED_ERROR.store(true, Ordering::Relaxed);
                stderr(&format!("ERROR: {text}"));
            }
        }
    }
    match error {
        Some(error) => Err(crate::Error::CookieLoad(error)),
        None => Ok(()),
    }
}

/// Print captured `--print-traffic` lines, on the sink's thread only.
pub(crate) fn flush_traffic() {
    if SINK_THREAD.get() != Some(&std::thread::current().id()) {
        return;
    }
    if let Some(sink) = SINK.get() {
        for line in crate::traffic::take_lines() {
            sink(Message::Stdout(&line));
        }
    }
}

/// `load_cookies` where a failure cannot be returned (it stays pending).
pub(crate) fn after_request() {
    flush_traffic();
    if COOKIE_ERROR.lock().unwrap().is_none() {
        let _ = load_cookies();
    }
}

pub(crate) fn screen(text: &str) {
    if let Some(sink) = SINK.get() {
        sink(Message::Screen(text));
    }
}

/// `PostProcessor._hook_progress({'status': status}, info)`.
pub(crate) fn postprocessor(postprocessor: &str, status: &str) {
    if let Some(sink) = SINK.get() {
        sink(Message::Postprocess {
            postprocessor,
            status,
        });
    }
}

/// Upstream `run_wrapper`: `started`, the run, then `finished` unless it
/// failed.
pub(crate) fn postprocess<T>(
    postprocessor: &str,
    run: impl FnOnce() -> crate::Result<T>,
) -> crate::Result<T> {
    self::postprocessor(postprocessor, "started");
    let result = run()?;
    self::postprocessor(postprocessor, "finished");
    Ok(result)
}

/// `report_warning(..., only_once=True)`.
pub(crate) fn warning_once(text: &str) {
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap();
    if seen.iter().any(|message| message == text) {
        return;
    }
    seen.push(text.to_owned());
    drop(seen);
    warning(text);
}

pub(crate) fn warning(text: &str) {
    if let Some(sink) = SINK.get() {
        sink(Message::Warning(text));
    }
}

static VERBOSE: AtomicBool = AtomicBool::new(false);

/// `--verbose`: deliver `write_debug` messages.
pub fn set_verbose(verbose: bool) {
    VERBOSE.store(verbose, Ordering::Relaxed);
}

pub fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// Upstream `write_debug`.
pub(crate) fn debug(text: &str) {
    if verbose()
        && let Some(sink) = SINK.get()
    {
        sink(Message::Debug(text));
    }
}

pub(crate) fn stderr(text: &str) {
    if let Some(sink) = SINK.get() {
        sink(Message::Stderr(text));
    }
}

/// Python `str(float)` for finite values in the ranges used by options.
pub fn py_float(value: f64) -> String {
    if value.is_nan() {
        "nan".into()
    } else if value.is_infinite() {
        if value > 0.0 { "inf" } else { "-inf" }.into()
    } else if value.fract() == 0.0 && value.abs() < 1e16 {
        format!("{value:.1}")
    } else {
        format!("{value}")
    }
}

static SLEEP_REQUESTS: OnceLock<f64> = OnceLock::new();
static FIRST_REQUEST_DONE: AtomicBool = AtomicBool::new(false);

thread_local! {
    static EXTRACTOR: Cell<&'static str> = const { Cell::new("generic") };
}

/// `--sleep-requests`: seconds to wait before each extraction request after
/// the first of the run. Set once; later calls are ignored.
pub fn set_sleep_requests(seconds: f64) {
    let _ = SLEEP_REQUESTS.set(seconds);
}

/// The extractor name used to prefix request-sleep messages.
pub(crate) fn set_extractor(name: &'static str) {
    EXTRACTOR.with(|extractor| extractor.set(name));
}

/// Upstream `_request_webpage`: sleep before every extraction request but the
/// first, announcing it like `[EXTRACTOR] Sleeping SECONDS seconds ...`.
pub(crate) fn before_extraction_request() {
    if !FIRST_REQUEST_DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    if let Some(&seconds) = SLEEP_REQUESTS.get().filter(|seconds| **seconds > 0.0) {
        let extractor = EXTRACTOR.with(Cell::get);
        screen(&format!(
            "[{extractor}] Sleeping {} seconds ...",
            py_float(seconds)
        ));
        if seconds.is_finite() {
            std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
        }
    }
}

/// `--dump-pages`/`--write-pages` (`dump_intermediate_pages`/`write_pages`)
/// and `--trim-filenames` for the dump names.
static DUMP_PAGES: AtomicBool = AtomicBool::new(false);
static WRITE_PAGES: AtomicBool = AtomicBool::new(false);
static TRIM_FILE_NAME: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn set_page_dumps(dump: bool, write: bool, trim_file_name: Option<usize>) {
    DUMP_PAGES.store(dump, Ordering::Relaxed);
    WRITE_PAGES.store(write, Ordering::Relaxed);
    TRIM_FILE_NAME.store(trim_file_name.unwrap_or(0), Ordering::Relaxed);
}

/// The next extraction body read through `_webpage_read_content`: the video
/// ID for its dump name and the lines around the read.
struct ExpectedPage {
    id: String,
    before: Option<String>,
    after: Option<String>,
}

thread_local! {
    static PAGE: std::cell::RefCell<Option<ExpectedPage>> = const { std::cell::RefCell::new(None) };
}

/// Expect a page read for `id`, printing `before` just before it (the
/// request's note) and `after` once it has been read.
pub(crate) fn expect_page(id: &str, before: Option<String>, after: Option<String>) {
    PAGE.with(|page| {
        *page.borrow_mut() = Some(ExpectedPage {
            id: id.into(),
            before,
            after,
        });
    });
}

/// Drop an expectation whose request was not made.
pub(crate) fn clear_page() {
    PAGE.with(|page| page.borrow_mut().take());
}

/// Upstream `_webpage_read_content` for an expected page: the note, then
/// `Dumping request to URL` with the base64 body and `Saving request to
/// FILE` (`_request_dump_filename`), then the follow-up line.
pub(crate) fn page_read(url: &url::Url, bytes: &[u8]) -> std::io::Result<()> {
    let Some(page) = PAGE.with(|page| page.borrow_mut().take()) else {
        return Ok(());
    };
    if let Some(before) = &page.before {
        screen(before);
    }
    let extractor = EXTRACTOR.with(Cell::get);
    if DUMP_PAGES.load(Ordering::Relaxed) {
        use base64::Engine;
        screen(&format!("[{extractor}] Dumping request to {url}"));
        screen(&base64::engine::general_purpose::STANDARD.encode(bytes));
    }
    if WRITE_PAGES.load(Ordering::Relaxed) {
        let filename = request_dump_filename(
            url.as_str(),
            &page.id,
            TRIM_FILE_NAME.load(Ordering::Relaxed),
        );
        screen(&format!("[{extractor}] Saving request to {filename}"));
        std::fs::write(&filename, bytes)?;
    }
    if let Some(after) = &page.after {
        screen(after);
    }
    Ok(())
}

/// Upstream `_request_dump_filename` without request data.
fn request_dump_filename(url: &str, id: &str, trim_length: usize) -> String {
    let mut basen = [id, url]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    let trim_length = if trim_length == 0 { 240 } else { trim_length };
    let length = basen.chars().count();
    if length > trim_length {
        let hash = format!("___{}", crate::md5::hex(basen.as_bytes()));
        let keep = trim_length.saturating_sub(hash.chars().count());
        basen = basen.chars().take(keep).collect::<String>() + &hash;
    }
    crate::output::sanitize_filename(&format!("{basen}.dump"), true)
}

/// Upstream `truncate_string(s, left, right)`.
pub(crate) fn truncate_string(text: &str, left: usize, right: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= left + right {
        return text.to_owned();
    }
    let head: String = chars[..left - 3].iter().collect();
    let tail: String = if right > 0 {
        chars[chars.len() - right..].iter().collect()
    } else {
        String::new()
    };
    format!("{head}...{tail}")
}

/// Upstream `_debug_cmd`/`real_run_ffmpeg`: `ffmpeg command line: …` with
/// the shell-quoted executable and arguments.
/// `_debug_cmd(args)`: `EXE command line: ARGS` with `basename(args[0])`.
pub(crate) fn debug_named_command(executable: &str, args: &[std::ffi::OsString]) {
    if !verbose() {
        return;
    }
    let name = std::path::Path::new(executable)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let quoted: Vec<String> = std::iter::once(executable.to_owned())
        .chain(args.iter().map(|arg| arg.to_string_lossy().into_owned()))
        .map(|arg| crate::output::exec::shell_quote(&arg))
        .collect();
    debug(&format!("{name} command line: {}", quoted.join(" ")));
}

pub(crate) fn debug_command(executable: &std::path::Path, args: &[std::ffi::OsString]) {
    if !verbose() {
        return;
    }
    let stem = executable
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let name = if stem.contains("ffprobe") {
        "ffprobe"
    } else {
        "ffmpeg"
    };
    let quoted: Vec<String> = std::iter::once(executable.as_os_str())
        .chain(args.iter().map(std::ffi::OsString::as_os_str))
        .map(|arg| crate::output::exec::shell_quote(&arg.to_string_lossy()))
        .collect();
    debug(&format!("{name} command line: {}", quoted.join(" ")));
}

/// The URL in `Extracting URL:` lines: truncated unless `--verbose`.
pub(crate) fn extracting_url(url: &str) -> String {
    if verbose() {
        url.to_owned()
    } else {
        truncate_string(url, 100, 20)
    }
}

/// Upstream `InfoExtractor._generic_id`: the unquoted last path component
/// of the URL text without its extension.
pub(crate) fn generic_id(url: &str) -> String {
    let last = url.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    // `os.path.splitext`: the last dot, unless only dots precede it.
    let stem = match last.rfind('.') {
        Some(dot) if last[..dot].bytes().any(|b| b != b'.') => &last[..dot],
        _ => last,
    };
    // `urllib.parse.unquote` with UTF-8 and replacement characters.
    percent_encoding::percent_decode_str(stem)
        .decode_utf8_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_ids_and_truncation_follow_upstream() {
        for (url, id) in [
            ("http://h/media/clip.mp4?x=1#f", "clip"),
            ("http://h/dir/", "dir"),
            ("http://127.0.0.1:9/", "127.0.0"),
            ("http://h/a%20b.mp4", "a b"),
            ("http://h/.hidden", ".hidden"),
            ("http://h/..x.y", "..x"),
            ("http://h/%FF.mp4", "\u{fffd}"),
        ] {
            assert_eq!(generic_id(url), id, "{url}");
        }
        let long = format!("http://h/{}", "a".repeat(200));
        let truncated = truncate_string(&long, 100, 20);
        assert_eq!(truncated.chars().count(), 120);
        assert!(truncated.contains("..."));
        assert_eq!(truncate_string("short", 100, 20), "short");
    }
}

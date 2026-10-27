use crate::{Error, Result, VideoInfo};
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_RANGE, ETAG, IF_RANGE, RANGE, REFERER},
};
use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    ops::ControlFlow,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

/// Checkpoints at which a synchronous download observer can cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadPhase {
    Requesting,
    Downloading,
    /// An external downloader finished with `downloaded_bytes` on disk
    /// (upstream's single `finished` progress hook).
    Completed,
    /// A downloader stopped without raising (upstream `real_download`
    /// returning `False`), which ends its status line.
    Failed,
    WaitingToRetry,
    Publishing,
    Merging,
    Postprocessing,
}

/// Bytes stored for the current representation, rather than cumulative traffic.
/// A full-response retry or the next selected stream can reset bytes and total.
/// Merging reports summed input bytes; final publication reports output bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DownloadProgress {
    /// One-based attempt number, including the initial request (maximum 256).
    pub attempt: u16,
    pub downloaded_bytes: u64,
    /// Total from validated download response headers, absent when unknown.
    pub total_bytes: Option<u64>,
    pub phase: DownloadPhase,
    /// One-based selected input stream, present only for merged downloads.
    pub stream_index: Option<u8>,
    pub stream_count: Option<u8>,
    /// One-based fragment position in the finite selected playlist.
    pub fragment_index: Option<u32>,
    pub fragment_count: Option<u32>,
    /// The current fragment's own bytes, as its HttpFD reports them.
    pub fragment: Option<FragmentBytes>,
}

/// One fragment's HttpFD progress: bytes so far and its total (the
/// response length; its size once finished).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FragmentBytes {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub finished: bool,
}

/// `--buffer-size` (upstream `buffersize`, default 1024).
pub static BUFFER_SIZE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(1024);
/// `--no-resize-buffer` clears this (upstream `noresizebuffer`).
pub static RESIZE_BUFFER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// HttpFD's reads: `data.read(block_size)` returns a full block (urllib's
/// buffered read) until the end, and each loop run resizes the block with
/// `best_block_size(elapsed, len(block))`.
pub(crate) struct Blocks {
    size: usize,
    resize: bool,
    before: std::time::Instant,
    buffer: Vec<u8>,
    /// A read error after part of a block, returned by the next read.
    pending: Option<std::io::Error>,
}

impl Blocks {
    pub(crate) fn new() -> Self {
        use std::sync::atomic::Ordering::Relaxed;
        Self {
            size: BUFFER_SIZE.load(Relaxed).max(1),
            resize: RESIZE_BUFFER.load(Relaxed),
            before: std::time::Instant::now(),
            buffer: Vec::new(),
            pending: None,
        }
    }

    /// The next block (at most `limit` bytes); empty at the end.
    pub(crate) fn read(
        &mut self,
        reader: &mut impl Read,
        limit: Option<u64>,
    ) -> std::io::Result<&[u8]> {
        if let Some(error) = self.pending.take() {
            return Err(error);
        }
        let want = limit.map_or(self.size, |limit| self.size.min(limit as usize));
        self.buffer.resize(want, 0);
        let mut filled = 0;
        while filled < want {
            match reader.read(&mut self.buffer[filled..]) {
                Ok(0) => break,
                Ok(count) => filled += count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) if filled > 0 => {
                    // Keep the data read so far; report the error next.
                    self.pending = Some(error);
                    break;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(&self.buffer[..filled])
    }

    /// The end of one loop run (after writing and rate limiting).
    pub(crate) fn finished(&mut self, length: usize) {
        let now = std::time::Instant::now();
        if self.resize {
            self.size = best_block_size(now.duration_since(self.before).as_secs_f64(), length);
        }
        self.before = now;
    }
}

/// Upstream `FileDownloader.best_block_size`.
fn best_block_size(elapsed: f64, bytes: usize) -> usize {
    let bytes = bytes as f64;
    let new_min = (bytes / 2.0).max(1.0);
    let new_max = (bytes * 2.0).clamp(1.0, 4_194_304.0);
    if elapsed < 0.001 {
        return new_max as usize;
    }
    let rate = bytes / elapsed;
    if rate > new_max {
        new_max as usize
    } else if rate < new_min {
        new_min as usize
    } else {
        rate as usize
    }
}

struct Reporter<'a> {
    callback: &'a mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
    attempt: u16,
    total: Option<u64>,
}

impl Reporter<'_> {
    fn report(&mut self, phase: DownloadPhase, downloaded_bytes: u64) -> Result<()> {
        match (self.callback)(DownloadProgress {
            attempt: self.attempt,
            downloaded_bytes,
            total_bytes: self.total,
            phase,
            stream_index: None,
            stream_count: None,
            fragment_index: None,
            fragment_count: None,
            fragment: None,
        }) {
            ControlFlow::Continue(()) => Ok(()),
            ControlFlow::Break(()) => Err(Error::DownloadCancelled),
        }
    }

    fn wait(&mut self, duration: Duration, actual: u64) -> Result<()> {
        let start = Instant::now();
        loop {
            self.report(DownloadPhase::WaitingToRetry, actual)?;
            let remaining = duration.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Ok(());
            }
            std::thread::sleep(remaining.min(Duration::from_millis(100)));
        }
    }
}

/// Download retry policy, independent of `ExtractionOptions`.
#[derive(Debug, Clone, Copy)]
pub struct DownloadOptions {
    /// Additional attempts after a transient failure (`u32::MAX` retries
    /// indefinitely, like upstream `infinite`).
    pub retries: u32,
    /// Additional attempts for each fragment (`--fragment-retries`).
    pub fragment_retries: u32,
    /// `--retry-sleep` delays; without one, a short native backoff applies.
    pub retry_sleep: crate::retry::RetrySleep,
    /// Upstream fragment semantics: every HTTP status error is retried, and
    /// with `Some(true)` (`--skip-unavailable-fragments`) a media fragment
    /// whose retries are exhausted is skipped (the first never is). `None`
    /// keeps all-or-error assembly retrying transient failures only.
    pub skip_unavailable_fragments: Option<bool>,
    /// `--min-filesize`: abort a whole-file media transfer whose declared
    /// length is smaller (upstream HTTP downloader check).
    pub min_filesize: Option<u64>,
    /// `--max-filesize`: abort a whole-file media transfer whose declared
    /// length is larger.
    pub max_filesize: Option<u64>,
    /// `--limit-rate`: maximum bytes per second for each media transfer or
    /// fragment request (upstream `slow_down`).
    pub rate_limit: Option<u64>,
    /// `--mtime`: set media and subtitle modification times from the
    /// response `Last-Modified` header.
    pub update_time: bool,
    /// Where a whole-file media transfer is written before publication.
    pub part_files: PartFiles,
    /// Where the streams of a merged download are stored.
    pub merge_files: MergeFiles,
    /// Replace an existing destination, like upstream downloading again over
    /// an existing file without `--continue` or with `--no-part`.
    pub overwrite: bool,
}

/// Refuse an existing destination unless `options.overwrite`.
pub(crate) fn check_destination(destination: &Path, options: &DownloadOptions) -> Result<()> {
    if destination.exists() && !options.overwrite {
        return Err(Error::OutputExists(destination.into()));
    }
    Ok(())
}

/// [`persist`] for a temporary path.
pub(crate) fn persist_path(
    path: tempfile::TempPath,
    destination: &Path,
    overwrite: bool,
) -> Result<()> {
    let result = if overwrite {
        path.persist(destination)
    } else {
        path.persist_noclobber(destination)
    };
    result.map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            Error::OutputExists(destination.into())
        } else {
            Error::Io(error.error)
        }
    })
}

/// Publish a finished temporary file, replacing an existing destination
/// only with `overwrite`.
pub(crate) fn persist(
    file: tempfile::NamedTempFile,
    destination: &Path,
    overwrite: bool,
) -> Result<()> {
    let result = if overwrite {
        file.persist(destination).map(drop)
    } else {
        file.persist_noclobber(destination).map(drop)
    };
    result.map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            Error::OutputExists(destination.into())
        } else {
            Error::Io(error.error)
        }
    })
}

/// Intermediate-file policy for merged (two-stream) downloads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MergeFiles {
    /// A private same-directory workspace, removed afterwards (library default).
    #[default]
    Private,
    /// Upstream `NAME.fFORMAT_ID.EXT` files beside the destination: existing
    /// ones are reused, each download prints its downloader lines, and the
    /// files are deleted after merging unless `keep` (`-k`).
    Upstream { keep: bool },
}

/// Temporary-file policy for whole-file media downloads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PartFiles {
    /// A private same-directory temporary file, removed on failure or
    /// cancellation and published without overwriting (library default).
    #[default]
    Private,
    /// Upstream `FILE.part`, kept on failure; with `resume`, an existing one
    /// is continued with a `Range` request (`--continue`).
    Part { resume: bool },
    /// Upstream `--no-part`: write the destination directly; with `resume`
    /// (`--continue`), an existing destination is continued.
    NoPart { resume: bool },
}

/// Upstream `FileDownloader.slow_down`: after each read, sleep until the
/// average speed since the request started is at most the limit.
pub(crate) struct RateLimit {
    limit: Option<u64>,
    start: Instant,
    bytes: u64,
}

impl RateLimit {
    pub(crate) fn new(limit: Option<u64>) -> Self {
        Self {
            limit,
            start: Instant::now(),
            bytes: 0,
        }
    }

    pub(crate) fn consume(&mut self, count: usize) {
        let Some(limit) = self.limit.filter(|&limit| limit > 0) else {
            return;
        };
        self.bytes = self.bytes.saturating_add(count as u64);
        let target = Duration::from_secs_f64(self.bytes as f64 / limit as f64);
        let elapsed = self.start.elapsed();
        if !elapsed.is_zero() && target > elapsed {
            std::thread::sleep(target - elapsed);
        }
    }
}

impl Default for DownloadOptions {
    fn default() -> Self {
        Self {
            retries: 3,
            fragment_retries: 3,
            retry_sleep: crate::retry::RetrySleep::default(),
            skip_unavailable_fragments: None,
            min_filesize: None,
            max_filesize: None,
            rate_limit: None,
            update_time: false,
            part_files: PartFiles::Private,
            merge_files: MergeFiles::Private,
            overwrite: false,
        }
    }
}

struct AttemptFailure {
    error: Error,
    retryable: bool,
    retry_after: std::time::Duration,
}

impl From<Error> for AttemptFailure {
    fn from(error: Error) -> Self {
        Self {
            error,
            retryable: false,
            retry_after: std::time::Duration::ZERO,
        }
    }
}

impl From<std::io::Error> for AttemptFailure {
    fn from(error: std::io::Error) -> Self {
        Error::Io(error).into()
    }
}

impl AttemptFailure {
    fn retry(error: Error) -> Self {
        Self {
            error,
            retryable: true,
            retry_after: std::time::Duration::ZERO,
        }
    }

    fn request(error: Error) -> Self {
        let delay = crate::retry::download_failure_delay(&error);
        Self {
            error,
            retryable: delay.is_some(),
            retry_after: delay.unwrap_or_default(),
        }
    }
}

// Only strong ETags identify byte-for-byte representations. Last-Modified alone
// cannot reliably guard appends when a resource changes within one second.
struct ResumeState {
    etag: reqwest::header::HeaderValue,
    url: url::Url,
    total: u64,
}

fn identity_encoded(response: &Response) -> bool {
    let values = response.headers().get_all(CONTENT_ENCODING);
    values.iter().all(|value| value == "identity")
}

fn resume_state(response: &Response) -> Option<ResumeState> {
    if response.status() != reqwest::StatusCode::OK || !identity_encoded(response) {
        return None;
    }
    let etag = response.headers().get(ETAG)?;
    let bytes = etag.as_bytes();
    // RFC entity-tag: DQUOTE *etagc DQUOTE, without the weak W/ prefix.
    if bytes.len() < 2
        || bytes[0] != b'"'
        || bytes[bytes.len() - 1] != b'"'
        || !bytes[1..bytes.len() - 1]
            .iter()
            .all(|&byte| byte == 0x21 || (0x23..=0x7e).contains(&byte) || byte >= 0x80)
        || response.headers().get_all(ETAG).iter().count() != 1
    {
        return None;
    }
    Some(ResumeState {
        etag: etag.clone(),
        url: response.url().clone(),
        total: response.content_length()?,
    })
}

/// `parse_http_range(Content-Range)`: the start and the total, if known.
fn content_range(response: &Response) -> Option<(u64, Option<u64>)> {
    let range = response.headers().get(CONTENT_RANGE)?.to_str().ok()?;
    let (span, total) = range.strip_prefix("bytes ")?.split_once('/')?;
    let (start, _) = span.split_once('-')?;
    let start = start.trim().parse().ok()?;
    let total = if total.trim() == "*" {
        None
    } else {
        Some(total.trim().parse().ok()?)
    };
    Some((start, total))
}

fn validate_range(response: &Response, state: &ResumeState, offset: u64) -> Result<()> {
    let invalid = || Error::InvalidRangeResponse;
    let range = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(invalid)?;
    let (span, total) = range
        .strip_prefix("bytes ")
        .and_then(|value| value.split_once('/'))
        .ok_or_else(invalid)?;
    let (start, end) = span.split_once('-').ok_or_else(invalid)?;
    let number = |value: &str| -> Option<u64> {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        value.parse().ok()
    };
    if number(start) != Some(offset)
        || number(end) != state.total.checked_sub(1)
        || number(total) != Some(state.total)
        || response
            .content_length()
            .is_some_and(|length| length != state.total - offset)
        || response.headers().get(ETAG) != Some(&state.etag)
        || response.headers().get_all(ETAG).iter().count() != 1
        || response.headers().get_all(CONTENT_RANGE).iter().count() != 1
        || response.url() != &state.url
        || !identity_encoded(response)
    {
        return Err(invalid());
    }
    Ok(())
}

/// Stream to a same-directory temporary file, then publish without overwriting.
/// Interrupted bodies resume only with a strong ETag and known total length.
/// Partial files are private to this invocation and removed on failure.
pub(crate) fn download(
    client: &Client,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    chunk: Option<Chunking>,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    download_resource(
        client,
        &HttpResource {
            url: &info.url,
            referrer: (!info.is_direct()).then_some(&info.webpage_url),
            validate_media: !info.is_direct(),
            subtitle: false,
            limits: (options.min_filesize, options.max_filesize),
            rate_limit: options.rate_limit,
            update_time: options.update_time,
            part_files: options.part_files,
            chunk,
        },
        destination,
        options,
        callback,
    )
}

/// A local `file:` media download (`--enable-file-urls`): copied with
/// progress like upstream's urllib file handler, then published.
pub(crate) fn download_file(
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    check_destination(destination, &options)?;
    let source = info
        .url
        .to_file_path()
        .map_err(|_| Error::UnsupportedScheme("file".into()))?;
    let mut input = fs::File::open(&source)?;
    let total = input.metadata()?.len();
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::Builder::new()
        .prefix(".yt-dlp-rs-")
        .suffix(".part")
        .tempfile_in(parent)?;
    let mut reporter = Reporter {
        callback,
        attempt: 1,
        total: Some(total),
    };
    let mut copied = 0;
    let mut buffer = [0; 64 * 1024];
    reporter.report(DownloadPhase::Downloading, 0)?;
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        temporary.write_all(&buffer[..count])?;
        copied += count as u64;
        reporter.report(DownloadPhase::Downloading, copied)?;
    }
    temporary.flush()?;
    if options.update_time {
        crate::filetime::apply(
            temporary.as_file(),
            info.metadata
                .timestamp
                .map(crate::info::PyNumber::floor_i64),
        );
    }
    reporter.report(DownloadPhase::Publishing, copied)?;
    Target::Private(temporary).publish(destination, options.overwrite)?;
    Ok(copied)
}

pub(crate) fn download_subtitle(
    client: &Client,
    info: &VideoInfo,
    subtitle: &crate::SubtitleFormat,
    destination: &Path,
    options: DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    if !matches!(subtitle.url.scheme(), "http" | "https") {
        return Err(Error::UnsupportedScheme(subtitle.url.scheme().into()));
    }
    download_resource(
        client,
        &HttpResource {
            url: &subtitle.url,
            referrer: Some(&info.webpage_url),
            validate_media: false,
            subtitle: true,
            limits: (None, None),
            rate_limit: None,
            update_time: options.update_time,
            part_files: PartFiles::Private,
            chunk: None,
        },
        destination,
        options,
        callback,
    )
}

/// Upstream HlsFD for an `m3u8_native` WebVTT subtitle rendition: the
/// playlist's segments joined by WebVTT packing (a single segment is copied
/// as it is), published without overwriting.
pub(crate) fn download_hls_subtitle(
    client: &Client,
    info: &VideoInfo,
    subtitle: &crate::SubtitleFormat,
    destination: &Path,
    options: DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    crate::screen::screen("[hlsnative] Downloading m3u8 manifest");
    let response = crate::extractor::direct::probe(client, &subtitle.url, Some(&info.webpage_url))?;
    let (_, playlist) = crate::extractor::hls::read_playlist(response)?;
    let crate::hls::Playlist::Media { fragments, .. } = playlist else {
        return Err(Error::Subtitle(
            "an HLS subtitle rendition must be a media playlist".into(),
        ));
    };
    join_subtitle_fragments(
        client,
        info,
        &fragments,
        "hlsnative",
        true,
        destination,
        options,
        callback,
    )
}

/// Upstream DashSegmentsFD for a segmented DASH text track: the fragments
/// appended as they are.
pub(crate) fn download_dash_subtitle(
    client: &Client,
    info: &VideoInfo,
    subtitle: &crate::SubtitleFormat,
    destination: &Path,
    options: DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    join_subtitle_fragments(
        client,
        info,
        &subtitle.fragments,
        "dashsegments",
        false,
        destination,
        options,
        callback,
    )
}

/// `Total fragments`, then each fragment downloaded and joined (with WebVTT
/// packing when `webvtt` and there are several), published without
/// overwriting.
#[allow(clippy::too_many_arguments)]
fn join_subtitle_fragments(
    client: &Client,
    info: &VideoInfo,
    fragments: &[crate::Fragment],
    downloader: &str,
    webvtt: bool,
    destination: &Path,
    options: DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    crate::screen::screen(&format!(
        "[{downloader}] Total fragments: {}",
        fragments.len()
    ));
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let workspace = tempfile::Builder::new()
        .prefix(".yt-dlp-rs-subtitle-")
        .tempdir_in(parent)?;
    let single = !webvtt || fragments.len() == 1;
    let mut stored = 0_u64;
    let mut packer = crate::webvtt::Packer::default();
    let mut joined = Vec::new();
    for (index, fragment) in fragments.iter().enumerate() {
        if fragment.byte_range.is_some() || fragment.encryption.is_some() {
            return Err(Error::Subtitle(
                "byte-range or encrypted HLS subtitle segments are unsupported".into(),
            ));
        }
        let path = workspace.path().join(format!("fragment-{index}"));
        download_resource(
            client,
            &HttpResource {
                url: &fragment.url,
                referrer: Some(&info.webpage_url),
                validate_media: false,
                subtitle: true,
                limits: (None, None),
                rate_limit: None,
                update_time: false,
                part_files: PartFiles::Private,
                chunk: None,
            },
            &path,
            DownloadOptions {
                retries: options.fragment_retries,
                ..options
            },
            // Fragment progress like FragmentFD's (one publication is
            // reported for the joined file).
            &mut |progress: DownloadProgress| {
                if progress.phase == DownloadPhase::Publishing {
                    return ControlFlow::Continue(());
                }
                callback(DownloadProgress {
                    downloaded_bytes: stored + progress.downloaded_bytes,
                    total_bytes: None,
                    fragment_index: Some(index as u32 + 1),
                    fragment_count: Some(fragments.len() as u32),
                    fragment: None,
                    ..progress
                })
            },
        )?;
        let content = fs::read(&path)?;
        stored += content.len() as u64;
        if single {
            joined.extend(content);
        } else {
            joined.extend(packer.pack(&content, index + 1, &mut |message| {
                crate::screen::warning(message)
            })?);
        }
    }
    if !single {
        joined.extend(packer.finish());
    }
    let mut output = tempfile::Builder::new()
        .prefix(".yt-dlp-rs-subtitle-")
        .suffix(".part")
        .tempfile_in(parent)?;
    output.write_all(&joined)?;
    output.flush()?;
    let bytes = joined.len() as u64;
    if callback(DownloadProgress {
        attempt: 1,
        downloaded_bytes: bytes,
        total_bytes: Some(bytes),
        phase: DownloadPhase::Publishing,
        stream_index: None,
        stream_count: None,
        fragment_index: None,
        fragment_count: None,
        fragment: None,
    })
    .is_break()
    {
        return Err(Error::DownloadCancelled);
    }
    persist(output, destination, options.overwrite)?;
    Ok(bytes)
}

/// Copy a thumbnail with the whole-file transport (no page referrer, no media
/// MIME check), publishing without overwriting.
pub(crate) fn download_thumbnail(
    client: &Client,
    url: &url::Url,
    destination: &Path,
    options: DownloadOptions,
) -> Result<u64> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::UnsupportedScheme(url.scheme().into()));
    }
    download_resource(
        client,
        &HttpResource {
            url,
            referrer: None,
            validate_media: false,
            subtitle: false,
            limits: (None, None),
            rate_limit: None,
            // Upstream writes thumbnails without the downloader's utime.
            update_time: false,
            part_files: PartFiles::Private,
            chunk: None,
        },
        destination,
        options,
        &mut |_| ControlFlow::Continue(()),
    )
}

struct HttpResource<'a> {
    url: &'a url::Url,
    referrer: Option<&'a url::Url>,
    validate_media: bool,
    subtitle: bool,
    /// Minimum/maximum declared total length in bytes.
    limits: (Option<u64>, Option<u64>),
    rate_limit: Option<u64>,
    update_time: bool,
    part_files: PartFiles,
    /// Upstream chunked HTTP downloading (`http_chunk_size` or `--test`).
    chunk: Option<Chunking>,
}

/// Upstream `HttpFD` chunking: `size` bytes per request (randomized to
/// 95-100% unless `test`); `test` keeps only the first `_TEST_FILE_SIZE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Chunking {
    pub size: u64,
    pub test: bool,
}

/// Upstream `FileDownloader._TEST_FILE_SIZE`.
pub(crate) const TEST_FILE_SIZE: u64 = 10241;

/// `random.randint(low, high)` from a clock-seeded xorshift64*.
fn randint(low: u64, high: u64) -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let mut state = STATE.load(Ordering::Relaxed);
    if state == 0 {
        state = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9e37_79b9_7f4a_7c15, |time| time.as_nanos() as u64)
            | 1;
    }
    state ^= state >> 12;
    state ^= state << 25;
    state ^= state >> 27;
    STATE.store(state, Ordering::Relaxed);
    let value = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
    low + value % (high - low + 1)
}

/// `parse_http_range(Content-Range)`: start, end and total.
fn content_range_span(response: &Response) -> Option<(u64, u64, Option<u64>)> {
    let range = response.headers().get(CONTENT_RANGE)?.to_str().ok()?;
    let (span, total) = range.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = span.split_once('-')?;
    let total = if total.trim() == "*" {
        None
    } else {
        Some(total.trim().parse().ok()?)
    };
    Some((start.trim().parse().ok()?, end.trim().parse().ok()?, total))
}

/// The file a transfer is written to.
enum Target {
    Private(tempfile::NamedTempFile),
    Part { file: fs::File, path: PathBuf },
}

impl Target {
    fn file(&mut self) -> &mut fs::File {
        match self {
            Target::Private(temporary) => temporary.as_file_mut(),
            Target::Part { file, .. } => file,
        }
    }

    fn publish(self, destination: &Path, overwrite: bool) -> Result<()> {
        match self {
            Target::Private(temporary) => persist(temporary, destination, overwrite)?,
            Target::Part { path, .. } if path != destination => {
                if !overwrite && fs::symlink_metadata(destination).is_ok() {
                    return Err(Error::OutputExists(destination.into()));
                }
                fs::rename(&path, destination)?;
            }
            Target::Part { .. } => {}
        }
        Ok(())
    }
}

fn download_resource(
    client: &Client,
    resource: &HttpResource<'_>,
    destination: &Path,
    options: DownloadOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    check_destination(destination, &options)?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let mut actual = 0;
    let mut temporary = match resource.part_files {
        PartFiles::Private => Target::Private(
            tempfile::Builder::new()
                .prefix(".yt-dlp-rs-")
                .suffix(".part")
                .tempfile_in(parent)?,
        ),
        mode => {
            let (path, resume) = match mode {
                PartFiles::NoPart { resume } => (destination.to_owned(), resume),
                PartFiles::Part { resume } => {
                    let mut name = destination.as_os_str().to_owned();
                    name.push(".part");
                    (PathBuf::from(name), resume)
                }
                PartFiles::Private => unreachable!("handled above"),
            };
            // Upstream `continuedl`: continue an existing regular part file
            // (with `--no-part`, the destination itself).
            if resume
                && let Ok(metadata) = fs::metadata(&path)
                && metadata.is_file()
            {
                actual = metadata.len();
            }
            let mut file = fs::OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(actual == 0)
                .open(&path)?;
            file.seek(SeekFrom::End(0))?;
            Target::Part { file, path }
        }
    };
    // A continued part file is first requested with a plain upstream Range.
    let mut continued = actual > 0;
    let resumed = continued;
    let mut state = None;
    let mut last_modified = None;
    let mut reporter = Reporter {
        callback,
        attempt: 1,
        total: None,
    };
    for attempt in 0..=options.retries {
        reporter.attempt = u16::try_from(attempt.saturating_add(1)).unwrap_or(u16::MAX);
        reporter.report(DownloadPhase::Requesting, actual)?;
        match download_attempt(
            client,
            resource,
            temporary.file(),
            &mut state,
            &mut actual,
            &mut continued,
            &mut last_modified,
            &mut reporter,
        ) {
            Ok(()) => {
                temporary.file().flush()?;
                temporary.file().sync_all()?;
                if resource.update_time {
                    crate::filetime::apply(
                        temporary.file(),
                        crate::filetime::filetime(last_modified.as_deref()),
                    );
                }
                reporter.report(DownloadPhase::Publishing, actual)?;
                temporary.publish(destination, options.overwrite)?;
                return Ok(actual);
            }
            Err(failure) if failure.retryable && attempt < options.retries => {
                let delay = crate::retry::announce_retry(
                    &failure.error,
                    attempt,
                    options.retries,
                    None,
                    options.retry_sleep.http,
                )
                .map_or_else(
                    || crate::retry::delay(attempt, failure.retry_after),
                    |delay| delay.max(failure.retry_after),
                );
                reporter.wait(delay, actual)?;
            }
            Err(failure) => {
                // Upstream opens the part file only once data arrives.
                if let Target::Part { path, .. } = &temporary
                    && !resumed
                    && fs::metadata(path).is_ok_and(|metadata| metadata.len() == 0)
                {
                    let _ = fs::remove_file(path);
                }
                return Err(if failure.retryable {
                    crate::retry::exhausted(failure.error, options.retries)
                } else {
                    failure.error
                });
            }
        }
    }
    unreachable!("the final attempt returns success or an error")
}

/// Upstream `HttpFD.real_download` with a chunk size: each connection asks
/// for `bytes=START-END`; a matching `206` continues with the next chunk
/// (`NextFragment`, not a retry), anything else restarts as a whole download.
#[allow(clippy::too_many_arguments)]
fn chunked_attempt(
    client: &Client,
    resource: &HttpResource<'_>,
    temporary: &mut fs::File,
    actual: &mut u64,
    continued: &mut bool,
    last_modified: &mut Option<String>,
    reporter: &mut Reporter<'_>,
    chunk: Chunking,
) -> std::result::Result<(), AttemptFailure> {
    let mut resuming = std::mem::take(continued) && *actual > 0;
    // `ctx.content_len`: the whole resource length once known.
    let mut content_len: Option<u64> = None;
    loop {
        let size = if chunk.test {
            chunk.size
        } else {
            randint((chunk.size as f64 * 0.95) as u64, chunk.size)
        }
        .max(1);
        let start = *actual;
        let mut end = start + size - 1;
        if let Some(length) = content_len
            && end >= length
        {
            end = length.saturating_sub(1);
        }
        if resuming {
            crate::screen::screen(&format!("[download] Resuming download at byte {start}"));
            resuming = false;
        }
        let mut request = client
            .get(resource.url.clone())
            .header(ACCEPT_ENCODING, "identity")
            .header(RANGE, format!("bytes={start}-{end}"));
        if let Some(referrer) = resource.referrer {
            request = request.header(REFERER, referrer.as_str());
        }
        crate::screen::load_cookies().map_err(AttemptFailure::from)?;
        let mut response = request
            .send()
            .inspect(|_| crate::screen::flush_traffic())
            .map_err(Error::Http)
            .and_then(crate::retry::check_response)
            .map_err(AttemptFailure::request)?;
        *last_modified = crate::filetime::header(&response);
        let matched = (response.status() == reqwest::StatusCode::PARTIAL_CONTENT)
            .then(|| content_range_span(&response))
            .flatten()
            .filter(|(range_start, range_end, total)| {
                *range_start == start
                    && (*range_end == end || total.is_some_and(|total| total < end))
            });
        // `ctx.data_len`: the whole length, reported as the total.
        let whole = match matched {
            Some((_, _, total)) => {
                content_len = total;
                total
            }
            None => {
                if start > 0 {
                    crate::screen::screen("[download] Unable to resume");
                }
                temporary.set_len(0)?;
                temporary.seek(SeekFrom::Start(0))?;
                *actual = 0;
                content_len = response.content_length();
                content_len
            }
        };
        if *actual == 0
            && resource.validate_media
            && crate::extractor::direct_media_info(&response, resource.url).is_none()
        {
            return Err(Error::UnsupportedHtml5(
                "source no longer serves supported direct HTTP media".into(),
            )
            .into());
        }
        // `download()`'s `data_len`: this response's end; upstream compares
        // the size limits with it.
        let mut data_len = response.content_length().map(|length| length + *actual);
        if let Some(length) = data_len {
            match resource.limits {
                (Some(min), _) if length < min => {
                    return Err(Error::FileSizeLimit(format!(
                        "File is smaller than min-filesize ({length} bytes < {min} bytes). Aborting."
                    ))
                    .into());
                }
                (_, Some(max)) if length > max => {
                    return Err(Error::FileSizeLimit(format!(
                        "File is larger than max-filesize ({length} bytes > {max} bytes). Aborting."
                    ))
                    .into());
                }
                _ => {}
            }
        }
        if chunk.test && data_len.is_none_or(|length| length > TEST_FILE_SIZE) {
            data_len = Some(TEST_FILE_SIZE);
        }
        reporter.total = whole;
        reporter.report(DownloadPhase::Downloading, *actual)?;
        let mut limiter = RateLimit::new(resource.rate_limit);
        let mut blocks = Blocks::new();
        loop {
            let limit = data_len.map(|length| length.saturating_sub(*actual));
            if limit == Some(0) {
                break;
            }
            let block = match blocks.read(&mut response, limit) {
                Ok([]) => break,
                Ok(block) => block,
                Err(error) => return Err(AttemptFailure::retry(Error::Io(error))),
            };
            let count = block.len();
            temporary.write_all(block)?;
            *actual += count as u64;
            limiter.consume(count);
            blocks.finished(count);
            reporter.report(DownloadPhase::Downloading, *actual)?;
        }
        if !chunk.test
            && let Some(length) = content_len
            && *actual < length
            && data_len.is_none_or(|data_len| *actual == data_len)
        {
            continue;
        }
        if let Some(expected) = data_len
            && *actual != expected
        {
            return Err(AttemptFailure::retry(Error::IncompleteDownload {
                expected,
                actual: *actual,
            }));
        }
        return Ok(());
    }
}

#[allow(clippy::too_many_arguments)]
fn download_attempt(
    client: &Client,
    resource: &HttpResource<'_>,
    temporary: &mut fs::File,
    state: &mut Option<ResumeState>,
    actual: &mut u64,
    continued: &mut bool,
    last_modified: &mut Option<String>,
    reporter: &mut Reporter<'_>,
) -> std::result::Result<(), AttemptFailure> {
    if let Some(chunk) = resource.chunk {
        return chunked_attempt(
            client,
            resource,
            temporary,
            actual,
            continued,
            last_modified,
            reporter,
            chunk,
        );
    }
    let resume = state
        .as_ref()
        .filter(|state| *actual > 0 && *actual < state.total);
    let loose = std::mem::take(continued) && *actual > 0;
    let build = |range: bool| {
        let mut request = client
            .get(resource.url.clone())
            .header(ACCEPT_ENCODING, "identity");
        if let Some(state) = resume {
            request = request
                .header(RANGE, format!("bytes={actual}-"))
                .header(IF_RANGE, state.etag.clone());
        } else if range {
            request = request.header(RANGE, format!("bytes={actual}-"));
        }
        if let Some(referrer) = resource.referrer {
            request = request.header(REFERER, referrer.as_str());
        }
        request
    };
    if loose {
        crate::screen::screen(&format!("[download] Resuming download at byte {actual}"));
    }
    // Upstream's 416 branch re-sends the same Range, so a complete part
    // file fails with that status; the part file is kept.
    crate::screen::load_cookies().map_err(AttemptFailure::from)?;
    let mut response = build(loose)
        .send()
        .inspect(|_| crate::screen::flush_traffic())
        .map_err(Error::Http)
        .and_then(crate::retry::check_response)
        .map_err(AttemptFailure::request)?;
    *last_modified = crate::filetime::header(&response);
    if resource.subtitle
        && !response
            .headers()
            .get_all(CONTENT_ENCODING)
            .iter()
            .all(|value| value.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        return Err(Error::Subtitle("compressed subtitle responses are unsupported".into()).into());
    }
    let expected = if response.status() == reqwest::StatusCode::PARTIAL_CONTENT && loose {
        // Upstream accepts a Content-Range starting at the part file's end.
        let (start, total) = content_range(&response).ok_or(Error::InvalidRangeResponse)?;
        if start != *actual {
            return Err(Error::InvalidRangeResponse.into());
        }
        total
    } else if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        let resume = resume.ok_or(Error::PartialResponse)?;
        validate_range(&response, resume, *actual)?;
        Some(resume.total)
    } else {
        if loose {
            crate::screen::screen("[download] Unable to resume");
        }
        // A server ignoring Range (or returning a changed representation for
        // If-Range) sends a full response. Never append it to the old bytes.
        temporary.set_len(0)?;
        temporary.seek(SeekFrom::Start(0))?;
        *actual = 0;
        *state = resume_state(&response);
        response.content_length()
    };
    if resource.validate_media
        && crate::extractor::direct_media_info(&response, resource.url).is_none()
    {
        return Err(Error::UnsupportedHtml5(
            "source no longer serves supported direct HTTP media".into(),
        )
        .into());
    }
    if let Some(total) = expected {
        match resource.limits {
            (Some(min), _) if total < min => {
                return Err(Error::FileSizeLimit(format!(
                    "File is smaller than min-filesize ({total} bytes < {min} bytes). Aborting."
                ))
                .into());
            }
            (_, Some(max)) if total > max => {
                return Err(Error::FileSizeLimit(format!(
                    "File is larger than max-filesize ({total} bytes > {max} bytes). Aborting."
                ))
                .into());
            }
            _ => {}
        }
    }
    reporter.total = expected;
    reporter.report(DownloadPhase::Downloading, *actual)?;
    let mut limiter = RateLimit::new(resource.rate_limit);
    let mut blocks = Blocks::new();
    loop {
        // Separate network reads from filesystem writes: only the former retry.
        let block = match blocks.read(&mut response, None) {
            Ok([]) => break,
            Ok(block) => block,
            Err(error) => return Err(AttemptFailure::retry(Error::Io(error))),
        };
        let count = block.len();
        let next = actual
            .checked_add(count as u64)
            .ok_or(Error::InvalidRangeResponse)?;
        if expected.is_some_and(|expected| next > expected) {
            return Err(Error::InvalidRangeResponse.into());
        }
        temporary.write_all(block)?;
        *actual = next;
        limiter.consume(count);
        blocks.finished(count);
        reporter.report(DownloadPhase::Downloading, *actual)?;
    }
    if let Some(expected) = expected
        && *actual != expected
    {
        return Err(AttemptFailure::retry(Error::IncompleteDownload {
            expected,
            actual: *actual,
        }));
    }
    Ok(())
}

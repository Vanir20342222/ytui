//! Sequential assembly of saved, finite fragment plans: all-or-error by
//! default, or skipping unavailable fragments like upstream when enabled.
use crate::{DownloadOptions, DownloadPhase, DownloadProgress, Error, Fragment, Result, VideoInfo};
use reqwest::{
    blocking::{Client, Response},
    header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_RANGE, CONTENT_TYPE, RANGE, REFERER},
};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    ops::ControlFlow,
    path::Path,
    time::{Duration, Instant},
};

struct Reporter<'a> {
    callback: &'a mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
    attempt: u16,
    index: u32,
    count: u32,
    /// `--limit-rate`, applied per fragment request like upstream.
    rate_limit: Option<u64>,
    /// The current fragment's HttpFD progress, reported with its data.
    fragment: Option<crate::FragmentBytes>,
}
impl Reporter<'_> {
    fn report(&mut self, phase: DownloadPhase, bytes: u64) -> Result<()> {
        let fragment = self
            .fragment
            .take()
            .filter(|_| phase == DownloadPhase::Downloading);
        if (self.callback)(DownloadProgress {
            attempt: self.attempt,
            downloaded_bytes: bytes,
            total_bytes: None,
            phase,
            stream_index: None,
            stream_count: None,
            fragment_index: Some(self.index),
            fragment_count: Some(self.count),
            fragment,
        })
        .is_break()
        {
            Err(Error::DownloadCancelled)
        } else {
            Ok(())
        }
    }
    fn wait(&mut self, delay: Duration, bytes: u64) -> Result<()> {
        let start = Instant::now();
        loop {
            self.report(DownloadPhase::WaitingToRetry, bytes)?;
            let remaining = delay.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Ok(());
            }
            std::thread::sleep(remaining.min(Duration::from_millis(100)));
        }
    }
}
struct Failure {
    error: Error,
    retry: Option<Duration>,
}
impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self { error, retry: None }
    }
}
impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Error::Io(error).into()
    }
}
fn body_failure(error: Error) -> Failure {
    Failure {
        error,
        retry: Some(Duration::ZERO),
    }
}
fn unsupported(message: &str) -> Error {
    Error::UnsupportedManifest(message.into())
}

pub(crate) fn download(
    client: &Client,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    concurrency: usize,
    keep_fragments: bool,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    download_with(
        client,
        info,
        destination,
        options,
        concurrency,
        keep_fragments,
        Assembly::Raw,
        callback,
    )
}

/// How fragments are written.
#[derive(Clone, Copy)]
enum Assembly<'a> {
    /// Appended as received (HlsFD, DashSegmentsFD).
    Raw,
    /// `F4mFD`: the FLV header, then each fragment's `mdat` payload.
    F4m(&'a [u8]),
    /// `IsmFD`: the PIFF header (from these `_download_params` and the first
    /// fragment's track ID) before the raw fragments.
    Ism(&'a serde_json::Value),
}

/// Upstream `IsmFD`'s assembly, sequentially.
pub(crate) fn download_ism(
    client: &Client,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    keep_fragments: bool,
    params: &serde_json::Value,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    download_with(
        client,
        info,
        destination,
        options,
        1,
        keep_fragments,
        Assembly::Ism(params),
        callback,
    )
}

/// Upstream `F4mFD`'s assembly: the FLV `header` first (unless resuming),
/// then each fragment's `mdat` payload, sequentially.
pub(crate) fn download_f4m(
    client: &Client,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    keep_fragments: bool,
    header: &[u8],
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    download_with(
        client,
        info,
        destination,
        options,
        1,
        keep_fragments,
        Assembly::F4m(header),
        callback,
    )
}

#[allow(clippy::too_many_arguments)]
fn download_with(
    client: &Client,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    concurrency: usize,
    keep_fragments: bool,
    assembly: Assembly<'_>,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    crate::download::check_destination(destination, &options)?;
    // The extractor-argument queries and key apply to the requests.
    let adjusted = crate::manifest_args::download_fragments(info).map(|fragments| {
        let mut adjusted = info.clone();
        adjusted.fragments = fragments;
        adjusted
    });
    let info = adjusted.as_ref().unwrap_or(info);
    let external_key = crate::manifest_args::external_key(info)?;
    let initial_keys = || -> HashMap<url::Url, [u8; 16]> {
        external_key
            .map(|key| {
                info.fragments
                    .iter()
                    .filter_map(|fragment| fragment.encryption.as_ref())
                    .map(|encryption| (encryption.key_url.clone(), key))
                    .collect()
            })
            .unwrap_or_default()
    };
    if info.fragments.is_empty() || info.fragments.len() > 10_000 {
        return Err(unsupported("fragment count must be between 1 and 10000"));
    }
    for fragment in &info.fragments {
        if !matches!(fragment.url.scheme(), "http" | "https") {
            return Err(Error::UnsupportedScheme(fragment.url.scheme().into()));
        }
        if let Some(encryption) = &fragment.encryption
            && !matches!(encryption.key_url.scheme(), "http" | "https")
        {
            return Err(Error::UnsupportedScheme(encryption.key_url.scheme().into()));
        }
        if let Some(range) = &fragment.byte_range
            && (range.length == 0 || range.offset.checked_add(range.length).is_none())
        {
            return Err(Error::InvalidRangeResponse);
        }
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    // Upstream `_prepare_frag_download`: `FILE.part` (or the destination
    // with --no-part) and `FILE.ytdl` resume state; the library default
    // assembles a private temporary file instead.
    let (upstream, resume) = match options.part_files {
        crate::PartFiles::Private => (false, false),
        crate::PartFiles::Part { resume } | crate::PartFiles::NoPart { resume } => (true, resume),
    };
    let part_path = {
        let mut name = destination.as_os_str().to_owned();
        if matches!(options.part_files, crate::PartFiles::Part { .. }) {
            name.push(".part");
        }
        std::path::PathBuf::from(name)
    };
    let ytdl_path = {
        let mut name = destination.as_os_str().to_owned();
        name.push(".ytdl");
        std::path::PathBuf::from(name)
    };
    // HlsFD keeps an (empty) `extra_state`, IsmFD whether its header is
    // written; DashSegmentsFD and F4mFD have none.
    let hls = info
        .protocol
        .as_deref()
        .is_some_and(|protocol| protocol.starts_with("m3u8"));
    let ism = matches!(assembly, Assembly::Ism(_));
    let write_state = |index: usize| -> Result<()> {
        let extra = if ism {
            format!(
                ", \"extra_state\": {{\"ism_track_written\": {}}}",
                if index > 0 { "true" } else { "false" }
            )
        } else if hls {
            ", \"extra_state\": {}".to_owned()
        } else {
            String::new()
        };
        fs::write(
            &ytdl_path,
            format!("{{\"downloader\": {{\"current_fragment\": {{\"index\": {index}}}{extra}}}}}"),
        )?;
        Ok(())
    };
    let mut start = 0_usize;
    let mut private = None;
    let mut named = None;
    if upstream {
        let mut resume_len = fs::metadata(&part_path).map_or(0, |metadata| metadata.len());
        if resume && ytdl_path.is_file() {
            let state: Option<usize> = fs::read_to_string(&ytdl_path)
                .ok()
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                .and_then(|value| value["downloader"]["current_fragment"]["index"].as_u64())
                .map(|index| index as usize);
            let message = match state {
                None => Some(".ytdl file is corrupt"),
                Some(index) if index > 0 && resume_len == 0 => {
                    Some("Inconsistent state of incomplete fragment download")
                }
                Some(index) => {
                    start = index.min(info.fragments.len());
                    None
                }
            };
            if let Some(message) = message {
                crate::screen::warning(&format!("{message}. Restarting from the beginning ..."));
                start = 0;
                resume_len = 0;
                write_state(0)?;
            }
        } else {
            if !resume {
                resume_len = 0;
            }
            write_state(0)?;
        }
        // Not `append`: an append-only Windows handle cannot `sync_all`
        // ("Access is denied"); resumed writes start at the end instead.
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .read(true)
            .truncate(resume_len == 0)
            .open(&part_path)?;
        file.seek(SeekFrom::Start(resume_len))?;
        named = Some((file, resume_len));
    } else {
        private = Some(
            tempfile::Builder::new()
                .prefix(".yt-dlp-rs-fragments-")
                .suffix(".part")
                .tempfile_in(parent)?,
        );
    }
    let resumed_bytes = named.as_ref().map_or(0, |(_, length)| *length);
    let mut stored = resumed_bytes;
    let mut keys = initial_keys();
    // Upstream keeps the latest nonzero fragment filetime.
    let mut filetime = None;
    let mut reporter = Reporter {
        callback,
        attempt: 1,
        index: start as u32 + 1,
        count: info.fragments.len() as u32,
        rate_limit: options.rate_limit,
        fragment: None,
    };
    // Upstream `--keep-fragments`: `TMPFILENAME-FragN` beside the output.
    let keep = keep_fragments.then(|| part_path.as_os_str().to_owned());
    let keep_fragment = |index: usize, bytes: &mut dyn Read| -> Result<()> {
        if let Some(base) = &keep {
            let mut name = base.clone();
            name.push(format!("-Frag{}", index + 1));
            let mut file = fs::File::create(std::path::PathBuf::from(name))?;
            std::io::copy(bytes, &mut file)?;
        }
        Ok(())
    };
    // Completed fragments for `FILE.ytdl` (upstream counts downloads, not
    // positions).
    let mut completed = start;
    let mut appended = |_: usize| -> Result<()> {
        completed += 1;
        if upstream {
            write_state(completed)
        } else {
            Ok(())
        }
    };
    let output_path = private.as_ref().map_or_else(
        || part_path.clone(),
        |temporary| temporary.path().to_path_buf(),
    );
    {
        let output: &mut fs::File = match (&mut private, &mut named) {
            (Some(temporary), _) => temporary.as_file_mut(),
            (_, Some((file, _))) => file,
            _ => unreachable!("one output file is open"),
        };
        // `write_flv_header` (and the metadata tag) before any fragment.
        if let Assembly::F4m(header) = assembly
            && stored == 0
        {
            output.write_all(header)?;
            stored = header.len() as u64;
        }
        // IsmFD's header follows the first fragment's download.
        let mut track_written = start > 0;
        if concurrency > 1 && info.fragments.len() - start > 1 {
            let (written, latest) = download_concurrent(
                client,
                info,
                &options,
                concurrency,
                start,
                parent,
                output,
                &mut reporter,
                &keep_fragment,
                &mut appended,
            )?;
            stored += written;
            filetime = latest;
        } else {
            for (index, fragment) in info.fragments.iter().enumerate().skip(start) {
                reporter.index = index as u32 + 1;
                let prefix = stored;
                if let Some(modified) = fetch_with_retries(
                    client,
                    info,
                    index,
                    fragment,
                    &options,
                    &mut keys,
                    output,
                    prefix,
                    &mut stored,
                    &mut reporter,
                )? {
                    // HttpFD's `finished` status for the fragment.
                    reporter.fragment = Some(crate::FragmentBytes {
                        downloaded: stored - prefix,
                        total: Some(stored - prefix),
                        finished: true,
                    });
                    reporter.report(DownloadPhase::Downloading, stored)?;
                    filetime = crate::filetime::filetime(modified.as_deref()).or(filetime);
                    if keep.is_some() {
                        let mut file = fs::File::open(&output_path)?;
                        file.seek(SeekFrom::Start(prefix))?;
                        keep_fragment(index, &mut file.take(stored - prefix))?;
                    }
                    if let Assembly::Ism(params) = assembly
                        && !track_written
                    {
                        let mut raw = Vec::new();
                        let mut file = fs::File::open(&output_path)?;
                        file.seek(SeekFrom::Start(prefix))?;
                        file.take(stored - prefix).read_to_end(&mut raw)?;
                        let id = crate::ism_fd::track_id(&raw)?;
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |elapsed| elapsed.as_secs());
                        let header = crate::ism_fd::piff_header(params, id, now)?;
                        output.set_len(prefix)?;
                        output.seek(SeekFrom::Start(prefix))?;
                        output.write_all(&header)?;
                        output.write_all(&raw)?;
                        stored = prefix + (header.len() + raw.len()) as u64;
                        track_written = true;
                    }
                    // F4mFD appends only the fragment's `mdat` box data.
                    if matches!(assembly, Assembly::F4m(_)) {
                        let mut raw = Vec::new();
                        let mut file = fs::File::open(&output_path)?;
                        file.seek(SeekFrom::Start(prefix))?;
                        file.take(stored - prefix).read_to_end(&mut raw)?;
                        let payload = crate::f4m_fd::mdat(&raw)?;
                        output.set_len(prefix)?;
                        output.seek(SeekFrom::Start(prefix))?;
                        output.write_all(payload)?;
                        stored = prefix + payload.len() as u64;
                    }
                    appended(index)?;
                }
            }
        }
        output.flush()?;
        output.sync_all()?;
        if options.update_time {
            crate::filetime::apply(output, filetime);
        }
    }
    reporter.report(DownloadPhase::Publishing, stored)?;
    match private {
        Some(temporary) => crate::download::persist(temporary, destination, options.overwrite)?,
        None => {
            // The part file must be closed before it is renamed (Windows).
            drop(named);
            // `_finish_frag_download`: drop the state, then rename the part file.
            let _ = fs::remove_file(&ytdl_path);
            if part_path != destination {
                fs::rename(&part_path, destination)?;
            }
        }
    }
    Ok(stored)
}
/// One fragment with `--fragment-retries` (`Ok(None)` when skipped as
/// unavailable): written to `file` after `prefix`, which a failed attempt
/// truncates back to.
#[allow(clippy::too_many_arguments)]
fn fetch_with_retries(
    client: &Client,
    info: &VideoInfo,
    index: usize,
    fragment: &Fragment,
    options: &DownloadOptions,
    keys: &mut HashMap<url::Url, [u8; 16]>,
    file: &mut fs::File,
    prefix: u64,
    stored: &mut u64,
    reporter: &mut Reporter<'_>,
) -> Result<Option<Option<String>>> {
    for attempt in 0..=options.fragment_retries {
        file.set_len(prefix)?;
        file.seek(SeekFrom::Start(prefix))?;
        *stored = prefix;
        reporter.attempt = u16::try_from(attempt.saturating_add(1)).unwrap_or(u16::MAX);
        reporter.report(DownloadPhase::Requesting, *stored)?;
        let failure = match fetch(client, info, fragment, keys, file, stored, reporter) {
            Ok(modified) => return Ok(Some(modified)),
            Err(failure) => failure,
        };
        // Discard every byte from the failed fragment, including bytes
        // reported to observers, before waiting, retrying or skipping.
        file.set_len(prefix)?;
        file.seek(SeekFrom::Start(prefix))?;
        *stored = prefix;
        // Upstream retries every HTTP status error for fragments when
        // unavailable fragments may be skipped.
        let retry = failure.retry.or_else(|| {
            (options.skip_unavailable_fragments.is_some()
                && matches!(&failure.error, Error::Http(source) if source.status().is_some()))
            .then_some(Duration::ZERO)
        });
        if let Some(retry_after) = retry
            && attempt < options.fragment_retries
        {
            let delay = crate::retry::announce_retry(
                &failure.error,
                attempt,
                options.fragment_retries,
                Some(index as u32 + 1),
                options.retry_sleep.fragment,
            )
            .map_or_else(
                || crate::retry::delay(attempt, retry_after),
                |delay| delay.max(retry_after),
            );
            reporter.wait(delay, *stored)?;
            continue;
        }
        // Upstream `skip_unavailable_fragments`, never for the first one.
        if options.skip_unavailable_fragments == Some(true)
            && index > 0
            && !matches!(failure.error, Error::DownloadCancelled | Error::Io(_))
        {
            crate::screen::screen(&format!(
                "[download] fragment not found; Skipping fragment {} ...",
                index + 1
            ));
            return Ok(None);
        }
        return Err(if retry.is_some() {
            Error::FragmentUnavailable {
                index: index + 1,
                source: Box::new(crate::retry::exhausted(
                    failure.error,
                    options.fragment_retries,
                )),
            }
        } else {
            failure.error
        });
    }
    unreachable!("the final attempt returns or errors")
}

/// A fragment worker's report to the assembling thread.
enum Event {
    Progress {
        index: usize,
        bytes: u64,
    },
    Done {
        index: usize,
        result: Result<Option<(tempfile::NamedTempFile, Option<String>)>>,
    },
}

/// Upstream `--concurrent-fragments`: `concurrency` workers fetch fragments
/// (each with its own retries) into private files, appended in order.
/// Returns the stored length and the latest fragment filetime.
#[allow(clippy::too_many_arguments)]
fn download_concurrent(
    client: &Client,
    info: &VideoInfo,
    options: &DownloadOptions,
    concurrency: usize,
    start: usize,
    parent: &Path,
    output: &mut fs::File,
    reporter: &mut Reporter<'_>,
    keep_fragment: &dyn Fn(usize, &mut dyn Read) -> Result<()>,
    appended: &mut dyn FnMut(usize) -> Result<()>,
) -> Result<(u64, Option<i64>)> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    };
    let next = Arc::new(AtomicUsize::new(start));
    let cancelled = Arc::new(AtomicBool::new(false));
    let (sender, receiver) = mpsc::channel::<Event>();
    let count = info.fragments.len();
    std::thread::scope(|scope| -> Result<(u64, Option<i64>)> {
        for _ in 0..concurrency.min(count - start) {
            let (next, cancelled, sender) = (next.clone(), cancelled.clone(), sender.clone());
            scope.spawn(move || {
                let mut keys: HashMap<url::Url, [u8; 16]> =
                    match crate::manifest_args::external_key(info).ok().flatten() {
                        Some(key) => info
                            .fragments
                            .iter()
                            .filter_map(|fragment| fragment.encryption.as_ref())
                            .map(|encryption| (encryption.key_url.clone(), key))
                            .collect(),
                        None => HashMap::new(),
                    };
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= count || cancelled.load(Ordering::Relaxed) {
                        return;
                    }
                    let progress = sender.clone();
                    let flag = cancelled.clone();
                    let mut forward = move |event: DownloadProgress| {
                        let _ = progress.send(Event::Progress {
                            index,
                            bytes: event.downloaded_bytes,
                        });
                        if flag.load(Ordering::Relaxed) {
                            ControlFlow::Break(())
                        } else {
                            ControlFlow::Continue(())
                        }
                    };
                    let mut worker = Reporter {
                        callback: &mut forward,
                        attempt: 1,
                        index: index as u32 + 1,
                        count: count as u32,
                        rate_limit: options.rate_limit,
                        fragment: None,
                    };
                    let result = (|| {
                        let mut file = tempfile::Builder::new()
                            .prefix(".yt-dlp-rs-fragment-")
                            .tempfile_in(parent)?;
                        let mut stored = 0;
                        let outcome = fetch_with_retries(
                            client,
                            info,
                            index,
                            &info.fragments[index],
                            options,
                            &mut keys,
                            file.as_file_mut(),
                            0,
                            &mut stored,
                            &mut worker,
                        )?;
                        Ok(outcome.map(|modified| (file, modified)))
                    })();
                    let failed = result.is_err();
                    let _ = sender.send(Event::Done { index, result });
                    if failed {
                        return;
                    }
                }
            });
        }
        drop(sender);
        // Assemble in order while reporting the in-flight total.
        let mut finished: HashMap<usize, Option<(tempfile::NamedTempFile, Option<String>)>> =
            HashMap::new();
        let mut in_flight: HashMap<usize, u64> = HashMap::new();
        let (mut written, mut done, mut cursor) = (0_u64, start, start);
        let mut latest = None;
        let mut outcome = Ok(());
        for event in receiver {
            match event {
                Event::Progress { index, bytes } => {
                    in_flight.insert(index, bytes);
                }
                Event::Done { index, result } => match result {
                    Ok(file) => {
                        in_flight.remove(&index);
                        finished.insert(index, file);
                        done += 1;
                    }
                    Err(error) => {
                        cancelled.store(true, Ordering::Relaxed);
                        if outcome.is_ok() {
                            outcome = Err(error);
                        }
                    }
                },
            }
            while let Some(file) = finished.remove(&cursor) {
                if let Some((file, modified)) = file {
                    let mut source = file.reopen()?;
                    written += std::io::copy(&mut source, output)?;
                    keep_fragment(cursor, &mut file.reopen()?)?;
                    appended(cursor)?;
                    latest = crate::filetime::filetime(modified.as_deref()).or(latest);
                }
                cursor += 1;
            }
            if outcome.is_ok() {
                let pending: u64 = finished
                    .values()
                    .flatten()
                    .map(|(file, _)| file.as_file().metadata().map_or(0, |m| m.len()))
                    .sum();
                reporter.index = (done + 1).min(count) as u32;
                reporter.attempt = 1;
                let total = written + pending + in_flight.values().sum::<u64>();
                if reporter.report(DownloadPhase::Downloading, total).is_err() {
                    cancelled.store(true, Ordering::Relaxed);
                    outcome = Err(Error::DownloadCancelled);
                }
            }
        }
        outcome?;
        Ok((written, latest))
    })
}

fn fetch(
    client: &Client,
    info: &VideoInfo,
    fragment: &Fragment,
    keys: &mut HashMap<url::Url, [u8; 16]>,
    file: &mut fs::File,
    stored: &mut u64,
    reporter: &mut Reporter<'_>,
) -> std::result::Result<Option<String>, Failure> {
    let decryption = if let Some(encryption) = &fragment.encryption {
        let key = match keys.get(&encryption.key_url) {
            Some(key) => *key,
            None => {
                let key = fetch_key(client, info, &encryption.key_url, *stored, reporter)?;
                keys.insert(encryption.key_url.clone(), key);
                key
            }
        };
        reporter.report(DownloadPhase::Requesting, *stored)?;
        Some(crate::crypto::Decryptor::new(&key, &encryption.iv))
    } else {
        None
    };
    let mut request = client
        .get(fragment.url.clone())
        .header(ACCEPT_ENCODING, "identity");
    if !info.is_direct() {
        request = request.header(REFERER, info.webpage_url.as_str());
    }
    if let Some(range) = &fragment.byte_range {
        request = request.header(
            RANGE,
            format!("bytes={}-{}", range.offset, range.offset + range.length - 1),
        );
    }
    let mut response = request
        .send()
        .inspect(|_| crate::screen::after_request())
        .map_err(Error::Http)
        .and_then(crate::retry::check_response)
        .map_err(|error| Failure {
            retry: crate::retry::failure_delay(&error),
            error,
        })?;
    let expected = validate(&response, fragment, false)?;
    let modified = crate::filetime::header(&response);
    reporter.report(DownloadPhase::Downloading, *stored)?;
    if let Some(decryptor) = decryption {
        if expected.is_some_and(|length| length == 0 || length % 16 != 0) {
            return Err(
                unsupported("AES-128 ciphertext length must be a positive multiple of 16").into(),
            );
        }
        return decrypt_response(response, expected, decryptor, file, stored, reporter)
            .map(|()| modified);
    }
    let mut received = 0_u64;
    // HttpFD's blocks; enough leading bytes are buffered to recognize split
    // HTML/manifest responses before anything is written.
    let mut limiter = crate::download::RateLimit::new(reporter.rate_limit);
    let mut blocks = crate::download::Blocks::new();
    let mut pending: Option<Vec<u8>> = Some(Vec::new());
    loop {
        let block = blocks
            .read(&mut response, None)
            .map_err(|error| body_failure(Error::Io(error)))?
            .to_vec();
        let count = block.len();
        limiter.consume(count);
        blocks.finished(count);
        let data = match &mut pending {
            Some(buffered) => {
                buffered.extend_from_slice(&block);
                if buffered.len() < 512 && count > 0 {
                    continue;
                }
                sniff(&buffered[..buffered.len().min(512)])?;
                pending.take().unwrap()
            }
            None if count == 0 => break,
            None => block,
        };
        if !data.is_empty() {
            reporter.fragment = Some(crate::FragmentBytes {
                downloaded: received + data.len() as u64,
                total: expected,
                finished: false,
            });
            append(file, &data, stored, &mut received, expected, reporter)?;
        }
        if count == 0 {
            break;
        }
    }
    if let Some(expected) = expected
        && received != expected
    {
        return Err(body_failure(Error::IncompleteDownload {
            expected,
            actual: received,
        }));
    }
    if received == 0 {
        return Err(unsupported("empty media fragment").into());
    }
    Ok(modified)
}
fn append(
    file: &mut fs::File,
    bytes: &[u8],
    stored: &mut u64,
    received: &mut u64,
    expected: Option<u64>,
    reporter: &mut Reporter<'_>,
) -> std::result::Result<(), Failure> {
    let next = received
        .checked_add(bytes.len() as u64)
        .ok_or(Error::InvalidRangeResponse)?;
    if expected.is_some_and(|expected| next > expected) {
        return Err(Error::InvalidRangeResponse.into());
    }
    let next_stored = stored
        .checked_add(bytes.len() as u64)
        .ok_or(Error::InvalidRangeResponse)?;
    file.write_all(bytes)?;
    *received = next;
    *stored = next_stored;
    reporter.report(DownloadPhase::Downloading, *stored)?;
    Ok(())
}
fn validate(response: &Response, fragment: &Fragment, key: bool) -> Result<Option<u64>> {
    let encoding: Vec<_> = response
        .headers()
        .get_all(CONTENT_ENCODING)
        .iter()
        .collect();
    if encoding.len() > 1
        || encoding.first().is_some_and(|value| {
            value
                .to_str()
                .map_or(true, |value| !value.eq_ignore_ascii_case("identity"))
        })
    {
        return Err(unsupported("encoded fragment response"));
    }
    let types: Vec<_> = response.headers().get_all(CONTENT_TYPE).iter().collect();
    if types.len() > 1 {
        return Err(unsupported("ambiguous fragment content type"));
    }
    if let Some(value) = types.first() {
        let mime = value
            .to_str()
            .map_err(|_| unsupported("invalid fragment content type"))?
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if mime.contains("mpegurl") || mime == "application/dash+xml" {
            return Err(unsupported("fragment response is another manifest"));
        }
        if key
            && matches!(
                mime.as_str(),
                "text/html" | "application/xhtml+xml" | "application/xml" | "text/xml"
            )
        {
            return Err(unsupported(
                "HLS AES-128 key response is HTML or a manifest",
            ));
        }
        if !key
            && !(mime.starts_with("video/")
                || mime.starts_with("audio/")
                || matches!(
                    mime.as_str(),
                    "application/octet-stream" | "application/mp4" | "application/ogg"
                ))
        {
            return Err(unsupported("fragment response is not media"));
        }
    }
    if let Some(range) = &fragment.byte_range {
        if response.status() != reqwest::StatusCode::PARTIAL_CONTENT
            || response.headers().get_all(CONTENT_RANGE).iter().count() != 1
        {
            return Err(Error::InvalidRangeResponse);
        }
        let value = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .ok_or(Error::InvalidRangeResponse)?;
        let (span, total) = value
            .strip_prefix("bytes ")
            .and_then(|value| value.split_once('/'))
            .ok_or(Error::InvalidRangeResponse)?;
        let (start, end) = span.split_once('-').ok_or(Error::InvalidRangeResponse)?;
        let number = |s: &str| {
            if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
                s.parse::<u64>().ok()
            } else {
                None
            }
        };
        let end_expected = range.offset + range.length - 1;
        if number(start) != Some(range.offset)
            || number(end) != Some(end_expected)
            || !number(total).is_some_and(|total| total > end_expected)
            || response
                .content_length()
                .is_some_and(|length| length != range.length)
        {
            return Err(Error::InvalidRangeResponse);
        }
        Ok(Some(range.length))
    } else {
        if response.status() == reqwest::StatusCode::PARTIAL_CONTENT
            || response.headers().contains_key(CONTENT_RANGE)
        {
            return Err(Error::PartialResponse);
        }
        Ok(response.content_length())
    }
}

fn sniff(prefix: &[u8]) -> Result<()> {
    let text = String::from_utf8_lossy(prefix)
        .trim_start_matches('\u{feff}')
        .trim_start()
        .to_ascii_lowercase();
    if text.starts_with("<!doctype html")
        || text.starts_with("<html")
        || text.starts_with("#extm3u")
        || text.starts_with("<?xml")
        || text.starts_with("<mpd")
    {
        return Err(unsupported("fragment returned HTML or another manifest"));
    }
    Ok(())
}

fn fetch_key(
    client: &Client,
    info: &VideoInfo,
    url: &url::Url,
    stored: u64,
    reporter: &mut Reporter<'_>,
) -> std::result::Result<[u8; 16], Failure> {
    reporter.report(DownloadPhase::Requesting, stored)?;
    let mut request = client.get(url.clone()).header(ACCEPT_ENCODING, "identity");
    if !info.is_direct() {
        request = request.header(REFERER, info.webpage_url.as_str());
    }
    let mut response = request
        .send()
        .inspect(|_| crate::screen::after_request())
        .map_err(Error::Http)
        .and_then(crate::retry::check_response)
        .map_err(|error| Failure {
            retry: crate::retry::failure_delay(&error),
            error,
        })?;
    let key_fragment = Fragment {
        url: url.clone(),
        byte_range: None,
        encryption: None,
        path: None,
        duration: None,
    };
    let expected = validate(&response, &key_fragment, true)?;
    if expected.is_some_and(|length| length != 16) {
        return Err(unsupported("HLS AES-128 key must contain exactly 16 bytes").into());
    }
    let mut bytes = [0_u8; 17];
    let mut received = 0;
    while received < bytes.len() {
        reporter.report(DownloadPhase::Downloading, stored)?;
        let count = match response.read(&mut bytes[received..]) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(body_failure(Error::Io(error))),
        };
        received += count;
    }
    if received != 16 {
        return Err(unsupported("HLS AES-128 key must contain exactly 16 bytes").into());
    }
    let mut key = [0; 16];
    key.copy_from_slice(&bytes[..16]);
    Ok(key)
}

fn decrypt_response(
    mut response: Response,
    expected: Option<u64>,
    mut decryptor: crate::crypto::Decryptor,
    file: &mut fs::File,
    stored: &mut u64,
    reporter: &mut Reporter<'_>,
) -> std::result::Result<(), Failure> {
    let mut cipher_bytes = 0_u64;
    let mut plain_bytes = 0_u64;
    let mut prefix = Vec::with_capacity(512);
    let mut sniffed = false;
    let mut buffer = [0; 32 * 1024];
    let mut limiter = crate::download::RateLimit::new(reporter.rate_limit);
    loop {
        reporter.report(DownloadPhase::Downloading, *stored)?;
        let count = match response.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(body_failure(Error::Io(error))),
        };
        limiter.consume(count);
        cipher_bytes = cipher_bytes
            .checked_add(count as u64)
            .ok_or(Error::InvalidRangeResponse)?;
        if expected.is_some_and(|expected| cipher_bytes > expected) {
            return Err(Error::InvalidRangeResponse.into());
        }
        let plain = decryptor.update(&buffer[..count]);
        write_plain(
            &plain,
            &mut prefix,
            &mut sniffed,
            file,
            stored,
            &mut plain_bytes,
            reporter,
        )?;
    }
    if let Some(expected) = expected
        && cipher_bytes != expected
    {
        return Err(body_failure(Error::IncompleteDownload {
            expected,
            actual: cipher_bytes,
        }));
    }
    let final_plain = decryptor.finish()?;
    write_plain(
        &final_plain,
        &mut prefix,
        &mut sniffed,
        file,
        stored,
        &mut plain_bytes,
        reporter,
    )?;
    if !sniffed {
        sniff(&prefix)?;
        append(file, &prefix, stored, &mut plain_bytes, None, reporter)?;
    }
    if plain_bytes == 0 {
        return Err(unsupported("empty media fragment").into());
    }
    Ok(())
}

fn write_plain(
    bytes: &[u8],
    prefix: &mut Vec<u8>,
    sniffed: &mut bool,
    file: &mut fs::File,
    stored: &mut u64,
    plain_bytes: &mut u64,
    reporter: &mut Reporter<'_>,
) -> std::result::Result<(), Failure> {
    if *sniffed {
        return append(file, bytes, stored, plain_bytes, None, reporter);
    }
    let take = bytes.len().min(512 - prefix.len());
    prefix.extend_from_slice(&bytes[..take]);
    if prefix.len() == 512 {
        sniff(prefix)?;
        append(file, prefix, stored, plain_bytes, None, reporter)?;
        prefix.clear();
        *sniffed = true;
        append(file, &bytes[take..], stored, plain_bytes, None, reporter)?;
    }
    Ok(())
}

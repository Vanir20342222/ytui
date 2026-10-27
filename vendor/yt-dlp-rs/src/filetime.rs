//! File timestamps like upstream: `--mtime` from `Last-Modified`
//! (`FileDownloader.try_utime`) and FFmpeg outputs inheriting the oldest
//! input modification time (`run_ffmpeg_multiple_files`). Failures are
//! ignored, as upstream suppresses or merely warns about them.

use std::{
    fs,
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// Python `Last-Modified` header text (Latin-1 decoded).
pub(crate) fn header(response: &reqwest::blocking::Response) -> Option<String> {
    let value = response.headers().get(reqwest::header::LAST_MODIFIED)?;
    Some(value.as_bytes().iter().map(|&b| char::from(b)).collect())
}

/// `timeconvert(header)`, ignoring the epoch itself like `try_utime`.
pub(crate) fn filetime(header: Option<&str>) -> Option<i64> {
    crate::dates::timeconvert(header?).filter(|&time| time != 0)
}

fn system_time(seconds: i64) -> Option<SystemTime> {
    if seconds >= 0 {
        UNIX_EPOCH.checked_add(Duration::from_secs(seconds as u64))
    } else {
        UNIX_EPOCH.checked_sub(Duration::from_secs(seconds.unsigned_abs()))
    }
}

fn set(file: &fs::File, accessed: SystemTime, modified: SystemTime) {
    let _ = file.set_times(
        fs::FileTimes::new()
            .set_accessed(accessed)
            .set_modified(modified),
    );
}

/// `os.utime(file, (time.time(), filetime))`.
pub(crate) fn apply(file: &fs::File, filetime: Option<i64>) {
    if let Some(modified) = filetime.and_then(system_time) {
        set(file, SystemTime::now(), modified);
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// The oldest modification time among `inputs`.
pub(crate) fn oldest<'a>(inputs: impl IntoIterator<Item = &'a Path>) -> Option<SystemTime> {
    inputs.into_iter().filter_map(modified).min()
}

/// `try_utime(output, oldest, oldest)` after an FFmpeg run.
pub(crate) fn inherit(output: &Path, oldest: Option<SystemTime>) {
    if let (Some(time), Ok(file)) = (oldest, fs::OpenOptions::new().write(true).open(output)) {
        set(&file, time, time);
    }
}

//! Persistent UTF-8 archive identities with bounded cooperating-process locks.
use crate::{Error, Result, VideoInfo};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions, TryLockError},
    io::{BufRead, BufReader, Seek, SeekFrom, Write},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const LOCK_POLL: Duration = Duration::from_millis(25);

/// A local archive cache. Membership reflects loading and this instance's records;
/// every record re-reads the same locked file to deduplicate concurrent writers.
#[derive(Debug)]
pub struct DownloadArchive {
    path: PathBuf,
    entries: HashSet<String>,
    lock_timeout: Duration,
}

/// The supported identity is exactly lowercase extractor key, one space, and ID.
/// This native subset requires explicit, nonempty metadata and never guesses an
/// extractor from a URL; an info's `_old_archive_ids` also match.
pub fn archive_id(info: &VideoInfo) -> Result<String> {
    for (name, value) in [("extractor_key", &info.extractor_key), ("id", &info.id)] {
        if value.is_empty()
            || value.trim_matches(python_whitespace) != value
            || value.chars().any(char::is_control)
        {
            return Err(Error::Archive(format!(
                "{name} must be nonempty and have no controls or surrounding whitespace"
            )));
        }
    }
    Ok(format!("{} {}", info.extractor_key.to_lowercase(), info.id))
}
impl DownloadArchive {
    /// Load a regular UTF-8 file under a shared lock. Missing files are empty and
    /// are not created until recording. Existing lines are cached using strip
    /// semantics, including unknown records and comments without interpretation.
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        Self::load_with_timeout(path.into(), LOCK_TIMEOUT)
    }
    fn load_with_timeout(path: PathBuf, lock_timeout: Duration) -> Result<Self> {
        let entries = if regular_path(&path)? {
            match File::open(&path) {
                Ok(file) => {
                    regular_file(&file, &path)?;
                    let mut file = LockedFile::acquire(file, false, lock_timeout, &path)?;
                    read(&mut file, &path)?.0
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => HashSet::new(),
                Err(error) => return Err(io_error("open", &path, error)),
            }
        } else {
            HashSet::new()
        };
        Ok(Self {
            path,
            entries,
            lock_timeout,
        })
    }
    /// Upstream `in_download_archive`: the identity or any of the info's
    /// `_old_archive_ids`.
    pub fn contains(&self, info: &VideoInfo) -> Result<bool> {
        if self.entries.contains(&archive_id(info)?) {
            return Ok(true);
        }
        Ok(info
            .extra
            .get("_old_archive_ids")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|ids| {
                ids.iter()
                    .filter_map(serde_json::Value::as_str)
                    .any(|id| self.entries.contains(id))
            }))
    }
    /// Append at most one LF-terminated identity under an exclusive lock and
    /// synchronize it before updating membership. Existing bytes are preserved;
    /// unlike upstream's raw append, a missing final CR/LF gets an LF separator.
    /// Parent directories are never created. A failed append is rolled back to
    /// the length observed under the lock; rollback failures are also reported.
    /// Successful media publication is not rolled back.
    pub fn record(&mut self, info: &VideoInfo) -> Result<()> {
        let identity = archive_id(info)?;
        crate::screen::debug(&format!("Adding to archive: {identity}"));
        regular_path(&self.path)?;
        let file = open_record(&self.path)
            .map_err(|error| io_error("open for append", &self.path, error))?;
        regular_file(&file, &self.path)?;
        let mut file = LockedFile::acquire(file, true, self.lock_timeout, &self.path)?;
        let (entries, separator) = read(&mut file, &self.path)?;
        self.entries = entries;
        if self.entries.contains(&identity) {
            return Ok(());
        }
        let mut append = String::with_capacity(identity.len() + 2);
        if separator {
            append.push('\n');
        }
        append.push_str(&identity);
        append.push('\n');
        append_transaction(&mut file, &self.path, append.as_bytes(), |file, bytes| {
            file.write_all(bytes)?;
            file.sync_data()
        })?;
        self.entries.insert(identity);
        Ok(())
    }
}
// Closing one descriptor does not necessarily release a Unix flock: a descriptor
// duplicated by try_clone or inherited briefly across fork/exec shares its open
// file description. Explicitly unlock before closing on every return path.
struct LockedFile(File);

impl LockedFile {
    fn acquire(file: File, exclusive: bool, timeout: Duration, path: &Path) -> Result<Self> {
        lock(&file, exclusive, timeout, path)?;
        Ok(Self(file))
    }
}

impl Deref for LockedFile {
    type Target = File;

    fn deref(&self) -> &File {
        &self.0
    }
}

impl DerefMut for LockedFile {
    fn deref_mut(&mut self) -> &mut File {
        &mut self.0
    }
}

impl Drop for LockedFile {
    fn drop(&mut self) {
        // Drop must also run while unwinding an I/O error. Closing remains the
        // fallback if the OS rejects unlocking; there is no fallible Drop API.
        let _ = self.0.unlock();
    }
}

fn open_record(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).create(true);
    // Rust's Windows append-only access excludes FILE_WRITE_DATA, which is
    // required to restore the original length after an interrupted append.
    // The exclusive lock plus the explicit EOF seek below provides appending.
    #[cfg(windows)]
    options.write(true).truncate(false);
    #[cfg(not(windows))]
    options.append(true);
    options.open(path)
}

fn append_transaction(
    file: &mut File,
    path: &Path,
    bytes: &[u8],
    write: impl FnOnce(&mut File, &[u8]) -> std::io::Result<()>,
) -> Result<()> {
    let original_len = file
        .metadata()
        .map_err(|error| io_error("inspect before append", path, error))?
        .len();
    file.seek(SeekFrom::End(0))
        .map_err(|error| io_error("seek before append", path, error))?;
    if let Err(error) = write(file, bytes) {
        let rollback = file.set_len(original_len).and_then(|()| file.sync_data());
        return Err(match rollback {
            Ok(()) => io_error("append/synchronize (append rolled back)", path, error),
            Err(rollback) => Error::Archive(format!(
                "cannot append/synchronize archive {path:?}: {error}; cannot roll back append: {rollback}"
            )),
        });
    }
    Ok(())
}

fn io_error(action: &str, path: &Path, error: std::io::Error) -> Error {
    Error::Archive(format!("cannot {action} archive {path:?}: {error}"))
}
fn regular_path(path: &Path) -> Result<bool> {
    if path.as_os_str().is_empty() {
        return Err(Error::Archive("archive path is empty".into()));
    }
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => Err(Error::Archive(format!(
            "archive {path:?} is not a regular file"
        ))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            validate_missing_ancestors(path)?;
            Ok(false)
        }
        Err(error) => Err(io_error("inspect", path, error)),
    }
}
// Windows can report NotFound rather than NotADirectory for file/child.
// Distinguish a genuinely missing path from an existing non-directory ancestor
// without creating directories or rejecting ordinary missing parent directories.
fn validate_missing_ancestors(path: &Path) -> Result<()> {
    for ancestor in path.ancestors().skip(1) {
        let ancestor = if ancestor.as_os_str().is_empty() {
            Path::new(".")
        } else {
            ancestor
        };
        match fs::metadata(ancestor) {
            Ok(metadata) if metadata.is_dir() => return Ok(()),
            Ok(_) => {
                return Err(Error::Archive(format!(
                    "archive {path:?} has a non-directory ancestor {ancestor:?}"
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(io_error("inspect ancestor", ancestor, error)),
        }
    }
    Ok(())
}

fn regular_file(file: &File, path: &Path) -> Result<()> {
    if !file
        .metadata()
        .map_err(|error| io_error("inspect opened", path, error))?
        .is_file()
    {
        return Err(Error::Archive(format!(
            "archive {path:?} is not a regular file"
        )));
    }
    Ok(())
}
fn lock(file: &File, exclusive: bool, timeout: Duration, path: &Path) -> Result<()> {
    let started = Instant::now();
    loop {
        match if exclusive {
            file.try_lock()
        } else {
            file.try_lock_shared()
        } {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) => {
                let remaining = timeout.saturating_sub(started.elapsed());
                if remaining.is_zero() {
                    return Err(Error::Archive(format!(
                        "timed out waiting for archive lock {path:?}"
                    )));
                }
                std::thread::sleep(remaining.min(LOCK_POLL));
            }
            Err(TryLockError::Error(error)) => return Err(io_error("lock", path, error)),
        }
    }
}
fn python_whitespace(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{1c}'..='\u{1f}')
}
fn read(file: &mut File, path: &Path) -> Result<(HashSet<String>, bool)> {
    file.seek(SeekFrom::Start(0))
        .map_err(|error| io_error("seek", path, error))?;
    let mut reader = BufReader::new(file);
    let mut entries = HashSet::new();
    let mut line = String::new();
    let mut last = None;
    loop {
        line.clear();
        if reader
            .read_line(&mut line)
            .map_err(|error| io_error("read UTF-8", path, error))?
            == 0
        {
            break;
        }
        last = line.as_bytes().last().copied();
        // Python's text-mode universal newlines also recognize lone CR.
        for record in line.split('\r') {
            let record = record.trim_matches(python_whitespace);
            if !record.is_empty() {
                entries.insert(record.into());
            }
        }
    }
    Ok((
        entries,
        last.is_some_and(|last| !matches!(last, b'\r' | b'\n')),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{
        io::Read,
        sync::{Arc, Barrier},
        thread,
    };
    fn info(key: &str, id: &str) -> VideoInfo {
        serde_json::from_value(json!({"id":id,"title":"Archive fixture","extractor":"ignored","extractor_key":key,"url":"https://example.invalid/source.mp4","webpage_url":"https://example.invalid/watch","ext":"mp4","direct":false})).unwrap()
    }
    fn open(path: &Path) -> File {
        open_record(path).unwrap()
    }
    fn locked_bytes(file: &mut File) -> Vec<u8> {
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        bytes
    }
    #[test]
    fn identity_is_explicit_lowercase_key_and_case_sensitive_id_without_url_guessing() {
        assert_eq!(
            archive_id(&info("YouTube", "CaseID")).unwrap(),
            "youtube CaseID"
        );
        assert_eq!(
            archive_id(&info("ΟΣ", "Épisode 影像 / literal")).unwrap(),
            "ος Épisode 影像 / literal"
        );
        for (key, id) in [
            ("", "id"),
            ("Test", ""),
            (" Test", "id"),
            ("Test ", "id"),
            ("Test", " id"),
            ("Test", "id\n"),
            ("Te\x1bst", "id"),
            ("Test", "id\tmore"),
            ("Test", "id\u{85}more"),
        ] {
            assert!(
                matches!(archive_id(&info(key, id)), Err(Error::Archive(_))),
                "{key:?}/{id:?}"
            );
        }
    }
    #[test]
    fn missing_archive_is_not_created_by_loading_membership_or_invalid_record() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive.txt");
        let mut archive = DownloadArchive::load(path.clone()).unwrap();
        assert!(!archive.contains(&info("Youtube", "id")).unwrap());
        assert!(!path.exists());
        assert!(archive.record(&info("", "id")).is_err());
        assert!(!path.exists());
        archive.record(&info("Youtube", "id")).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"youtube id\n");
    }
    #[test]
    fn utf8_strip_and_universal_newline_loading_preserve_every_original_byte() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive");
        let contents = "  youtube CaseID \r\n\r\n# ordinary unknown comment\nlegacy unrelated record\r\u{1c}custom Épisode 影像\u{1f}\rplain other\n";
        fs::write(&path, contents).unwrap();
        let mut archive = DownloadArchive::load(path.clone()).unwrap();
        assert!(archive.contains(&info("YouTube", "CaseID")).unwrap());
        assert!(!archive.contains(&info("YouTube", "caseid")).unwrap());
        assert!(archive.contains(&info("Custom", "Épisode 影像")).unwrap());
        archive.record(&info("YouTube", "CaseID")).unwrap();
        assert_eq!(fs::read(&path).unwrap(), contents.as_bytes());
        archive.record(&info("New", "id")).unwrap();
        assert_eq!(
            fs::read(&path).unwrap(),
            format!("{contents}new id\n").as_bytes()
        );
    }
    #[test]
    fn append_repairs_missing_newline_without_rewriting_old_data() {
        let directory = tempfile::tempdir().unwrap();
        for (before, expected) in [
            ("unknown previous", "unknown previous\ntest id\n"),
            ("unknown previous\r", "unknown previous\rtest id\n"),
            ("unknown previous\r\n", "unknown previous\r\ntest id\n"),
            ("", "test id\n"),
        ] {
            let path = directory.path().join("archive");
            fs::write(&path, before).unwrap();
            let mut archive = DownloadArchive::load(path.clone()).unwrap();
            archive.record(&info("Test", "id")).unwrap();
            archive.record(&info("Test", "id")).unwrap();
            assert_eq!(fs::read_to_string(path).unwrap(), expected);
        }
    }
    #[test]
    fn stale_instances_reread_locked_file_and_do_not_duplicate_existing_records() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive");
        let mut first = DownloadArchive::load(path.clone()).unwrap();
        let mut second = DownloadArchive::load(path.clone()).unwrap();
        first.record(&info("Test", "one")).unwrap();
        assert!(!second.contains(&info("Test", "one")).unwrap());
        second.record(&info("Test", "one")).unwrap();
        assert!(second.contains(&info("Test", "one")).unwrap());
        second.record(&info("Test", "two")).unwrap();
        first.record(&info("Test", "two")).unwrap();
        assert!(first.contains(&info("Test", "two")).unwrap());
        assert_eq!(fs::read_to_string(path).unwrap(), "test one\ntest two\n");
    }
    #[test]
    fn concurrent_cooperating_thread_writers_share_records_and_deduplicate_same_id() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive");
        let barrier = Arc::new(Barrier::new(9));
        let workers: Vec<_> = (0..8)
            .map(|index| {
                let path = path.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    let mut archive = DownloadArchive::load(path).unwrap();
                    barrier.wait();
                    archive.record(&info("Test", "shared")).unwrap();
                    archive
                        .record(&info("Test", &format!("unique-{index}")))
                        .unwrap();
                    archive.record(&info("Test", "shared")).unwrap();
                })
            })
            .collect();
        barrier.wait();
        for worker in workers {
            worker.join().unwrap();
        }
        let contents = fs::read_to_string(&path).unwrap();
        assert_eq!(contents.lines().count(), 9);
        assert_eq!(
            contents
                .lines()
                .filter(|line| *line == "test shared")
                .count(),
            1
        );
        let archive = DownloadArchive::load(path).unwrap();
        for index in 0..8 {
            assert!(
                archive
                    .contains(&info("Test", &format!("unique-{index}")))
                    .unwrap()
            );
        }
    }
    #[test]
    fn locks_have_a_bounded_deadline_and_are_released_on_success_and_error() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive");
        fs::write(&path, "test original\n").unwrap();
        let mut archive =
            DownloadArchive::load_with_timeout(path.clone(), Duration::from_millis(20)).unwrap();
        let mut held = open(&path);
        held.try_lock().unwrap();
        let started = Instant::now();
        assert!(
            matches!(DownloadArchive::load_with_timeout(path.clone(), Duration::from_millis(20)), Err(Error::Archive(message)) if message.contains("timed out"))
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(archive.record(&info("Test", "blocked")).is_err());
        assert!(!archive.contains(&info("Test", "blocked")).unwrap());
        assert_eq!(locked_bytes(&mut held), b"test original\n");
        held.unlock().unwrap();
        archive.record(&info("Test", "released")).unwrap();
        held.try_lock_shared().unwrap();
        assert!(
            DownloadArchive::load_with_timeout(path.clone(), Duration::from_millis(20)).is_ok()
        );
        held.unlock().unwrap();
        fs::write(&path, [0xff]).unwrap();
        assert!(DownloadArchive::load(&path).is_err());
        held.try_lock().unwrap();
        held.unlock().unwrap();
    }
    #[cfg(unix)]
    #[test]
    fn explicit_unlock_releases_lock_even_when_a_duplicate_descriptor_survives() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive");
        fs::write(&path, "test original\n").unwrap();
        for exclusive in [false, true] {
            // A duplicated descriptor shares the flock, like a descriptor
            // inherited by another thread's fork before the child execs.
            let file = LockedFile::acquire(open(&path), exclusive, LOCK_TIMEOUT, &path).unwrap();
            let duplicate = file.try_clone().unwrap();
            let contender = open(&path);
            assert!(matches!(
                contender.try_lock(),
                Err(TryLockError::WouldBlock)
            ));
            drop(file);
            contender.try_lock().unwrap();
            contender.unlock().unwrap();
            drop(duplicate);
        }
        // The same guarantee applies to an early error while holding the lock.
        fs::write(&path, [0xff]).unwrap();
        let mut file = LockedFile::acquire(open(&path), true, LOCK_TIMEOUT, &path).unwrap();
        let duplicate = file.try_clone().unwrap();
        let result = (|| -> Result<()> {
            read(&mut file, &path)?;
            Ok(())
        })();
        assert!(result.is_err());
        drop(file);
        let contender = open(&path);
        contender.try_lock().unwrap();
        contender.unlock().unwrap();
        drop(duplicate);
    }

    #[test]
    fn partial_write_and_failed_sync_roll_back_only_our_append_under_lock() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive");
        for prefix in [b"existing without LF".as_slice(), b"existing\r\n"] {
            fs::write(&path, prefix).unwrap();
            let mut file = open(&path);
            file.try_lock().unwrap();
            for complete in [false, true] {
                let error = append_transaction(
                    &mut file,
                    &path,
                    b"\ntest never-recorded\n",
                    |file, bytes| {
                        file.write_all(if complete { bytes } else { &bytes[..8] })?;
                        Err(std::io::Error::other("injected write/sync failure"))
                    },
                )
                .unwrap_err();
                assert!(error.to_string().contains("rolled back"));
                assert_eq!(locked_bytes(&mut file), prefix);
            }
            file.unlock().unwrap();
            let mut archive = DownloadArchive::load(path.clone()).unwrap();
            assert!(!archive.contains(&info("Test", "never-recorded")).unwrap());
            archive.record(&info("Test", "safe")).unwrap();
            let expected = if prefix.ends_with(b"\n") {
                [prefix, b"test safe\n"].concat()
            } else {
                [prefix, b"\ntest safe\n"].concat()
            };
            assert_eq!(fs::read(&path).unwrap(), expected);
        }
    }
    #[test]
    fn rollback_failure_reports_both_the_write_and_recovery_errors() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive");
        fs::write(&path, b"original\n").unwrap();
        let mut readonly = File::open(&path).unwrap();
        let error = append_transaction(&mut readonly, &path, b"unused", |_, _| {
            Err(std::io::Error::other("injected original failure"))
        })
        .unwrap_err();
        assert!(error.to_string().contains("injected original failure"));
        assert!(error.to_string().contains("cannot roll back append"));
        assert_eq!(fs::read(path).unwrap(), b"original\n");
    }

    #[test]
    fn invalid_utf8_nonregular_paths_and_missing_parent_errors_do_not_rewrite_or_mkdir() {
        let directory = tempfile::tempdir().unwrap();
        assert!(DownloadArchive::load(directory.path()).is_err());
        assert!(DownloadArchive::load("").is_err());
        let invalid = directory.path().join("invalid");
        fs::write(&invalid, [0xff, b'\n']).unwrap();
        assert!(DownloadArchive::load(&invalid).is_err());
        assert_eq!(fs::read(&invalid).unwrap(), [0xff, b'\n']);
        for suffix in ["child", "child/grandchild", "child/grandchild/archive"] {
            assert!(DownloadArchive::load(invalid.join(suffix)).is_err());
        }
        let missing = directory.path().join("missing/child/grandchild/archive");
        let mut archive = DownloadArchive::load(missing.clone()).unwrap();
        assert!(archive.record(&info("Test", "id")).is_err());
        assert!(!directory.path().join("missing").exists());
        let valid = directory.path().join("valid");
        fs::write(&valid, b"test previous\n").unwrap();
        let mut archive = DownloadArchive::load(valid.clone()).unwrap();
        fs::write(&valid, [0xff]).unwrap();
        assert!(archive.record(&info("Test", "next")).is_err());
        assert_eq!(fs::read(&valid).unwrap(), [0xff]);
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_to_regular_files_follow_normal_filesystem_behavior() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        let link = directory.path().join("link");
        fs::write(&target, "test previous\n").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let mut archive = DownloadArchive::load(link).unwrap();
        assert!(archive.contains(&info("Test", "previous")).unwrap());
        archive.record(&info("Test", "next")).unwrap();
        assert_eq!(
            fs::read_to_string(target).unwrap(),
            "test previous\ntest next\n"
        );
    }
    #[test]
    fn saved_pinned_identity_membership_and_record_fixture_matches() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/archive/cases.json")).unwrap();
        assert_eq!(
            fixture["upstream_commit"],
            "897341241f2bfb2439fe0367f4c7c9ccd29a3828"
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("archive");
        let original = fixture["archive_contents"].as_str().unwrap();
        fs::write(&path, original).unwrap();
        let mut archive = DownloadArchive::load(path.clone()).unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let info: VideoInfo = serde_json::from_value(case["info"].clone()).unwrap();
            assert_eq!(
                archive_id(&info).unwrap(),
                case["expected_archive_id"],
                "{}",
                case["name"]
            );
            assert_eq!(
                archive.contains(&info).unwrap(),
                case["expected_hit"],
                "{}",
                case["name"]
            );
        }
        let mut expected = original.to_owned();
        for case in fixture["record_cases"].as_array().unwrap() {
            let info: VideoInfo = serde_json::from_value(case["info"].clone()).unwrap();
            assert_eq!(archive_id(&info).unwrap(), case["expected_archive_id"]);
            archive.record(&info).unwrap();
            assert!(archive.contains(&info).unwrap());
            expected.push_str(case["expected_archive_id"].as_str().unwrap());
            expected.push('\n');
        }
        assert_eq!(fs::read_to_string(path).unwrap(), expected);
    }
}

use anyhow::{Context, Result};
use fs2::FileExt;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
};
use tokio::sync::{mpsc, oneshot};
use ytui_core::{DownloadQueue, Media, PlaybackQueue};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    pub saved_media: ytui_core::saved::SavedMedia,
    pub recent_destinations: Vec<PathBuf>,
    pub downloads: DownloadQueue,
    pub playback: PlaybackQueue,
    pub favorites: Vec<Media>,
    pub positions: ytui_core::playback_history::PlaybackHistory,
    pub watches: Vec<ytui_core::watch::CollectionWatch>,
}
enum Request {
    Load(oneshot::Sender<Result<Snapshot>>),
    Save(Box<Snapshot>, oneshot::Sender<Result<()>>),
    Stop(oneshot::Sender<()>),
}
pub struct Store {
    tx: mpsc::Sender<Request>,
    join: Option<std::thread::JoinHandle<()>>,
    _lock: File,
}
impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        crate::paths::private_dir(dir)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("instance.lock"))?;
        lock.try_lock_exclusive()
            .context("Another ytui Rust instance is using this state directory")?;
        cleanup_transient(&dir.join("auth/transient"))?;
        let (tx, mut rx) = mpsc::channel::<Request>(16);
        let (ready, ready_rx) = std::sync::mpsc::channel();
        let path = dir.join("state.db");
        let join = std::thread::spawn(move || {
            let opened = (|| -> Result<Connection> {
                let conn = Connection::open(&path)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
                }
                let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
                anyhow::ensure!(
                    version <= 2,
                    "State database was created by a newer ytui; use that version or a separate state directory"
                );
                if version == 1 {
                    let parent = path.parent().context("State database has no parent")?;
                    let backup = parent.join("state-v1-backup.db");
                    if !backup.exists() {
                        let temp = tempfile::NamedTempFile::new_in(parent)?;
                        backup_legacy(&path, temp.path())?;
                        temp.as_file().sync_all()?;
                        temp.persist_noclobber(&backup).map_err(|e| e.error)?;
                    }
                }
                conn.busy_timeout(std::time::Duration::from_secs(3))?;
                conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS app_state(name TEXT PRIMARY KEY,data TEXT NOT NULL); PRAGMA user_version=2;")?;
                Ok(conn)
            })();
            let mut conn = match opened {
                Ok(c) => {
                    let _ = ready.send(Ok(()));
                    c
                }
                Err(e) => {
                    let _ = ready.send(Err(e));
                    return;
                }
            };
            while let Some(request) = rx.blocking_recv() {
                match request {
                    Request::Load(reply) => {
                        let _ = reply.send(load(&conn));
                    }
                    Request::Save(state, reply) => {
                        let _ = reply.send(save(&mut conn, &state));
                    }
                    Request::Stop(reply) => {
                        drop(conn);
                        let _ = reply.send(());
                        return;
                    }
                }
            }
        });
        ready_rx.recv()??;
        Ok(Self {
            tx,
            join: Some(join),
            _lock: lock,
        })
    }
    pub async fn load(&self) -> Result<Snapshot> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(Request::Load(tx)).await?;
        rx.await?
    }
    /// Queue one bounded background save; the interface awaits its acknowledgement
    /// independently of keyboard input and never queues overlapping snapshots.
    pub fn begin_save(&self, snapshot: Snapshot) -> Result<oneshot::Receiver<Result<()>>> {
        let (tx, rx) = oneshot::channel();
        self.tx
            .try_send(Request::Save(Box::new(snapshot), tx))
            .map_err(|_| anyhow::anyhow!("State save worker is busy or unavailable"))?;
        Ok(rx)
    }
    pub async fn save(&self, snapshot: Snapshot) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(Request::Save(Box::new(snapshot), tx)).await?;
        rx.await?
    }
    pub async fn close(mut self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        self.tx.send(Request::Stop(tx)).await?;
        rx.await?;
        if let Some(join) = self.join.take() {
            tokio::task::spawn_blocking(move || {
                join.join()
                    .map_err(|_| anyhow::anyhow!("Database worker panicked"))
            })
            .await??;
        }
        // Closing only this descriptor does not release flock if a concurrent
        // fork still holds the same open file description before exec. The DB
        // owner is joined, so explicitly release before acknowledging shutdown.
        FileExt::unlock(&self._lock)?;
        Ok(())
    }
}
fn cleanup_transient(directory: &Path) -> Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let owned = (name.starts_with("session-") && name.ends_with(".cookies.txt"))
            || (name.starts_with("native-") && name.ends_with(".json"));
        if owned && entry.file_type()?.is_file() {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}
fn load(conn: &Connection) -> Result<Snapshot> {
    use rusqlite::OptionalExtension;
    let value: Option<String> = conn
        .query_row(
            "SELECT data FROM app_state WHERE name='snapshot'",
            [],
            |r| r.get(0),
        )
        .optional()?;
    let mut snapshot: Snapshot = value
        .map(|s| serde_json::from_str(&s))
        .transpose()?
        .unwrap_or_default();
    for job in &mut snapshot.downloads.jobs {
        job.restart_recovery();
    }
    Ok(snapshot)
}
fn save(conn: &mut Connection, state: &Snapshot) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("INSERT INTO app_state(name,data) VALUES('snapshot',?1) ON CONFLICT(name) DO UPDATE SET data=excluded.data",params![serde_json::to_string(state)?])?;
    tx.commit()?;
    Ok(())
}

pub fn backup_legacy(source: &Path, destination: &Path) -> Result<PathBuf> {
    let source = Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut dest = Connection::open(destination)?;
    let backup = rusqlite::backup::Backup::new(&source, &mut dest)?;
    backup.run_to_completion(100, std::time::Duration::from_millis(10), None)?;
    Ok(destination.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ytui_core::*;
    #[tokio::test]
    async fn saved_media_survives_restart_and_old_snapshots_default_to_empty() {
        let old: Snapshot =
            serde_json::from_str(r#"{"favorites":[],"positions":{"entries":[]}}"#).unwrap();
        assert!(old.saved_media.entries.is_empty());
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path()).unwrap();
        let mut snapshot = old;
        snapshot
            .saved_media
            .toggle(Media {
                url: "https://youtu.be/abcdefghijk".into(),
                title: "Saved fixture".into(),
                ..Default::default()
            })
            .unwrap();
        store.save(snapshot).await.unwrap();
        store.close().await.unwrap();
        let store = Store::open(temp.path()).unwrap();
        let restored = store.load().await.unwrap();
        assert_eq!(restored.saved_media.entries[0].title, "Saved fixture");
        assert!(restored.downloads.jobs.is_empty());
        store.close().await.unwrap();
    }
    #[tokio::test]
    async fn close_releases_instance_lock_with_an_inherited_file_description() {
        let temp = tempfile::tempdir().unwrap();
        let store = Store::open(temp.path()).unwrap();
        // dup() and fork() keep the same open file description alive. A child
        // racing exec can retain it briefly even with close-on-exec enabled.
        let inherited = store._lock.try_clone().unwrap();
        store.close().await.unwrap();
        let restarted =
            Store::open(temp.path()).expect("acknowledged shutdown must release the instance lock");
        drop(inherited);
        restarted.close().await.unwrap();
    }
    #[tokio::test]
    async fn persists_paused_intent_recovers_active_and_locks_instances() {
        let d = tempfile::tempdir().unwrap();
        let store = Store::open(d.path()).unwrap();
        assert!(Store::open(d.path()).is_err());
        let mut s = Snapshot::default();
        let mut a = Job::new(Media::default(), DownloadOptions::default());
        a.state = JobState::Downloading;
        a.not_before = Some(12345678900);
        let mut b = a.clone();
        b.id = uuid::Uuid::new_v4();
        b.state = JobState::Paused;
        s.downloads.jobs = vec![a, b];
        store.save(s).await.unwrap();
        store.close().await.unwrap();
        let store = Store::open(d.path()).unwrap();
        let loaded = store.load().await.unwrap();
        assert_eq!(loaded.downloads.jobs[0].state, JobState::Queued);
        assert_eq!(loaded.downloads.jobs[0].not_before, Some(12345678900));
        assert_eq!(loaded.downloads.jobs[1].state, JobState::Paused);
        store.close().await.unwrap();
    }
    #[tokio::test]
    async fn stale_private_exports_are_removed_only_after_the_instance_lock_is_acquired() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("auth/transient");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("session-old.cookies.txt"), b"synthetic").unwrap();
        std::fs::write(directory.join("native-old.json"), b"{}").unwrap();
        std::fs::write(directory.join("unrelated.json"), b"keep").unwrap();
        let store = Store::open(temp.path()).unwrap();
        assert!(!directory.join("session-old.cookies.txt").exists());
        assert!(!directory.join("native-old.json").exists());
        std::fs::write(directory.join("session-active.cookies.txt"), b"synthetic").unwrap();
        assert!(Store::open(temp.path()).is_err());
        assert!(directory.join("session-active.cookies.txt").exists());
        assert_eq!(
            std::fs::read(directory.join("unrelated.json")).unwrap(),
            b"keep"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(temp.path().join("state.db"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        store.close().await.unwrap();
    }
    #[tokio::test]
    async fn version_one_upgrade_retains_an_online_backup_before_advancing_the_version() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.db");
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA user_version=1; CREATE TABLE app_state(name TEXT PRIMARY KEY,data TEXT NOT NULL); INSERT INTO app_state VALUES('snapshot','{}');").unwrap();
        drop(db);
        let store = Store::open(temp.path()).unwrap();
        store.load().await.unwrap();
        store.close().await.unwrap();
        let backup = Connection::open(temp.path().join("state-v1-backup.db")).unwrap();
        assert_eq!(
            backup
                .query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            backup
                .query_row("SELECT data FROM app_state", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "{}"
        );
        let db = Connection::open(path).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, u32>(0))
                .unwrap(),
            2
        );
    }
}

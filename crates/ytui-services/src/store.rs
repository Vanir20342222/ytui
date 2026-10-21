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
    pub downloads: DownloadQueue,
    pub playback: PlaybackQueue,
    pub favorites: Vec<Media>,
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
        std::fs::create_dir_all(dir)?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("instance.lock"))?;
        lock.try_lock_exclusive()
            .context("Another ytui Rust instance is using this state directory")?;
        let (tx, mut rx) = mpsc::channel::<Request>(16);
        let (ready, ready_rx) = std::sync::mpsc::channel();
        let path = dir.join("state.db");
        let join = std::thread::spawn(move || {
            let opened = (|| -> Result<Connection> {
                let conn = Connection::open(path)?;
                let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
                anyhow::ensure!(
                    version <= 1,
                    "State database was created by a newer ytui; use that version or a separate state directory"
                );
                conn.busy_timeout(std::time::Duration::from_secs(3))?;
                conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS app_state(name TEXT PRIMARY KEY,data TEXT NOT NULL); PRAGMA user_version=1;")?;
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
        Ok(())
    }
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
}

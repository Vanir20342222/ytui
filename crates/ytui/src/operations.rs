//! Ordered account/configuration commits. Slow preparation is cancellable;
//! filesystem/keyring commits retain their lock even if their UI task is dropped.
use anyhow::{Result, ensure};
use std::{
    future::Future,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Epoch {
    id: u64,
    cancel: CancellationToken,
}
impl Epoch {
    fn next(&mut self) -> Part {
        self.cancel.cancel();
        self.id = self.id.wrapping_add(1);
        self.cancel = CancellationToken::new();
        Part {
            id: self.id,
            cancel: self.cancel.clone(),
        }
    }
}
#[derive(Default)]
struct State {
    config: Epoch,
    account: Epoch,
}
#[derive(Clone)]
struct Part {
    id: u64,
    cancel: CancellationToken,
}
#[derive(Clone)]
pub struct Ticket {
    config: Option<Part>,
    account: Option<Part>,
    preparation_cancel: CancellationToken,
}
impl Ticket {
    pub fn same(&self, other: &Self) -> bool {
        self.config.as_ref().map(|p| p.id) == other.config.as_ref().map(|p| p.id)
            && self.account.as_ref().map(|p| p.id) == other.account.as_ref().map(|p| p.id)
    }
    async fn cancelled(&self) {
        let wait = |part: Option<Part>| async move {
            match part {
                Some(part) => part.cancel.cancelled().await,
                None => std::future::pending().await,
            }
        };
        tokio::select! { _ = wait(self.config.clone()) => {}, _ = wait(self.account.clone()) => {} }
    }
    pub async fn prepare<T>(&self, future: impl Future<Output = Result<T>>) -> Result<T> {
        tokio::select! {
            biased;
            _ = self.preparation_cancel.cancelled() => anyhow::bail!("Operation cancelled"),
            _ = self.cancelled() => anyhow::bail!("Operation superseded"),
            result = future => result,
        }
    }
    pub fn cancel_preparation(&self) {
        self.preparation_cancel.cancel();
    }
}
#[derive(Clone, Default)]
pub struct Operations {
    state: Arc<Mutex<State>>,
    writer: Arc<tokio::sync::Mutex<()>>,
}
impl Operations {
    pub fn config(&self, changes_account: bool) -> Ticket {
        let mut state = self.state.lock().unwrap();
        Ticket {
            config: Some(state.config.next()),
            account: changes_account.then(|| state.account.next()),
            preparation_cancel: CancellationToken::new(),
        }
    }
    pub fn account(&self) -> Ticket {
        Ticket {
            config: None,
            account: Some(self.state.lock().unwrap().account.next()),
            preparation_cancel: CancellationToken::new(),
        }
    }
    pub fn current(&self, ticket: &Ticket) -> bool {
        let state = self.state.lock().unwrap();
        ticket
            .config
            .as_ref()
            .is_none_or(|p| p.id == state.config.id && !p.cancel.is_cancelled())
            && ticket
                .account
                .as_ref()
                .is_none_or(|p| p.id == state.account.id && !p.cancel.is_cancelled())
    }
    pub fn cancel(&self) {
        let mut state = self.state.lock().unwrap();
        state.config.next();
        state.account.next();
    }
    pub async fn commit<T: Send + 'static>(
        &self,
        ticket: Ticket,
        action: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let lock = self.writer.clone().lock_owned().await;
        let operations = self.clone();
        // Move the lock into the blocking operation. Aborting its async caller
        // must not allow a newer credential write to race a surviving old one.
        tokio::task::spawn_blocking(move || {
            let _lock = lock;
            ensure!(operations.current(&ticket), "Operation superseded");
            ensure!(
                !ticket.preparation_cancel.is_cancelled(),
                "Operation cancelled"
            );
            action()
        })
        .await?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    #[tokio::test]
    async fn obsolete_preparation_cannot_commit_and_unrelated_settings_preserve_disconnect() {
        let operations = Operations::default();
        let import = operations.account();
        let disconnect = operations.account();
        let settings = operations.config(false);
        assert!(!operations.current(&import));
        assert!(operations.current(&disconnect));
        assert!(operations.current(&settings));
        assert!(import.prepare(async { Ok(()) }).await.is_err());
        let called = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let observed = called.clone();
        assert!(
            operations
                .commit(import, move || {
                    observed.store(true, std::sync::atomic::Ordering::SeqCst);
                    Ok(())
                })
                .await
                .is_err()
        );
        assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
        let old = operations.config(true);
        operations.account();
        assert!(!operations.current(&old));
    }
    #[tokio::test]
    async fn a_cancelled_async_writer_cannot_outlive_and_overwrite_the_new_commit() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("synthetic-session");
        let operations = Operations::default();
        let old = operations.account();
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let (op, p) = (operations.clone(), path.clone());
        let task = tokio::spawn(async move {
            op.commit(old, move || {
                let _ = entered.send(());
                gate.recv().unwrap();
                std::fs::write(p, "old")?;
                Ok(())
            })
            .await
        });
        started.await.unwrap();
        task.abort();
        let _ = task.await;
        let new = operations.account();
        let (op, p) = (operations.clone(), path.clone());
        let next = tokio::spawn(async move {
            op.commit(new, move || {
                std::fs::write(p, "new")?;
                Ok(())
            })
            .await
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(
            !next.is_finished(),
            "new commit must wait for the surviving old writer"
        );
        release.send(()).unwrap();
        next.await.unwrap().unwrap();
        assert_eq!(std::fs::read_to_string(path).unwrap(), "new");
    }
}

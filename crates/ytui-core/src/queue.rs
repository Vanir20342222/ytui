use crate::{DownloadOptions, Media};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum JobState {
    #[default]
    Queued,
    Resolving,
    Downloading,
    Merging,
    Converting,
    Backoff,
    Paused,
    Cancelled,
    Done,
    Error,
}
impl JobState {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Resolving | Self::Downloading | Self::Merging | Self::Converting | Self::Backoff
        )
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Progress {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub speed: Option<f64>,
    pub eta: Option<f64>,
}
impl Progress {
    pub fn fraction(&self) -> Option<f64> {
        self.total
            .filter(|n| *n > 0)
            .map(|n| (self.downloaded as f64 / n as f64).clamp(0.0, 1.0))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: uuid::Uuid,
    pub generation: u64,
    pub media: Media,
    pub options: DownloadOptions,
    pub state: JobState,
    pub progress: Progress,
    pub outputs: Vec<PathBuf>,
    pub error: Option<String>,
    pub attempt: u32,
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default)]
    pub not_before: Option<u64>,
    #[serde(default)]
    pub created_at: Option<u64>,
    #[serde(default)]
    pub completed_at: Option<u64>,
}
impl Job {
    pub fn new(mut media: Media, options: DownloadOptions) -> Self {
        if options.audio {
            media.kind = crate::MediaKind::Song;
        }
        Self {
            id: uuid::Uuid::new_v4(),
            generation: 0,
            media,
            options,
            state: JobState::Queued,
            progress: Progress::default(),
            outputs: Vec::new(),
            error: None,
            attempt: 0,
            group_id: None,
            not_before: None,
            created_at: Some(crate::schedule::unix_now()),
            completed_at: None,
        }
    }
    pub fn ready(&self, now: u64) -> bool {
        self.state == JobState::Queued && self.not_before.is_none_or(|at| now >= at)
    }
    pub fn invalidate(&mut self, state: JobState) {
        self.generation += 1;
        self.state = state;
    }
    pub fn restart_recovery(&mut self) {
        if self.state.active() {
            self.invalidate(JobState::Queued);
        }
    }
}
#[derive(Clone, Debug)]
pub enum JobUpdate {
    Phase(JobState),
    Metadata(Box<Media>),
    Progress(Progress),
    Output(PathBuf),
    Finished,
    Failed { message: String, transient: bool },
    Retrying(u32),
}
#[derive(Clone, Debug)]
pub struct JobEvent {
    pub id: uuid::Uuid,
    pub generation: u64,
    pub update: JobUpdate,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DownloadQueue {
    pub jobs: Vec<Job>,
}
impl DownloadQueue {
    pub fn apply(&mut self, event: JobEvent) -> bool {
        let Some(job) = self
            .jobs
            .iter_mut()
            .find(|j| j.id == event.id && j.generation == event.generation && j.state.active())
        else {
            return false;
        };
        match event.update {
            JobUpdate::Phase(state) => {
                if state.active() {
                    job.state = state;
                }
            }
            JobUpdate::Metadata(mut media) => {
                if media.url.is_empty() {
                    media.url = job.media.url.clone();
                }
                if media.thumbnail.is_empty() {
                    media.thumbnail = job.media.thumbnail.clone();
                }
                if media.description.is_empty() {
                    media.description = job.media.description.clone();
                }
                if media.chapters.is_empty() {
                    media.chapters = job.media.chapters.clone();
                }
                if media.duration.is_none() {
                    media.duration = job.media.duration;
                }
                if media.id.is_empty() {
                    media.id = job.media.id.clone();
                }
                media.kind = job.media.kind;
                if media.thumbnails.is_empty() {
                    media.thumbnails = job.media.thumbnails.clone();
                }
                if media.section.is_empty() {
                    media.section = job.media.section.clone();
                }
                job.media = *media;
            }
            JobUpdate::Progress(progress) => {
                if job.state == JobState::Resolving {
                    job.state = JobState::Downloading;
                }
                job.progress = progress;
            }
            JobUpdate::Output(path) => {
                if !job.outputs.contains(&path) {
                    job.outputs.push(path);
                }
            }
            JobUpdate::Finished => {
                job.state = JobState::Done;
                job.completed_at = Some(crate::schedule::unix_now());
                job.error = None;
            }
            JobUpdate::Failed { message, .. } => {
                job.state = JobState::Error;
                job.error = Some(message);
            }
            JobUpdate::Retrying(attempt) => {
                job.attempt = attempt;
                job.state = JobState::Backoff;
            }
        }
        true
    }
    pub fn remove(&mut self, id: uuid::Uuid) {
        self.jobs.retain(|j| j.id != id);
    }
    pub fn move_by(&mut self, index: usize, direction: isize) -> bool {
        if index >= self.jobs.len() {
            return false;
        }
        let next = index
            .saturating_add_signed(direction)
            .min(self.jobs.len() - 1);
        self.jobs.swap(index, next);
        next != index
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removed_metadata_cannot_resurrect_a_job() {
        let mut q = DownloadQueue::default();
        let mut job = Job::new(Media::default(), DownloadOptions::default());
        job.state = JobState::Resolving;
        let id = job.id;
        q.jobs.push(job);
        q.remove(id);
        assert!(!q.apply(JobEvent {
            id,
            generation: 0,
            update: JobUpdate::Metadata(Box::default())
        }));
        assert!(q.jobs.is_empty());
    }
    #[test]
    fn pause_and_retry_reject_previous_attempt_results() {
        let mut q = DownloadQueue::default();
        let mut job = Job::new(Media::default(), DownloadOptions::default());
        job.state = JobState::Downloading;
        let id = job.id;
        job.invalidate(JobState::Paused);
        q.jobs.push(job);
        assert!(!q.apply(JobEvent {
            id,
            generation: 0,
            update: JobUpdate::Finished
        }));
        q.jobs[0].invalidate(JobState::Resolving);
        assert!(!q.apply(JobEvent {
            id,
            generation: 1,
            update: JobUpdate::Finished
        }));
        assert!(q.apply(JobEvent {
            id,
            generation: 2,
            update: JobUpdate::Finished
        }));
        assert!(!q.apply(JobEvent {
            id,
            generation: 2,
            update: JobUpdate::Phase(JobState::Downloading)
        }));
    }
}

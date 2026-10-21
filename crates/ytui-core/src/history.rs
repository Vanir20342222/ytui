use crate::{Job, JobState};
use anyhow::{Result, ensure};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Completed,
    Failed,
    Cancelled,
    Audio,
    Video,
}
impl Filter {
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::Completed,
        Self::Failed,
        Self::Cancelled,
        Self::Audio,
        Self::Video,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Audio => "audio",
            Self::Video => "video",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.name() == value)
    }
    pub fn next(self) -> Self {
        Self::ALL[(Self::ALL.iter().position(|f| *f == self).unwrap_or(0) + 1) % Self::ALL.len()]
    }
    pub fn matches(self, job: &Job) -> bool {
        match self {
            Self::All => true,
            Self::Completed => job.state == JobState::Done,
            Self::Failed => job.state == JobState::Error,
            Self::Cancelled => job.state == JobState::Cancelled,
            Self::Audio => job.options.audio,
            Self::Video => !job.options.audio,
        }
    }
}
#[derive(Default)]
pub struct View {
    pub selected: usize,
    pub filter: Filter,
    pub query: String,
    pub group: Option<String>,
}
impl View {
    /// Keep an index into the real queue; filtering never mutates stored rows.
    pub fn rows<'a>(&self, jobs: &'a [Job]) -> Vec<(usize, &'a Job)> {
        let query = self.query.to_lowercase();
        let mut rows = jobs
            .iter()
            .enumerate()
            .filter(|(_, j)| {
                self.filter.matches(j)
                    && self
                        .group
                        .as_ref()
                        .is_none_or(|g| j.group_id.as_ref() == Some(g))
                    && (query.is_empty()
                        || j.media.title.to_lowercase().contains(&query)
                        || j.media.creator.to_lowercase().contains(&query)
                        || j.media.url.to_lowercase().contains(&query))
            })
            .collect::<Vec<_>>();
        if self.group.is_none() {
            rows.sort_by_key(|(index, j)| {
                std::cmp::Reverse((j.completed_at.or(j.created_at).unwrap_or(0), *index))
            });
        }
        rows
    }
}
#[derive(Debug, Default)]
pub struct Statistics {
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
    pub audio: usize,
    pub outputs: usize,
    pub media_seconds: u64,
    pub recorded_bytes: u64,
}
impl Statistics {
    pub fn from_jobs<'a>(jobs: impl IntoIterator<Item = &'a Job>) -> Self {
        let mut stats = Self::default();
        for job in jobs {
            stats.total += 1;
            stats.completed += usize::from(job.state == JobState::Done);
            stats.failed += usize::from(job.state == JobState::Error);
            stats.audio += usize::from(job.options.audio);
            stats.outputs += job.outputs.len();
            if job.state == JobState::Done {
                stats.media_seconds = stats
                    .media_seconds
                    .saturating_add(job.media.duration.unwrap_or(0));
            }
            stats.recorded_bytes = stats.recorded_bytes.saturating_add(job.progress.downloaded);
        }
        stats
    }
}
pub fn selection_indices(value: &str, count: usize) -> Result<Vec<usize>> {
    ensure!(count > 0, "The current page has no media");
    if value.is_empty() || value == "all" {
        return Ok((0..count).collect());
    }
    let mut selected = vec![false; count];
    for part in value.split(',') {
        let part = part.trim();
        let (start, end) = part
            .split_once(':')
            .or_else(|| part.split_once('-'))
            .unwrap_or((part, part));
        let (start, end) = (start.parse::<usize>()?, end.parse::<usize>()?);
        ensure!(
            start > 0 && start <= end && end <= count,
            "Indices must be between 1 and {count}, e.g. 1,3:7"
        );
        selected[start - 1..end].fill(true);
    }
    Ok(selected
        .into_iter()
        .enumerate()
        .filter_map(|(i, selected)| selected.then_some(i))
        .collect())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DownloadOptions, Media};
    #[test]
    fn filtering_and_groups_preserve_real_indices_and_order() {
        let mut a = Job::new(
            Media {
                title: "日本語".into(),
                ..Default::default()
            },
            DownloadOptions::default(),
        );
        a.state = JobState::Done;
        a.completed_at = Some(100);
        a.group_id = Some("group".into());
        let mut b = a.clone();
        b.state = JobState::Error;
        b.media.title = "Other".into();
        b.completed_at = Some(200);
        let jobs = [a, b];
        let view = View {
            query: "日本".into(),
            ..Default::default()
        };
        assert_eq!(view.rows(&jobs)[0].0, 0);
        assert_eq!(View::default().rows(&jobs)[0].0, 1);
        assert_eq!(
            View {
                group: Some("group".into()),
                ..Default::default()
            }
            .rows(&jobs)[0]
                .0,
            0
        );
        let stats = Statistics::from_jobs(jobs.iter());
        assert_eq!((stats.total, stats.completed, stats.failed), (2, 1, 1));
        assert_eq!(selection_indices("1,3:5,4", 5).unwrap(), vec![0, 2, 3, 4]);
        for value in ["0", "6", "4:2", "1:99999999999"] {
            assert!(selection_indices(value, 5).is_err());
        }
    }
}

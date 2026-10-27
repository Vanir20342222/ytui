//! Adversarial transfer lifecycle checks. Only controlled tools and isolated files.
use std::{
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use ytui_core::{Config, DownloadQueue, Job, JobEvent, JobState, JobUpdate, Media};
use ytui_services::{download, paths::Paths};

struct Fixture {
    temp: tempfile::TempDir,
    config: Config,
    paths: Paths,
}
impl Fixture {
    fn new(script: &str) -> Self {
        let temp = tempfile::Builder::new()
            .prefix("ytui 日本語 tests-")
            .tempdir()
            .unwrap();
        let root = temp.path();
        let paths = Paths {
            config_override: None,
            xdg_state: root.join("xdg"),
            config: root.join("config"),
            state: root.join("state"),
            cache: root.join("cache"),
            legacy_data: root.join("legacy"),
            home: root.into(),
        };
        paths.create().unwrap();
        let mut config = Config {
            download_backend: "yt-dlp".into(),
            max_retries: 0,
            retry_delay_seconds: 0,
            auth_store: "file".into(),
            ..Default::default()
        };
        paths.fill_directories(&mut config);
        config.download.metadata = false;
        let mut fixture = Self {
            temp,
            config,
            paths,
        };
        fixture.config.yt_dlp = fixture.tool("extractor", script).display().to_string();
        fixture
    }
    fn root(&self) -> &Path {
        self.temp.path()
    }
    fn tool(&self, name: &str, script: &str) -> PathBuf {
        let path = self.root().join(name);
        let quoted = self.root().display().to_string().replace('\'', "'\\''");
        std::fs::write(
            &path,
            format!("#!/bin/sh\ncd '{quoted}' || exit 90\n{script}\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
    fn job(&self) -> Job {
        let mut job = Job::new(
            Media {
                url: "https://music.youtube.com/playlist?list=PLsynthetic".into(),
                title: "Fixture playlist".into(),
                ..Default::default()
            },
            self.config.download.clone(),
        );
        job.state = JobState::Resolving;
        job
    }
    async fn run(&self, job: Job) -> (Job, Vec<JobUpdate>) {
        let (tx, mut rx) = mpsc::channel(16);
        let mut queue = DownloadQueue {
            jobs: vec![job.clone()],
        };
        let worker = tokio::spawn(download::execute(
            job,
            self.config.clone(),
            self.paths.clone(),
            None,
            CancellationToken::new(),
            tx,
        ));
        let mut events = Vec::new();
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = rx.recv().await {
                events.push(event.update.clone());
                queue.apply(event);
            }
            worker.await.unwrap();
        })
        .await
        .expect("fixture stalled");
        (queue.jobs.remove(0), events)
    }
}
const TWO_OUTPUTS: &str = r#"
printf first > first.media
printf second > second.media
printf '%s\n' 'YTUI_META:{"id":"one","title":"First track"}'
printf 'YTUI_OUTPUT:"%s/first.media"\n' "$PWD"
printf '%s\n' 'YTUI_META:{"id":"two","title":"Second track"}'
printf 'YTUI_OUTPUT:"%s/second.media"\n' "$PWD"
"#;
#[test]
fn retry_policy_does_not_repeat_permanent_proxy_or_storage_rejections() {
    for message in [
        "Unable to connect to proxy: Tunnel connection failed: 403 Forbidden",
        "CONNECT tunnel failed, response 403",
        "HTTP 407 Proxy Authentication Required; connection failed",
        "Read-only file system after connection reset",
    ] {
        assert!(!download::transient(message), "{message}");
    }
    assert!(download::transient(
        "Connection reset while contacting proxy"
    ));
    assert!(download::transient("HTTP Error 503: Service Unavailable"));
}
#[tokio::test]
async fn playlist_hooks_use_each_tracks_metadata_once_even_for_duplicate_output_records() {
    let mut fixture = Fixture::new(&format!(
        "{TWO_OUTPUTS}\nprintf 'YTUI_OUTPUT:\"%s/second.media\"\\n' \"$PWD\""
    ));
    fixture.config.download.post_download_hook = Some(fixture.tool(
        "hook",
        r#"printf '%s|%s|%s\n' "$YTUI_VIDEO_ID" "$YTUI_TITLE" "$YTUI_OUTPUT_PATH" >> hooks"#,
    ));
    let (job, _) = fixture.run(fixture.job()).await;
    assert_eq!(job.state, JobState::Done);
    let calls = std::fs::read_to_string(fixture.root().join("hooks")).unwrap();
    let lines = calls.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        2,
        "duplicate output invoked a hook twice: {calls}"
    );
    assert!(
        lines[0].starts_with("one|First track|"),
        "first output got another track's metadata: {calls}"
    );
    assert!(lines[1].starts_with("two|Second track|"));
    assert_eq!(job.outputs.len(), 2);
}
#[tokio::test]
async fn hook_failure_keeps_every_completed_file_and_never_marks_playlist_done() {
    let mut fixture = Fixture::new(TWO_OUTPUTS);
    fixture.config.download.post_download_hook =
        Some(fixture.tool("hook", "printf x >> hook-count\nexit 1"));
    let (job, events) = fixture.run(fixture.job()).await;
    assert_eq!(job.state, JobState::Error);
    assert_eq!(
        job.outputs.len(),
        2,
        "later completed output was forgotten on hook failure"
    );
    assert!(job.outputs.iter().all(|path| path.is_file()));
    assert_eq!(
        std::fs::read(fixture.root().join("hook-count")).unwrap(),
        b"x"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, JobUpdate::Finished))
    );
}
#[tokio::test]
async fn success_without_valid_final_outputs_cannot_finish_a_job() {
    for script in [
        "exit 0",
        r#"printf 'YTUI_OUTPUT:"%s/missing.media"\n' "$PWD""#,
        "printf '%s\\n' 'YTUI_OUTPUT:not-json'",
    ] {
        let fixture = Fixture::new(script);
        let (job, events) = fixture.run(fixture.job()).await;
        assert_eq!(job.state, JobState::Error);
        assert!(job.outputs.is_empty());
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, JobUpdate::Finished))
        );
    }
}
#[tokio::test]
async fn a_later_invalid_output_never_discards_completed_playlist_files() {
    for invalid in [
        r#"printf 'YTUI_OUTPUT:"%s/missing.media"\n' "$PWD""#,
        "printf '%s\\n' 'YTUI_OUTPUT:not-json'",
    ] {
        let fixture = Fixture::new(&format!(
            r#"
printf first > first.media
printf 'YTUI_OUTPUT:"%s/first.media"\n' "$PWD"
{invalid}
"#
        ));
        let (job, events) = fixture.run(fixture.job()).await;
        assert_eq!(job.state, JobState::Error);
        assert_eq!(job.outputs, vec![fixture.root().join("first.media")]);
        assert!(!events.iter().any(|e| matches!(e, JobUpdate::Finished)));
    }
}
#[tokio::test]
async fn an_empty_final_file_is_not_a_successful_download() {
    let fixture = Fixture::new(
        r#": > empty.media
printf 'YTUI_OUTPUT:"%s/empty.media"\n' "$PWD""#,
    );
    let (job, _) = fixture.run(fixture.job()).await;
    assert_eq!(job.state, JobState::Error);
    assert!(job.outputs.is_empty());
}
#[tokio::test]
async fn a_failed_single_video_keeps_its_published_output() {
    let fixture = Fixture::new(
        r#"printf media > first.media
printf 'YTUI_OUTPUT:"%s/first.media"\n' "$PWD"
echo 'postprocessing failed' >&2
exit 1"#,
    );
    let mut job = fixture.job();
    job.options.playlist = false;
    job.media.url = "https://www.youtube.com/watch?v=abcdefghijk".into();
    let (job, _) = fixture.run(job).await;
    assert_eq!(job.state, JobState::Error);
    assert_eq!(job.outputs, vec![fixture.root().join("first.media")]);
}
#[tokio::test]
async fn temporary_playlist_failure_retries_whole_source_and_keeps_completed_outputs() {
    let mut fixture = Fixture::new(
        r#"
printf '%s\n' "$@" >> arguments
if [ ! -f attempted ]; then
  : > attempted
  printf keep-first > first.media
  printf 'YTUI_OUTPUT:"%s/first.media"\n' "$PWD"
  echo 'HTTP 503 Service Unavailable' >&2
  exit 1
fi
printf next > second.media
printf 'YTUI_OUTPUT:"%s/first.media"\n' "$PWD"
printf 'YTUI_OUTPUT:"%s/second.media"\n' "$PWD"
"#,
    );
    fixture.config.max_retries = 1;
    let source = fixture.job().media.url;
    let (job, events) = fixture.run(fixture.job()).await;
    assert_eq!(job.state, JobState::Done);
    assert_eq!(job.media.url, source);
    assert_eq!(job.outputs.len(), 2);
    assert_eq!(
        std::fs::read(fixture.root().join("first.media")).unwrap(),
        b"keep-first"
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root().join("arguments"))
            .unwrap()
            .matches(&source)
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, JobUpdate::Retrying(1)))
            .count(),
        1
    );
}
#[tokio::test]
async fn unavailable_item_has_no_retry_loop_and_retains_prior_output() {
    let mut fixture = Fixture::new(
        r#"
printf x >> calls
printf first > first.media
printf 'YTUI_OUTPUT:"%s/first.media"\n' "$PWD"
echo 'Video unavailable: private video' >&2
exit 1
"#,
    );
    fixture.config.max_retries = 3;
    let (job, events) = fixture.run(fixture.job()).await;
    assert_eq!(job.state, JobState::Error);
    assert_eq!(job.outputs.len(), 1);
    assert_eq!(std::fs::read(fixture.root().join("calls")).unwrap(), b"x");
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, JobUpdate::Retrying(_)))
    );
}
#[tokio::test]
async fn saved_partial_playlist_can_retry_without_losing_identity_or_old_files() {
    let fixture = Fixture::new(
        r#"
if [ ! -f first.media ]; then
  printf original > first.media
  printf '%s\n' 'YTUI_META:{"id":"first","webpage_url":"https://www.youtube.com/watch?v=first"}'
  printf 'YTUI_OUTPUT:"%s/first.media"\n' "$PWD"
  echo 'Video unavailable' >&2
  exit 1
fi
printf second > second.media
printf 'YTUI_OUTPUT:"%s/first.media"\n' "$PWD"
printf 'YTUI_OUTPUT:"%s/second.media"\n' "$PWD"
"#,
    );
    let (first, _) = fixture.run(fixture.job()).await;
    assert_eq!(first.state, JobState::Error);
    let source = first.media.url.clone();
    let mut restored: Job = serde_json::from_str(&serde_json::to_string(&first).unwrap()).unwrap();
    restored.invalidate(JobState::Resolving);
    let (retried, _) = fixture.run(restored).await;
    assert_eq!(retried.state, JobState::Done);
    assert_eq!(retried.media.url, source);
    assert_eq!(retried.outputs.len(), 2);
    assert_eq!(
        std::fs::read(fixture.root().join("first.media")).unwrap(),
        b"original"
    );
}
#[tokio::test]
async fn pausing_blocked_transfer_keeps_files_and_rejects_late_generation_events() {
    let fixture = Fixture::new("printf keep > first.media\n: > ready\nsleep 30");
    let mut job = fixture.job();
    let old_generation = job.generation;
    let (tx, mut rx) = mpsc::channel(16);
    let cancel = CancellationToken::new();
    let worker = tokio::spawn(download::execute(
        job.clone(),
        fixture.config.clone(),
        fixture.paths.clone(),
        None,
        cancel.clone(),
        tx,
    ));
    // Process scheduling on loaded runners is not the cancellation latency contract.
    // Keep the separate two-second cancellation deadline below unchanged.
    tokio::time::timeout(Duration::from_secs(10), async {
        while !fixture.root().join("ready").exists() {
            assert!(
                !worker.is_finished(),
                "Extractor fixture exited before signalling ready"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    job.invalidate(JobState::Paused);
    let mut queue = DownloadQueue { jobs: vec![job] };
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(2), worker)
        .await
        .unwrap()
        .unwrap();
    while let Some(event) = rx.recv().await {
        assert!(!queue.apply(event), "paused job accepted a stale event");
    }
    let id = queue.jobs[0].id;
    assert!(!queue.apply(JobEvent {
        id,
        generation: old_generation,
        update: JobUpdate::Finished
    }));
    assert_eq!(queue.jobs[0].state, JobState::Paused);
    assert_eq!(
        std::fs::read(fixture.root().join("first.media")).unwrap(),
        b"keep"
    );
}

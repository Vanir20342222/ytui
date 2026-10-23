//! Native chapter/SponsorBlock adapters; FFmpeg runs in the isolated worker.
use crate::{native::WorkerRequest, worker_process};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use ytui_core::{Chapter, chapters};
#[derive(Clone, Debug)]
pub struct Segment {
    pub start: f64,
    pub end: f64,
    pub category: String,
}
#[derive(Debug)]
struct Probe {
    duration: f64,
    video: bool,
    audio: bool,
    subtitles: bool,
    chapters: Vec<Chapter>,
}
fn probe(request: &WorkerRequest, path: &Path) -> Result<Probe> {
    let out = worker_process::run(
        &request.config.ffprobe,
        &[
            "-v".into(),
            "error".into(),
            "-show_entries".into(),
            "format=duration:stream=codec_type:chapter=start_time,end_time:chapter_tags=title"
                .into(),
            "-of".into(),
            "json".into(),
            path.to_string_lossy().into(),
        ],
    )?;
    ensure!(
        out.success,
        "Cannot inspect media for chapter/SponsorBlock processing: {}",
        out.stderr
    );
    let data: Value = serde_json::from_slice(&out.stdout)?;
    let number = |v: &Value| {
        v.as_f64()
            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
    };
    let duration = data
        .pointer("/format/duration")
        .and_then(number)
        .context("Media has no finite duration")?;
    ensure!(
        duration.is_finite() && duration > 0.0,
        "Media duration is invalid"
    );
    let streams = data
        .get("streams")
        .and_then(Value::as_array)
        .context("Media has no stream information")?;
    let has = |kind| {
        streams
            .iter()
            .any(|s| s.get("codec_type").and_then(Value::as_str) == Some(kind))
    };
    let chapters = data
        .get("chapters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| {
            Some(Chapter {
                start: number(c.get("start_time")?)?,
                end: c.get("end_time").and_then(number),
                title: c
                    .pointer("/tags/title")
                    .and_then(Value::as_str)
                    .unwrap_or("Chapter")
                    .into(),
            })
        })
        .collect();
    Ok(Probe {
        duration,
        video: has("video"),
        audio: has("audio"),
        subtitles: has("subtitle"),
        chapters,
    })
}
fn parse_segments(data: &Value, categories: &[String], duration: f64) -> Result<Vec<Segment>> {
    let rows = data
        .as_array()
        .context("SponsorBlock returned an unsupported response")?;
    ensure!(
        rows.len() <= 1000,
        "SponsorBlock returned too many segments"
    );
    let mut segments = Vec::new();
    for row in rows {
        let Some(category) = row.get("category").and_then(Value::as_str) else {
            continue;
        };
        if !categories.iter().any(|s| s == category)
            || row
                .get("actionType")
                .and_then(Value::as_str)
                .is_some_and(|v| v != "skip")
        {
            continue;
        }
        let Some(pair) = row.get("segment").and_then(Value::as_array) else {
            continue;
        };
        let (Some(start), Some(end)) = (
            pair.first().and_then(Value::as_f64),
            pair.get(1).and_then(Value::as_f64),
        ) else {
            continue;
        };
        if start.is_finite() && end.is_finite() && start >= 0.0 && end > start && start < duration {
            segments.push(Segment {
                start,
                end: end.min(duration),
                category: category.into(),
            });
        }
    }
    segments.sort_by(|a, b| a.start.total_cmp(&b.start));
    Ok(segments)
}
fn fetch_segments(request: &WorkerRequest, duration: f64) -> Result<Vec<Segment>> {
    if request.job.options.sponsorblock.is_empty() {
        return Ok(Vec::new());
    }
    let media_url = ytui_core::http_url(&request.job.media.url)?;
    ensure!(
        ytui_core::youtube_host(media_url.host_str().unwrap_or_default()),
        "SponsorBlock requires a YouTube video"
    );
    let id = if !request.job.media.id.is_empty() {
        request.job.media.id.clone()
    } else if media_url.host_str() == Some("youtu.be") {
        media_url.path().trim_start_matches('/').into()
    } else {
        media_url
            .query_pairs()
            .find(|(k, _)| k == "v")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default()
    };
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c)),
        "Cannot identify this YouTube video for SponsorBlock"
    );
    let mut url = ytui_core::http_url(&request.job.options.sponsorblock_api)?;
    url.set_path(&format!(
        "{}/api/skipSegments",
        url.path().trim_end_matches('/')
    ));
    url.query_pairs_mut()
        .append_pair("videoID", &id)
        .append_pair(
            "categories",
            &serde_json::to_string(&request.job.options.sponsorblock)?,
        );
    let mut builder = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none());
    if let Some(proxy) = &request.config.proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy)?);
    }
    let mut response = builder.build()?.get(url).send()?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    ensure!(
        response.status().is_success(),
        "SponsorBlock returned HTTP {}",
        response.status().as_u16()
    );
    let mut body = Vec::new();
    Read::take(&mut response, 1024 * 1024 + 1).read_to_end(&mut body)?;
    ensure!(
        body.len() <= 1024 * 1024,
        "SponsorBlock response exceeds 1 MiB"
    );
    parse_segments(
        &serde_json::from_slice(&body)?,
        &request.job.options.sponsorblock,
        duration,
    )
}
fn intervals(segments: &[Segment], duration: f64) -> Vec<(f64, f64)> {
    let mut removed: Vec<(f64, f64)> = Vec::new();
    for s in segments {
        if let Some(last) = removed.last_mut()
            && s.start <= last.1
        {
            last.1 = last.1.max(s.end);
            continue;
        }
        removed.push((s.start, s.end));
    }
    let mut kept = Vec::new();
    let mut cursor: f64 = 0.0;
    for (start, end) in removed {
        if start > cursor {
            kept.push((cursor, start));
        }
        cursor = cursor.max(end);
    }
    if cursor < duration {
        kept.push((cursor, duration));
    }
    kept.into_iter().filter(|(s, e)| e - s > 0.001).collect()
}
fn marked(chapters: &[Chapter], segments: &[Segment], duration: f64) -> Vec<Chapter> {
    let mut boundaries = vec![0.0, duration];
    boundaries.extend(
        chapters
            .iter()
            .flat_map(|c| [c.start, c.end.unwrap_or(duration)]),
    );
    boundaries.extend(segments.iter().flat_map(|s| [s.start, s.end]));
    boundaries.sort_by(f64::total_cmp);
    boundaries.dedup();
    let mut result: Vec<Chapter> = Vec::new();
    for pair in boundaries.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        if start < 0.0 || end > duration || end <= start {
            continue;
        }
        let middle = (start + end) / 2.0;
        let categories = segments
            .iter()
            .filter(|s| s.start <= middle && s.end > middle)
            .map(|s| s.category.as_str())
            .collect::<Vec<_>>();
        let title = if categories.is_empty() {
            chapters
                .iter()
                .rev()
                .find(|c| c.start <= middle && c.end.unwrap_or(duration) > middle)
                .map(|c| c.title.clone())
                .unwrap_or_else(|| "Content".into())
        } else {
            format!("SponsorBlock: {}", categories.join(", "))
        };
        if let Some(last) = result.last_mut()
            && last.title == title
            && last.end == Some(start)
        {
            last.end = Some(end);
            continue;
        }
        result.push(Chapter {
            title,
            start,
            end: Some(end),
        });
    }
    result
}
fn retime(chapters: &[Chapter], kept: &[(f64, f64)]) -> Vec<Chapter> {
    let mut result = Vec::new();
    let mut offset = 0.0;
    for &(start, end) in kept {
        for chapter in chapters {
            let from = start.max(chapter.start);
            let to = end.min(chapter.end.unwrap_or(end));
            if to > from {
                result.push(Chapter {
                    title: chapter.title.clone(),
                    start: offset + from - start,
                    end: Some(offset + to - start),
                });
            }
        }
        offset += end - start;
    }
    result
}
fn execute(request: &WorkerRequest, args: Vec<String>) -> Result<()> {
    let out = worker_process::run(&request.config.ffmpeg, &args)?;
    ensure!(
        out.success,
        "Native FFmpeg postprocessing failed: {}",
        out.stderr
    );
    Ok(())
}
fn cut(
    request: &WorkerRequest,
    source: &Path,
    probe: &Probe,
    kept: &[(f64, f64)],
) -> Result<PathBuf> {
    ensure!(
        !kept.is_empty() && kept.len() <= 128,
        "SponsorBlock cut would remove everything or create too many sections"
    );
    ensure!(
        !probe.subtitles,
        "Native SponsorBlock cutting with embedded subtitles needs explicit yt-dlp fallback to preserve caption timing"
    );
    let path = source.with_file_name(format!(
        ".ytui-cut-{}.{}",
        uuid::Uuid::new_v4(),
        source
            .extension()
            .and_then(|s| s.to_str())
            .context("Missing media extension")?
    ));
    let mut filters = Vec::new();
    let mut inputs = String::new();
    for (i, (start, end)) in kept.iter().enumerate() {
        if probe.video {
            filters.push(format!(
                "[0:v:0]trim=start={start:.6}:end={end:.6},setpts=PTS-STARTPTS[v{i}]"
            ));
            inputs.push_str(&format!("[v{i}]"));
        }
        if probe.audio {
            filters.push(format!(
                "[0:a:0]atrim=start={start:.6}:end={end:.6},asetpts=PTS-STARTPTS[a{i}]"
            ));
            inputs.push_str(&format!("[a{i}]"));
        }
    }
    ensure!(probe.video || probe.audio, "No audio/video to cut");
    filters.push(format!(
        "{inputs}concat=n={}:v={}:a={}{}{}",
        kept.len(),
        u8::from(probe.video),
        u8::from(probe.audio),
        if probe.video { "[v]" } else { "" },
        if probe.audio { "[a]" } else { "" }
    ));
    let mut args = vec![
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-i".into(),
        source.to_string_lossy().into(),
        "-filter_complex".into(),
        filters.join(";"),
    ];
    if probe.video {
        args.extend([
            "-map".into(),
            "[v]".into(),
            "-c:v".into(),
            if request.job.options.container == "webm" {
                "libvpx-vp9"
            } else {
                "libx264"
            }
            .into(),
            "-crf".into(),
            "18".into(),
        ]);
        if request.job.options.container == "webm" {
            args.extend(["-b:v".into(), "0".into()]);
        }
    }
    if probe.audio {
        let format = if request.job.options.audio {
            request.job.options.audio_container.as_str()
        } else if request.job.options.container == "webm" {
            "opus"
        } else {
            "m4a"
        };
        args.extend(["-map".into(), "[a]".into()]);
        args.extend(crate::audio::encoding(
            format,
            &request.job.options.audio_quality,
        )?);
    }
    args.extend([
        "-map_chapters".into(),
        "-1".into(),
        "-n".into(),
        path.to_string_lossy().into(),
    ]);
    execute(request, args)?;
    Ok(path)
}
fn metadata_escape(value: &str) -> String {
    ytui_core::clean_text(value)
        .replace('\\', "\\\\")
        .replace('\n', " ")
        .replace('=', "\\=")
        .replace(';', "\\;")
        .replace('#', "\\#")
}
fn embed(request: &WorkerRequest, source: &Path, chapters: &[Chapter]) -> Result<()> {
    let dir = source.parent().context("No source parent")?;
    let mut metadata = tempfile::Builder::new()
        .suffix(".ffmetadata")
        .tempfile_in(dir)?;
    writeln!(metadata, ";FFMETADATA1")?;
    for chapter in chapters {
        writeln!(
            metadata,
            "[CHAPTER]\nTIMEBASE=1/1000\nSTART={}\nEND={}\ntitle={}",
            (chapter.start * 1000.0).round() as u64,
            (chapter.end.unwrap_or(chapter.start) * 1000.0).round() as u64,
            metadata_escape(&chapter.title)
        )?;
    }
    metadata.flush()?;
    let extension = source
        .extension()
        .and_then(|s| s.to_str())
        .context("No source extension")?;
    let target = dir.join(format!(
        ".ytui-chapters-{}.{extension}",
        uuid::Uuid::new_v4()
    ));
    execute(
        request,
        vec![
            "-nostdin".into(),
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-i".into(),
            source.to_string_lossy().into(),
            "-i".into(),
            metadata.path().to_string_lossy().into(),
            "-map".into(),
            "0".into(),
            "-map_metadata".into(),
            "0".into(),
            "-map_chapters".into(),
            "1".into(),
            "-c".into(),
            "copy".into(),
            "-n".into(),
            target.to_string_lossy().into(),
        ],
    )?;
    std::fs::rename(target, source)?;
    ensure!(
        !probe(request, source)?.chapters.is_empty(),
        "This container did not retain chapters; choose MKV/MP4/M4A or explicit fallback"
    );
    Ok(())
}
fn truncate(value: &str, bytes: usize) -> &str {
    let mut end = value.len().min(bytes);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}
fn split(request: &WorkerRequest, source: &Path, chapters: &[Chapter]) -> Result<Vec<PathBuf>> {
    ensure!(
        !chapters.is_empty(),
        "No chapters found; open video details first or use explicit yt-dlp fallback"
    );
    let mut paths = Vec::new();
    for (index, chapter) in chapters.iter().enumerate() {
        let title = ytui_core::clean_text(&chapter.title)
            .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
        let stem = source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("media");
        let extension = source
            .extension()
            .and_then(|s| s.to_str())
            .context("No chapter extension")?;
        let target = source.with_file_name(format!(
            "{} - {:03} {}.{extension}",
            truncate(stem, 120),
            index + 1,
            truncate(&title, 60)
        ));
        execute(
            request,
            vec![
                "-nostdin".into(),
                "-hide_banner".into(),
                "-loglevel".into(),
                "error".into(),
                "-i".into(),
                source.to_string_lossy().into(),
                "-ss".into(),
                format!("{:.6}", chapter.start),
                "-t".into(),
                format!(
                    "{:.6}",
                    chapter.end.unwrap_or(chapter.start) - chapter.start
                ),
                "-map".into(),
                "0".into(),
                "-map_chapters".into(),
                "-1".into(),
                "-c".into(),
                "copy".into(),
                "-avoid_negative_ts".into(),
                "make_zero".into(),
                "-n".into(),
                target.to_string_lossy().into(),
            ],
        )?;
        paths.push(target);
    }
    Ok(paths)
}
pub fn apply(request: &WorkerRequest, source: &Path) -> Result<Vec<PathBuf>> {
    let o = &request.job.options;
    if !o.split_chapters && o.sponsorblock.is_empty() {
        return Ok(Vec::new());
    }
    let probe = probe(request, source)?;
    let source_chapters = if request.job.media.chapters.is_empty() {
        if probe.chapters.is_empty() {
            chapters::from_description(&request.job.media.description)
        } else {
            probe.chapters.clone()
        }
    } else {
        request.job.media.chapters.clone()
    };
    let mut chapters = chapters::normalize(&source_chapters, probe.duration);
    let segments = fetch_segments(request, probe.duration)?;
    if !segments.is_empty() {
        if o.sponsorblock_remove {
            let kept = intervals(&segments, probe.duration);
            let cut = cut(request, source, &probe, &kept)?;
            chapters = retime(&chapters, &kept);
            std::fs::rename(cut, source)?;
        } else {
            chapters = marked(&chapters, &segments, probe.duration);
        }
    }
    if !chapters.is_empty() {
        embed(request, source, &chapters)?;
    }
    if o.split_chapters {
        split(request, source, &chapters)
    } else {
        Ok(Vec::new())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn untrusted_segments_are_filtered_and_overlaps_do_not_duplicate_content() {
        let segments = parse_segments(
            &serde_json::json!([
                {"category":"sponsor","segment":[1,3]}, {"category":"sponsor","segment":[2,4]},
                {"category":"sponsor","segment":[8,30]}, {"category":"intro","segment":[0,1]},
                {"category":"sponsor","segment":[-1,2]}, {"category":"sponsor","segment":[3,2]}
            ]),
            &["sponsor".into()],
            10.0,
        )
        .unwrap();
        assert_eq!(intervals(&segments, 10.0), vec![(0.0, 1.0), (4.0, 8.0)]);
        let chapters = vec![Chapter {
            title: "Whole".into(),
            start: 0.0,
            end: Some(10.0),
        }];
        let marked = marked(&chapters, &segments, 10.0);
        assert_eq!(marked.first().unwrap().start, 0.0);
        assert_eq!(marked.last().unwrap().end, Some(10.0));
        let moved = retime(&chapters, &intervals(&segments, 10.0));
        assert_eq!(moved.last().unwrap().end, Some(5.0));
        assert_eq!(metadata_escape("a=b;#\\\n"), "a\\=b\\;\\#\\\\ ");
    }
}

#[cfg(test)]
mod real_media_tests {
    use super::*;
    #[test]
    #[ignore = "requires FFmpeg and ffprobe; no Internet or audio device"]
    fn real_ffmpeg_marks_cuts_and_splits_without_touching_the_original() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            thread,
            time::Instant,
        };
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("original.mp4");
        let generated = worker_process::run(
            "ffmpeg",
            &[
                "-nostdin",
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=160x90:rate=10:duration=4",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=4",
                "-c:v",
                "libx264",
                "-g",
                "10",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
            ]
            .into_iter()
            .map(str::to_owned)
            .chain(std::iter::once(original.to_string_lossy().into()))
            .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(generated.success, "{}", generated.stderr);
        let before = std::fs::read(&original).unwrap();
        let mut config = ytui_core::Config::default();
        config.download.directory = temp.path().into();
        let mut request = WorkerRequest {
            job: ytui_core::Job::new(
                ytui_core::Media {
                    id: "abcdefghijk".into(),
                    url: "https://www.youtube.com/watch?v=abcdefghijk".into(),
                    description: "0:00 First\n0:02 Second".into(),
                    ..Default::default()
                },
                config.download.clone(),
            ),
            config,
            cookies: None,
            resolve: false,
            staging: None,
        };
        request.job.options.split_chapters = true;
        let split_source = temp.path().join("split.mp4");
        std::fs::copy(&original, &split_source).unwrap();
        let chapters = apply(&request, &split_source).unwrap();
        assert_eq!(chapters.len(), 2);
        assert_eq!(probe(&request, &split_source).unwrap().chapters.len(), 2);
        assert!(chapters.iter().all(|p| p.is_file()));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        request.job.options.sponsorblock_api = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let mut served = 0;
            let started = Instant::now();
            while served < 2 && started.elapsed() < Duration::from_secs(10) {
                if let Ok((mut stream, _)) = listener.accept() {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut input = [0u8; 4096];
                    let n = stream.read(&mut input).unwrap();
                    let input = String::from_utf8_lossy(&input[..n]);
                    assert!(input.contains("videoID=abcdefghijk"));
                    assert!(!input.to_lowercase().contains("cookie:"));
                    let body = r#"[{"category":"sponsor","segment":[1,2]}]"#;
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                    served += 1;
                } else {
                    thread::sleep(Duration::from_millis(5));
                }
            }
            assert_eq!(served, 2);
        });
        request.job.options.split_chapters = false;
        request.job.options.sponsorblock = vec!["sponsor".into()];
        let marked = temp.path().join("mark.mp4");
        std::fs::copy(&original, &marked).unwrap();
        apply(&request, &marked).unwrap();
        assert!(
            probe(&request, &marked)
                .unwrap()
                .chapters
                .iter()
                .any(|c| c.title.contains("SponsorBlock"))
        );
        request.job.options.sponsorblock_remove = true;
        let cut = temp.path().join("remove.mp4");
        std::fs::copy(&original, &cut).unwrap();
        apply(&request, &cut).unwrap();
        let result = probe(&request, &cut).unwrap();
        assert!(
            (result.duration - 3.0).abs() < 0.15,
            "duration={}",
            result.duration
        );
        assert!(result.chapters.iter().all(|c| c.end.unwrap() <= 3.01));
        server.join().unwrap();
        assert_eq!(std::fs::read(original).unwrap(), before);
    }
}

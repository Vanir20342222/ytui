use crate::{
    paths::Paths,
    store::{Snapshot, backup_legacy},
};
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
use ytui_core::*;

#[derive(Debug, Serialize)]
pub struct Report {
    pub queue_items: usize,
    pub history_items: usize,
    pub imported: usize,
    pub notes: Vec<String>,
}
pub fn import(
    paths: &Paths,
    config: &mut Config,
    snapshot: &mut Snapshot,
    apply: bool,
) -> Result<Report> {
    let mut report = Report {
        queue_items: 0,
        history_items: 0,
        imported: 0,
        notes: vec!["Python files are never modified. Run without --apply to preview.".into()],
    };
    let settings = paths.config.join("settings.json");
    if settings.exists() {
        let data: Value = serde_json::from_str(&std::fs::read_to_string(&settings)?)?;
        config.legacy_settings = Some(serde_json::to_string(&data)?);
        map_settings(&data, config);
        if apply {
            let backup = paths.state.join("legacy-settings.json");
            if !backup.exists() {
                crate::paths::atomic_write(&backup, &std::fs::read(&settings)?)?;
            }
        }
        report.notes.push("Mapped quality, directories, subtitles, metadata, proxy/bandwidth schedule, clipboard, notifications, SponsorBlock, hooks and appearance. Unmapped settings are retained verbatim in legacy_settings and backups.".into());
    }
    let old = paths.legacy_data.join("queue.db");
    if old.exists() {
        let db = Connection::open_with_flags(&old, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        if apply {
            let backup = paths.state.join("legacy-queue-backup.db");
            if !backup.exists() {
                backup_legacy(&old, &backup)?;
            }
        }
        let mut statement=db.prepare("SELECT id,url,state,info_json,output_path,download_mode,quality_override,parent_group_id FROM queue_items ORDER BY priority DESC,created_at ASC")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?;
        for row in rows {
            let (id, url, state, raw, output, mode, quality, group) = row?;
            report.queue_items += 1;
            let uuid = legacy_id(&format!("queue:{id}"));
            if snapshot.downloads.jobs.iter().any(|j| j.id == uuid) {
                continue;
            }
            let info: Value = serde_json::from_str(&raw).unwrap_or_default();
            let media = Media {
                id: info
                    .get("video_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                title: info
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or(&url)
                    .into(),
                creator: info
                    .get("uploader")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                url,
                thumbnail: info
                    .get("thumbnail_url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                duration: info.get("duration").and_then(Value::as_u64),
                ..Default::default()
            };
            let mut options = config.download.clone();
            options.audio = mode == "audio";
            if !quality.is_empty() && !options.audio {
                options.quality = quality;
            }
            let mut job = Job::new(media, options);
            job.id = uuid;
            job.created_at = None;
            job.state = match state.as_str() {
                "paused" => JobState::Paused,
                "done" => JobState::Done,
                "cancelled" => JobState::Cancelled,
                "error" => JobState::Error,
                _ => JobState::Queued,
            };
            if !output.is_empty() {
                job.outputs.push(output.into());
            }
            if !group.is_empty() {
                job.group_id = Some(group);
            }
            snapshot.downloads.jobs.push(job);
            report.imported += 1;
        }
        if let Ok(mut history)=db.prepare("SELECT id,item_id,video_id,title,output_path,url,download_mode FROM download_history ORDER BY completed_at ASC") {
            let rows=history.query_map([],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?)))?;
            for row in rows {let(id,item,video,title,output,url,mode)=row?;report.history_items+=1;
                if snapshot.downloads.jobs.iter().any(|j|j.id==legacy_id(&format!("queue:{item}"))||j.outputs.iter().any(|p|p==Path::new(&output))){continue;}
                let uuid=legacy_id(&format!("history:{id}"));if snapshot.downloads.jobs.iter().any(|j|j.id==uuid){continue;}
                let mut options=config.download.clone();options.audio=mode=="audio";let mut job=Job::new(Media{id:video,title,url,..Default::default()},options);job.id=uuid;job.created_at=None;job.state=JobState::Done;if !output.is_empty(){job.outputs.push(output.into());}snapshot.downloads.jobs.push(job);report.imported+=1;
            }
        }
    } else {
        report
            .notes
            .push("No Python queue database was found.".into());
    }
    if apply {
        if paths.config_file().exists() && !paths.state.join("pre-migration-rust.toml").exists() {
            crate::paths::atomic_write(
                &paths.state.join("pre-migration-rust.toml"),
                &std::fs::read(paths.config_file())?,
            )?;
        }
        let old_keys = paths.config.join("keybindings.json");
        if old_keys.exists() {
            let raw = std::fs::read(&old_keys)?;
            let backup = paths.state.join("legacy-keybindings.json");
            if !backup.exists() {
                crate::paths::atomic_write(&backup, &raw)?;
            }
            let keys: std::collections::BTreeMap<String, String> = serde_json::from_slice(&raw)?;
            config.legacy_keybindings = keys.clone();
            for (action, key) in keys {
                let mapped = match action.as_str() {
                    "focus_input" | "open_search" => "search",
                    "scroll_up" => "up",
                    "scroll_down" => "down",
                    "select_item" => "open",
                    "pause_resume" => "pause",
                    "cancel_item" => "remove",
                    "open_settings" => "settings",
                    "open_help" => "help",
                    "close_panel" => "back",
                    "quit" => "quit",
                    "move_up" => "move_up",
                    "move_down" => "move_down",
                    _ => {
                        report.notes.push(format!("Legacy shortcut {action} was retained in the backup; no equivalent Rust action exists yet"));
                        continue;
                    }
                };
                let normalized = key
                    .replace("ctrl+comma", "ctrl+,")
                    .replace("escape", "esc")
                    .replace("slash", "/");
                if ytui_core::shortcuts::valid_key(&normalized) {
                    config.keybindings.insert(normalized, mapped.into());
                }
            }
        }
        let profiles = paths.config.join("profiles");
        let target = paths.config.join("rust-profiles");
        if profiles.exists() {
            std::fs::create_dir_all(&target)?;
            for entry in std::fs::read_dir(profiles)?.flatten() {
                let p = entry.path();
                if p.extension().is_some_and(|e| e == "json") {
                    let raw = std::fs::read_to_string(&p)?;
                    let value: Value = serde_json::from_str(&raw)?;
                    let mut c = config.clone();
                    c.legacy_settings = Some(serde_json::to_string(&value)?);
                    map_settings(&value, &mut c);
                    let backup = paths
                        .state
                        .join("legacy-profiles")
                        .join(p.file_name().context("Profile has no filename")?);
                    if !backup.exists() {
                        crate::paths::atomic_write(&backup, raw.as_bytes())?;
                    }
                    let dest = target
                        .join(p.file_stem().context("Profile has no name")?)
                        .with_extension("toml");
                    if !dest.exists() {
                        crate::paths::atomic_write(&dest, toml::to_string_pretty(&c)?.as_bytes())?;
                    }
                }
            }
        }
        paths.save_config(config)?;
    }
    report.notes.push("Original settings, profiles and keybindings are preserved, including unmapped values. VPN control and some legacy commands still require parity review; the Rust installer does not replace Python. See docs/PARITY.md.".into());
    Ok(report)
}
fn legacy_id(value: &str) -> uuid::Uuid {
    let hash = Sha256::digest(value);
    let mut bytes = [0; 16];
    bytes.copy_from_slice(&hash[..16]);
    uuid::Uuid::from_bytes(bytes)
}
fn map_settings(v: &Value, c: &mut Config) {
    let string = |path: &str| {
        v.pointer(path)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    for (path, target) in [
        ("/advanced/clipboard_watcher", &mut c.clipboard_watcher),
        ("/advanced/desktop_notifications", &mut c.notifications),
        ("/advanced/notification_sound", &mut c.notification_sound),
        ("/advanced/skip_duplicates", &mut c.download.skip_duplicates),
        ("/advanced/chapter_split", &mut c.download.split_chapters),
    ] {
        if let Some(value) = v.pointer(path).and_then(Value::as_bool) {
            *target = value;
        }
    }
    if v.pointer("/bandwidth_schedule/enabled")
        .and_then(Value::as_bool)
        == Some(true)
    {
        let start = v
            .pointer("/bandwidth_schedule/start_hour")
            .and_then(Value::as_u64)
            .unwrap_or(9);
        let end = v
            .pointer("/bandwidth_schedule/end_hour")
            .and_then(Value::as_u64)
            .unwrap_or(18);
        if start < 24 && end < 24 && start != end {
            c.schedule.bandwidth_window = Some(format!("{start:02}:00-{end:02}:00"));
            c.schedule.window_bandwidth_kib = v
                .pointer("/bandwidth_schedule/limit_kbps")
                .and_then(Value::as_u64)
                .unwrap_or(500);
        }
    }
    if let Some(s) = string("/quality/video_quality") {
        c.download.quality = s;
    }
    if let Some(s) = string("/quality/video_container") {
        c.download.container = s;
    }
    if let Some(s) = string("/quality/audio_container") {
        c.download.audio_container = if s == "ogg" { "vorbis".into() } else { s };
    }
    if let Some(s) = string("/quality/download_mode") {
        c.download.audio = s == "audio";
    }
    if let Some(s) = string("/directories/video_dir") {
        c.download.directory = s.into();
    }
    if let Some(s) = string("/directories/audio_dir") {
        c.download.audio_directory = s.into();
    }
    if let Some(s) = string("/metadata/filename_template") {
        c.download.filename_template = s.replace(".180B", ".180s").replace(".200B", ".200s");
    }
    if let Some(s) = string("/network/proxy_url") {
        c.proxy = Some(s);
    }
    if let Some(n) = v
        .pointer("/network/max_concurrent_downloads")
        .and_then(Value::as_u64)
    {
        c.concurrency = n.clamp(1, 16) as usize;
    }
    if let Some(n) = v
        .pointer("/network/per_download_bandwidth_limit")
        .and_then(Value::as_u64)
    {
        c.download.bandwidth_kib = n;
    }
    if let Some(n) = v
        .pointer("/network/global_bandwidth_limit")
        .and_then(Value::as_u64)
    {
        c.download.global_bandwidth_kib = n;
    }
    if let Some(s) = string("/appearance/theme") {
        c.theme = s;
    }
    if let Some(b) = v
        .pointer("/appearance/show_thumbnails")
        .and_then(Value::as_bool)
    {
        c.thumbnails = b;
    }
    if let Some(b) = v
        .pointer("/metadata/write_metadata")
        .and_then(Value::as_bool)
    {
        c.download.metadata = b;
    }
    if let Some(b) = v
        .pointer("/metadata/embed_thumbnail")
        .and_then(Value::as_bool)
    {
        c.download.thumbnail = b;
    }
    if v.pointer("/subtitles/enabled").and_then(Value::as_bool) == Some(true) {
        c.download.subtitles = v
            .pointer("/subtitles/languages")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
    }
    if let Some(b) = v.pointer("/subtitles/embed").and_then(Value::as_bool) {
        c.download.embed_subtitles = b;
    }
    if v.pointer("/advanced/sponsor_block")
        .and_then(Value::as_bool)
        == Some(true)
    {
        c.download.sponsorblock = v
            .pointer("/advanced/sponsor_block_categories")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        c.download.sponsorblock_remove =
            string("/advanced/sponsor_block_action").is_some_and(|s| s == "skip" || s == "remove");
    }
    if v.pointer("/advanced/enable_custom_script")
        .and_then(Value::as_bool)
        == Some(true)
    {
        c.download.post_download_hook = string("/advanced/custom_script").map(Into::into);
    }
    if let Some(n) = v
        .pointer("/advanced/auto_retry_count")
        .and_then(Value::as_u64)
    {
        c.max_retries = n.min(10) as u32;
    }
    if let Some(n) = v
        .pointer("/advanced/retry_delay_seconds")
        .and_then(Value::as_u64)
    {
        c.retry_delay_seconds = n.min(300);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn repeated_migration_does_not_duplicate_jobs_and_preserves_source() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_override: None,
            xdg_state: t.path().join("state"),
            config: t.path().join("config"),
            state: t.path().join("state"),
            cache: t.path().join("cache"),
            legacy_data: t.path().join("legacy"),
            home: t.path().into(),
        };
        paths.create().unwrap();
        std::fs::create_dir_all(&paths.legacy_data).unwrap();
        let db = Connection::open(paths.legacy_data.join("queue.db")).unwrap();
        db.execute_batch("CREATE TABLE queue_items(id TEXT,url TEXT,state TEXT,info_json TEXT,output_path TEXT,download_mode TEXT,quality_override TEXT,parent_group_id TEXT,priority INTEGER,created_at REAL); INSERT INTO queue_items VALUES('old','https://youtube.com/watch?v=abcdefghijk','paused','{}','','video','','group',1,1);").unwrap();
        drop(db);
        let mut c = Config::default();
        paths.fill_directories(&mut c);
        let mut s = Snapshot::default();
        assert_eq!(import(&paths, &mut c, &mut s, false).unwrap().imported, 1);
        assert_eq!(import(&paths, &mut c, &mut s, false).unwrap().imported, 0);
        assert_eq!(s.downloads.jobs[0].state, JobState::Paused);
        assert!(paths.legacy_data.join("queue.db").exists());
    }
    #[test]
    fn migration_maps_desktop_schedule_and_profiles_without_losing_unknown_values() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths {
            config_override: None,
            xdg_state: t.path().join("state"),
            config: t.path().join("config"),
            state: t.path().join("state/ytui"),
            cache: t.path().join("cache"),
            legacy_data: t.path().join("data"),
            home: t.path().into(),
        };
        paths.create().unwrap();
        std::fs::create_dir_all(paths.config.join("profiles")).unwrap();
        let raw = r#"{"advanced":{"clipboard_watcher":true,"desktop_notifications":false,"notification_sound":true,"future_switch":42},"bandwidth_schedule":{"enabled":true,"start_hour":22,"end_hour":6,"limit_kbps":123},"future_group":{"value":"keep"}}"#;
        std::fs::write(paths.config.join("settings.json"), raw).unwrap();
        std::fs::write(paths.config.join("profiles/Music.json"), raw).unwrap();
        std::fs::write(
            paths.config.join("keybindings.json"),
            r#"{"open_help":"f8","future_action":"f9"}"#,
        )
        .unwrap();
        let mut config = Config::default();
        paths.fill_directories(&mut config);
        let mut snapshot = Snapshot::default();
        import(&paths, &mut config, &mut snapshot, true).unwrap();
        assert!(config.clipboard_watcher);
        assert!(!config.notifications);
        assert!(config.notification_sound);
        assert_eq!(
            config.schedule.bandwidth_window.as_deref(),
            Some("22:00-06:00")
        );
        assert_eq!(config.schedule.window_bandwidth_kib, 123);
        assert_eq!(
            config.legacy_keybindings.get("future_action").unwrap(),
            "f9"
        );
        let legacy: Value =
            serde_json::from_str(config.legacy_settings.as_deref().unwrap()).unwrap();
        assert_eq!(legacy.pointer("/advanced/future_switch").unwrap(), 42);
        let profile = crate::profiles::load(&paths, "Music").unwrap();
        assert_eq!(profile.legacy_settings, config.legacy_settings);
        assert_eq!(
            std::fs::read_to_string(paths.state.join("legacy-profiles/Music.json")).unwrap(),
            raw
        );
        assert_eq!(
            std::fs::read_to_string(paths.config.join("settings.json")).unwrap(),
            raw
        );
    }
}

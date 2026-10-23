//! Native WQHD capture tool: actual Rust executable PTY output, synthetic data.
#[path = "../tests/support/terminal.rs"]
mod terminal;
use anyhow::{Context, Result, ensure};
use clap::Parser;
use fontdue::{Font, FontSettings};
use image::{Rgb, RgbImage};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};
use terminal::Capture;
use ytui_core::{
    BrowseRequest, DownloadOptions, Job, JobState, Media, MediaKind, playback_history::Position,
    watch::CollectionWatch,
};
use ytui_services::store::{Snapshot, Store};

#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "target/debug/ytui")]
    binary: PathBuf,
    /// Refresh only the three existing download-form screenshots.
    #[arg(long)]
    downloads_only: bool,
    /// Refresh only the audit dialogs, without recapturing every theme/workspace.
    #[arg(long, conflicts_with = "downloads_only")]
    audit_panels_only: bool,
    /// Refresh German navigation/settings and the local lyrics panel.
    #[arg(long, conflicts_with_all = ["downloads_only", "audit_panels_only"])]
    comfort_panels_only: bool,
    #[arg(long, default_value = "docs/screenshots")]
    output: PathBuf,
    #[arg(
        long,
        default_value = "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf"
    )]
    font: PathBuf,
}
fn color(color: vt100::Color, background: bool) -> [u8; 3] {
    match color {
        vt100::Color::Default => {
            if background {
                [16, 20, 24]
            } else {
                [229, 237, 244]
            }
        }
        vt100::Color::Rgb(r, g, b) => [r, g, b],
        vt100::Color::Idx(n) => {
            const ANSI: [[u8; 3]; 16] = [
                [16, 20, 24],
                [255, 127, 134],
                [142, 221, 153],
                [245, 195, 108],
                [122, 162, 247],
                [203, 166, 247],
                [109, 219, 193],
                [229, 237, 244],
                [82, 104, 123],
                [255, 156, 162],
                [166, 227, 161],
                [249, 226, 175],
                [137, 180, 250],
                [224, 182, 255],
                [148, 226, 213],
                [255, 255, 255],
            ];
            if n < 16 {
                ANSI[n as usize]
            } else if n < 232 {
                let n = n - 16;
                let v = |x: u8| if x == 0 { 0 } else { 55 + 40 * x };
                [v(n / 36), v(n / 6 % 6), v(n % 6)]
            } else {
                let v = 8 + (n - 232) * 10;
                [v, v, v]
            }
        }
    }
}
fn shot(capture: &mut Capture, path: &Path, font: &Font) -> Result<()> {
    // Drain complete redraws after asynchronous demo image/protocol work.
    // Readiness no longer depends on the removed loading-placeholder text.
    capture.read_for(Duration::from_millis(900));
    let screen = capture.parser.screen();
    let (rows, cols) = screen.size();
    let cw = (2544 / u32::from(cols)).min(1392 / (2 * u32::from(rows)));
    let ch = cw * 2;
    let left = (2560 - u32::from(cols) * cw) / 2;
    let top = (1440 - u32::from(rows) * ch) / 2;
    let mut image = RgbImage::from_pixel(2560, 1440, Rgb([16, 20, 24]));
    let mut glyphs = HashMap::new();
    for row in 0..rows {
        for col in 0..cols {
            let Some(cell) = screen.cell(row, col) else {
                continue;
            };
            let mut fg = color(cell.fgcolor(), false);
            let mut bg = color(cell.bgcolor(), true);
            if cell.inverse() {
                std::mem::swap(&mut fg, &mut bg);
            }
            let x = left + u32::from(col) * cw;
            let y = top + u32::from(row) * ch;
            for py in y..y + ch {
                for px in x..x + cw {
                    image.put_pixel(px, py, Rgb(bg));
                }
            }
            if cell.is_wide_continuation() {
                continue;
            }
            for character in cell.contents().chars().filter(|c| !c.is_whitespace()) {
                let (metrics, bitmap) = glyphs
                    .entry(character)
                    .or_insert_with(|| font.rasterize(character, ch as f32 * 0.77));
                let gx = x as i32 + metrics.xmin;
                let gy =
                    y as i32 + (ch as f32 * 0.82) as i32 - metrics.height as i32 - metrics.ymin;
                for (offset, alpha) in bitmap.iter().enumerate() {
                    if *alpha == 0 {
                        continue;
                    }
                    let px = gx + (offset % metrics.width) as i32;
                    let py = gy + (offset / metrics.width) as i32;
                    if px < 0 || py < 0 || px >= 2560 || py >= 1440 {
                        continue;
                    }
                    let base = image.get_pixel(px as u32, py as u32).0;
                    let a = u32::from(*alpha);
                    let blend = std::array::from_fn(|i| {
                        ((u32::from(fg[i]) * a + u32::from(base[i]) * (255 - a)) / 255) as u8
                    });
                    image.put_pixel(px as u32, py as u32, Rgb(blend));
                }
            }
        }
    }
    std::fs::create_dir_all(path.parent().context("Screenshot parent")?)?;
    image.save(path)?;
    Ok(())
}
async fn seed_offline(root: &Path) -> Result<()> {
    std::fs::create_dir_all(root)?;
    let file = root.join("Studio Sessions.mp3");
    std::fs::write(&file, b"Synthetic UI fixture; never played")?;
    let media = Media {
        id: "local-fixture".into(),
        title: "Studio Sessions · A quiet evening".into(),
        creator: "Fictional catalog".into(),
        url: file.display().to_string(),
        kind: MediaKind::Song,
        duration: Some(240),
        ..Default::default()
    };
    let options = DownloadOptions {
        audio: true,
        ..Default::default()
    };
    let mut job = Job::new(media.clone(), options.clone());
    job.state = JobState::Done;
    job.outputs.push(file);
    let snapshot = Snapshot {
        downloads: ytui_core::DownloadQueue { jobs: vec![job] },
        positions: ytui_core::playback_history::PlaybackHistory {
            entries: vec![Position {
                media,
                seconds: 82.,
                duration: Some(240.),
                updated_at: 1,
                finished: false,
            }],
        },
        watches: vec![CollectionWatch {
            id: uuid::Uuid::new_v4(),
            title: "Studio Sessions · New releases".into(),
            request: BrowseRequest::Collection("VLfixture".into()),
            music: true,
            options,
            interval_seconds: 1800,
            next_check: 0,
            enabled: true,
            generation: 0,
            owner: "anonymous".into(),
            seen: None,
            failures: 0,
            error: None,
        }],
        ..Default::default()
    };
    let store = Store::open(&root.join("state"))?;
    store.save(snapshot).await?;
    store.close().await?;
    Ok(())
}
fn write_manifest(args: &Args) -> Result<()> {
    let mut captures = Vec::new();
    let mut dirs = vec![args.output.clone()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "png") {
                ensure!(
                    image::image_dimensions(&path)? == (2560, 1440),
                    "Not WQHD: {}",
                    path.display()
                );
                captures.push(path.strip_prefix(&args.output)?.display().to_string());
            }
        }
    }
    captures.sort();
    let manifest = serde_json::json!({"source":"Actual Rust executable PTY output captured by native Rust tooling; fictional demo/offline data and generated artwork","binary":args.binary,"size":[2560,1440],"terminal":[212,58],"compact_terminal":[80,24],"themes":ytui_services::theme::THEMES,"captures":captures,"real_desktop_capture":false});
    std::fs::write(
        args.output.join("manifest.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )?;
    println!("{} WQHD screenshots captured with Rust", captures.len());
    Ok(())
}
#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let binary = args.binary.canonicalize()?;
    let font = Font::from_bytes(std::fs::read(&args.font)?, FontSettings::default())
        .map_err(|e| anyhow::anyhow!("Cannot load font: {e}"))?;
    let temp = tempfile::tempdir()?;
    let mut capture = Capture::new(&binary, &temp.path().join("demo"), false)?;
    if args.comfort_panels_only {
        let file = temp.path().join("fictional song.lrc");
        std::fs::write(
            &file,
            "[ar:Fictional studio]\n[00:01]A quiet morning in the city\n[00:06]The sunlight finds an open door\n[00:11]We keep the rhythm of the river\n[00:16]And let the next song carry on\n",
        )?;
        capture.command(
            &format!("lyrics-local {}", file.display()),
            "A quiet morning",
        )?;
        shot(&mut capture, &args.output.join("lyrics.png"), &font)?;
        capture.close()?;
        let mut capture =
            Capture::with_language(&binary, &temp.path().join("german"), false, "de")?;
        shot(&mut capture, &args.output.join("german-youtube.png"), &font)?;
        capture.command("settings", "Allgemeine Einstellungen")?;
        shot(
            &mut capture,
            &args.output.join("german-settings.png"),
            &font,
        )?;
        capture.send("\x1b")?;
        capture.send(":")?;
        capture.send("Liedtext")?;
        shot(&mut capture, &args.output.join("german-actions.png"), &font)?;
        capture.close()?;
        write_manifest(&args)?;
        return Ok(());
    }
    if args.audit_panels_only {
        for (command, name, expected) in [
            ("player-formats", "player-formats", "Available formats"),
            (
                "sponsor-status",
                "sponsorblock-status",
                "SponsorBlock & ad handling",
            ),
            (
                "download-recent",
                "recent-folders",
                "Recent download folders",
            ),
            ("capabilities", "capabilities", "Setup & capabilities"),
            ("log", "diagnostics", "Diagnostics"),
            ("thumbnail-cache", "thumbnail-cache", "Thumbnail cache"),
        ] {
            capture.command(command, expected)?;
            shot(
                &mut capture,
                &args.output.join(format!("{name}.png")),
                &font,
            )?;
            capture.send("\x1b")?;
        }
        capture.close()?;
        let offline = temp.path().join("offline");
        seed_offline(&offline).await?;
        let mut capture = Capture::new(&binary, &offline, true)?;
        for (command, name, expected) in [
            ("download-files", "download-files", "Files ·"),
            ("auth", "account-setup", "YouTube account"),
        ] {
            capture.command(command, expected)?;
            shot(
                &mut capture,
                &args.output.join(format!("{name}.png")),
                &font,
            )?;
            capture.send("\x1b")?;
        }
        capture.close()?;
        write_manifest(&args)?;
        return Ok(());
    }
    if args.downloads_only {
        capture.command(
            "add https://www.youtube.com/watch?v=fixture1234",
            "Video quality",
        )?;
        for name in [
            "download-options",
            "download-destination",
            "download-captions",
        ] {
            shot(
                &mut capture,
                &args.output.join(format!("{name}.png")),
                &font,
            )?;
            capture.send("\x1b[6~")?;
        }
        capture.send("\x1b")?;
        capture.close()?;
        write_manifest(&args)?;
        println!("3 WQHD download-form screenshots refreshed");
        return Ok(());
    }

    for theme in ytui_services::theme::THEMES {
        capture.command(&format!("theme {theme}"), theme)?;
        capture.read_for(Duration::from_millis(350));
        shot(
            &mut capture,
            &args.output.join(format!("themes/{theme}.png")),
            &font,
        )?;
    }
    capture.command("theme ytui-dark", "ytui-dark")?;
    for (tab, name) in [
        ("1", "downloads"),
        ("2", "youtube"),
        ("3", "music"),
        ("4", "radio"),
    ] {
        capture.send(tab)?;
        shot(
            &mut capture,
            &args.output.join(format!("{name}.png")),
            &font,
        )?;
    }
    capture.send("2:")?;
    shot(&mut capture, &args.output.join("actions.png"), &font)?;
    capture.send("\x1b")?;
    capture.send("o")?;
    shot(
        &mut capture,
        &args.output.join("player-controls.png"),
        &font,
    )?;
    capture.send("\x1b")?;
    capture.command("categories", "Feed categories")?;
    shot(&mut capture, &args.output.join("categories.png"), &font)?;
    capture.send("\x1b/")?;
    capture.send("Studio sessions")?;
    shot(&mut capture, &args.output.join("search.png"), &font)?;
    capture.send("\x1b")?;
    capture.send("2b")?;
    shot(&mut capture, &args.output.join("feeds.png"), &font)?;
    capture.send("\x1b")?;
    for (command, name, expected) in [
        ("help", "help", "Keyboard & navigation"),
        ("stats", "history", "Downloads · all"),
        ("keys", "keybindings", "Keyboard shortcuts"),
        ("settings", "settings", "General settings"),
        ("auth", "auth", "Connect your YouTube account"),
        ("theme", "theme-picker", "Themes"),
        ("groups", "groups", "Download groups"),
        ("log", "diagnostics", "Diagnostics"),
        ("update", "update", "Update ytui"),
        ("player-formats", "player-formats", "Available formats"),
        (
            "sponsor-status",
            "sponsorblock-status",
            "SponsorBlock & ad handling",
        ),
        (
            "download-recent",
            "recent-folders",
            "Recent download folders",
        ),
        ("capabilities", "capabilities", "Setup & capabilities"),
        ("thumbnail-cache", "thumbnail-cache", "Thumbnail cache"),
        (
            "add https://www.youtube.com/watch?v=fixture1234",
            "download-options",
            "Video quality",
        ),
    ] {
        capture.command(command, expected)?;
        shot(
            &mut capture,
            &args.output.join(format!("{name}.png")),
            &font,
        )?;
        capture.send("\x1b")?;
    }
    capture.command(
        "add https://www.youtube.com/watch?v=fixture1234",
        "Video quality",
    )?;
    capture.send("\x1b[6~")?;
    shot(
        &mut capture,
        &args.output.join("download-destination.png"),
        &font,
    )?;
    capture.send("\x1b[6~")?;
    shot(
        &mut capture,
        &args.output.join("download-captions.png"),
        &font,
    )?;
    capture.send("\x1b")?;
    capture.send("p")?;
    shot(&mut capture, &args.output.join("playback-queue.png"), &font)?;
    capture.send("\x1b")?;
    capture.command("settings", "General settings")?;
    for name in [
        "general",
        "appearance",
        "downloads",
        "subtitles",
        "network",
        "schedule",
        "tools",
        "radio",
        "playback",
    ] {
        shot(
            &mut capture,
            &args.output.join(format!("settings/{name}.png")),
            &font,
        )?;
        capture.send("\x1b[6~")?;
    }
    capture.send("\x1b")?;
    capture.resize(80, 24)?;
    shot(&mut capture, &args.output.join("compact.png"), &font)?;
    capture.command("settings", "General settings")?;
    shot(
        &mut capture,
        &args.output.join("settings/compact.png"),
        &font,
    )?;
    capture.send("\x1b")?;
    capture.resize(35, 12)?;
    capture.command("settings", "General settings")?;
    ensure!(
        capture.text().contains("General settings"),
        "Tiny settings window disappeared"
    );
    capture.send("\x1b")?;
    capture.resize(212, 58)?;
    capture.close()?;
    let offline = temp.path().join("offline");
    seed_offline(&offline).await?;
    let mut capture = Capture::new(&binary, &offline, true)?;
    shot(&mut capture, &args.output.join("offline.png"), &font)?;
    for (command, name) in [("resume", "resume"), ("watches", "watches")] {
        capture.command(
            command,
            if command == "resume" {
                "Continue watching"
            } else {
                "Collection"
            },
        )?;
        shot(
            &mut capture,
            &args.output.join(format!("{name}.png")),
            &font,
        )?;
        capture.send("\x1b")?;
    }
    capture.command("download-files", "Files ·")?;
    shot(&mut capture, &args.output.join("download-files.png"), &font)?;
    capture.send("\x1b")?;
    capture.command("auth", "YouTube account")?;
    shot(&mut capture, &args.output.join("account-setup.png"), &font)?;
    capture.send("\x1b")?;
    capture.close()?;
    std::fs::copy(
        args.output.join("themes/ytui-light.png"),
        args.output.join("light.png"),
    )?;
    write_manifest(&args)?;
    Ok(())
}

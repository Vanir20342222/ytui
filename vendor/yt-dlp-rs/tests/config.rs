//! Configuration files, compared with pinned upstream `parseOpts` by hand
//! (upstream reads `yt-dlp` names; this port reads `yt-dlp-rs` names).
use std::{path::Path, process::Command};

struct Tree {
    root: tempfile::TempDir,
}

impl Tree {
    fn new(files: &[(&str, &str)]) -> Self {
        let root = tempfile::tempdir().unwrap();
        for directory in ["home", "xdg", "cwd"] {
            std::fs::create_dir_all(root.path().join(directory)).unwrap();
        }
        for (relative, contents) in files {
            let path = root.path().join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
        Self { root }
    }

    fn path(&self, relative: &str) -> std::path::PathBuf {
        self.root.path().join(relative)
    }

    /// `--get-filename` for an unreachable direct URL is not possible, so use
    /// `--print` of the output template with `--load-info-json`.
    fn filename(&self, args: &[&str]) -> (Option<i32>, String, String) {
        let info = self.path("info.json");
        std::fs::write(
            &info,
            r#"{"id":"clip","title":"Clip title","ext":"mp4","url":"http://127.0.0.1:1/clip.mp4","webpage_url":"http://127.0.0.1:1/clip.mp4","extractor":"generic","extractor_key":"Generic"}"#,
        )
        .unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .env("HOME", self.path("home"))
            .env("XDG_CONFIG_HOME", self.path("xdg"))
            .env_remove("APPDATA")
            .env_remove("appdata")
            .args(args)
            .arg("--get-filename")
            .arg("--load-info-json")
            .arg(&info)
            .current_dir(self.path("cwd"))
            .output()
            .unwrap();
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).trim().to_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    }
}

fn name(tree: &Tree, args: &[&str]) -> String {
    let (code, stdout, stderr) = tree.filename(args);
    assert_eq!(code, Some(0), "{stderr}");
    Path::new(&stdout)
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn user_configuration_is_found_like_upstream_and_the_command_line_wins() {
    let tree = Tree::new(&[("xdg/yt-dlp-rs/config", "-o \"user-%(id)s.%(ext)s\"")]);
    assert_eq!(name(&tree, &[]), "user-clip.mp4");
    assert_eq!(name(&tree, &["-o", "cli.%(ext)s"]), "cli.mp4");
    for flag in ["--ignore-config", "--no-config"] {
        assert_eq!(name(&tree, &[flag]), "Clip title [clip].mp4");
    }
    // `XDG/yt-dlp-rs.conf` precedes `XDG/yt-dlp-rs/config`.
    let beside = Tree::new(&[
        ("xdg/yt-dlp-rs.conf", "-o beside.%(ext)s"),
        ("xdg/yt-dlp-rs/config", "-o inner.%(ext)s"),
    ]);
    assert_eq!(name(&beside, &[]), "beside.mp4");
    let dot = Tree::new(&[("home/.yt-dlp-rs/config.txt", "-o dot.%(ext)s")]);
    assert_eq!(name(&dot, &[]), "dot.mp4");
}

#[test]
fn home_configuration_follows_paths_and_overrides_the_user_one() {
    let tree = Tree::new(&[
        ("cwd/yt-dlp-rs.conf", "-o cwd.%(ext)s"),
        ("xdg/yt-dlp-rs/config", "-o user.%(ext)s"),
        ("cwd/dl/yt-dlp-rs.conf", "-o pconf.%(ext)s"),
    ]);
    assert_eq!(name(&tree, &[]), "cwd.mp4");
    // `-P dl` makes `dl/yt-dlp-rs.conf` the home configuration.
    assert_eq!(name(&tree, &["-P", "dl"]), "pconf.mp4");
    // `--ignore-config` inside a loaded configuration stops later ones.
    let ignore = Tree::new(&[
        ("cwd/yt-dlp-rs.conf", "--ignore-config -o cwd.%(ext)s"),
        ("xdg/yt-dlp-rs/config", "-o user.%(ext)s"),
    ]);
    assert_eq!(name(&ignore, &[]), "cwd.mp4");
}

#[test]
fn config_locations_nest_reset_and_validate_like_upstream() {
    let tree = Tree::new(&[
        (
            "cwd/a.conf",
            "# comment\n--config-locations sub/b.conf\n-o \"a %(id)s.%(ext)s\"  # trailing",
        ),
        ("cwd/sub/b.conf", "-o b.%(ext)s"),
        ("cwd/c.conf", "-o c.%(ext)s"),
        ("cwd/d/yt-dlp-rs.conf", "-o dir.%(ext)s"),
        (
            "cwd/reset.conf",
            "--config-locations c.conf --no-config-locations\n-o reset.%(ext)s",
        ),
        (
            "cwd/self.conf",
            "--config-locations self.conf -o self.%(ext)s",
        ),
    ]);
    // A file's own arguments follow (override) the files it names.
    assert_eq!(name(&tree, &["--config-locations", "a.conf"]), "a clip.mp4");
    assert_eq!(name(&tree, &["--config-locations", "sub/b.conf"]), "b.mp4");
    // Earlier locations take precedence over later ones.
    assert_eq!(
        name(
            &tree,
            &[
                "--config-locations",
                "a.conf",
                "--config-locations",
                "c.conf"
            ]
        ),
        "a clip.mp4"
    );
    assert_eq!(name(&tree, &["--config-locations", "d"]), "dir.mp4");
    assert_eq!(
        name(&tree, &["--config-locations", "reset.conf"]),
        "reset.mp4"
    );
    assert_eq!(
        name(&tree, &["--config-locations", "self.conf"]),
        "self.mp4"
    );
    let (code, _, stderr) = tree.filename(&["--config-locations", "nope.conf"]);
    assert_eq!(code, Some(2));
    assert!(
        stderr.ends_with("yt-dlp-rs: error: config location nope.conf does not exist\n"),
        "{stderr}"
    );
}

#[test]
fn unparsable_configuration_is_a_usage_error() {
    let tree = Tree::new(&[("xdg/yt-dlp-rs/config", "-o \"broken")]);
    let (code, _, stderr) = tree.filename(&[]);
    assert_eq!(code, Some(2));
    assert!(stderr.contains("No closing quotation"), "{stderr}");
    assert!(stderr.contains("Unable to parse \""), "{stderr}");
}

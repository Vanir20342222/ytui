#![cfg(unix)]
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    process::Command,
};
fn executable(path: &Path, content: &str) {
    fs::write(path, content).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
#[test]
fn rust_installation_replaces_launchers_and_preserves_legacy_targets_and_failed_builds() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let bin = root.join("bin");
    let fake = root.join("tools");
    let build = root.join("build");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(&fake).unwrap();
    fs::create_dir_all(build.join("release")).unwrap();
    let legacy = root.join("legacy-venv-launcher");
    let original = "#!/bin/sh\nprintf legacy\n";
    executable(&legacy, original);
    symlink(&legacy, bin.join("ytui")).unwrap();
    fs::copy(env!("CARGO_BIN_EXE_ytui"), build.join("release/ytui")).unwrap();
    let cargo = fake.join("cargo");
    executable(&cargo, "#!/bin/sh\nexit 17\n");
    let run = |script: &str| {
        Command::new("bash")
            .arg(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../..")
                    .join(script),
            )
            .env("PATH", format!("{}:/usr/bin:/bin", fake.display()))
            .env("YTUI_INSTALL_DIR", &bin)
            .env("CARGO_TARGET_DIR", &build)
            .output()
            .unwrap()
    };
    assert!(!run("install.sh").status.success());
    assert!(
        fs::symlink_metadata(bin.join("ytui"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read_to_string(&legacy).unwrap(), original);
    executable(&cargo, "#!/bin/sh\nexit 0\n");
    for script in ["install.sh", "install-rust.sh"] {
        let result = run(script);
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        for name in ["ytui", "ytui-rs"] {
            assert!(
                !fs::symlink_metadata(bin.join(name))
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            let version = Command::new(bin.join(name))
                .arg("--version")
                .output()
                .unwrap();
            assert!(version.status.success());
            assert!(String::from_utf8_lossy(&version.stdout).contains(env!("CARGO_PKG_VERSION")));
        }
    }
    assert_eq!(fs::read_to_string(&legacy).unwrap(), original);
}

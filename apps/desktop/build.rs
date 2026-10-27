use std::{
    path::{Path, PathBuf},
    process::Command,
};
fn git(root: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
fn git_path(root: &Path, name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(git(root, &["rev-parse", "--git-path", name])?);
    Some(if path.is_absolute() {
        path
    } else {
        root.join(path)
    })
}
fn main() {
    println!("cargo:rerun-if-env-changed=YTUI_BUILD_REVISION");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for source in ["crates", "Cargo.toml", "Cargo.lock"] {
        println!("cargo:rerun-if-changed={}", root.join(source).display());
    }
    if let Some(head) = git_path(&root, "HEAD") {
        println!("cargo:rerun-if-changed={}", head.display());
        if let Ok(head) = std::fs::read_to_string(head)
            && let Some(reference) = head.trim().strip_prefix("ref: ")
        {
            if let Some(reference) = git_path(&root, reference) {
                println!("cargo:rerun-if-changed={}", reference.display());
            }
            if let Some(packed) = git_path(&root, "packed-refs") {
                println!("cargo:rerun-if-changed={}", packed.display());
            }
        }
    }
    let revision = std::env::var("YTUI_BUILD_REVISION")
        .ok()
        .or_else(|| git(&root, &["rev-parse", "HEAD"]))
        .filter(|value| value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=YTUI_BUILD_REVISION={revision}");
    let dirty = git(
        &root,
        &[
            "status",
            "--porcelain",
            "--untracked-files=normal",
            "--",
            "Cargo.toml",
            "Cargo.lock",
            "crates",
        ],
    )
    .is_some_and(|status| !status.is_empty());
    println!(
        "cargo:rustc-env=YTUI_BUILD_DESCRIPTION={revision}{}",
        if dirty { "-dirty" } else { "" }
    );
}

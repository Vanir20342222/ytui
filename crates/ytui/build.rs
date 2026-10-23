use std::{path::Path, process::Command};
fn main() {
    println!("cargo:rerun-if-env-changed=YTUI_BUILD_REVISION");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let git = root.join(".git");
    println!("cargo:rerun-if-changed={}", git.join("HEAD").display());
    if let Ok(head) = std::fs::read_to_string(git.join("HEAD"))
        && let Some(reference) = head.trim().strip_prefix("ref: ")
    {
        println!("cargo:rerun-if-changed={}", git.join(reference).display());
        println!(
            "cargo:rerun-if-changed={}",
            git.join("packed-refs").display()
        );
    }
    let revision = std::env::var("YTUI_BUILD_REVISION")
        .ok()
        .or_else(|| {
            let output = Command::new("git")
                .args(["rev-parse", "HEAD"])
                .current_dir(root)
                .output()
                .ok()?;
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        })
        .filter(|value| value.len() == 40 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=YTUI_BUILD_REVISION={revision}");
}

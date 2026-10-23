//! Build-time license inventory; does not inspect user configuration or media.
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::PathBuf;
fn main() -> Result<()> {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    ensure!(
        arguments.len() == 2,
        "Usage: license_bundle metadata.json NEW_DIRECTORY"
    );
    let metadata: Value = serde_json::from_slice(&std::fs::read(&arguments[0])?)?;
    let output = PathBuf::from(&arguments[1]);
    std::fs::create_dir(&output).context("License output directory must be new")?;
    let mut inventory = Vec::new();
    for package in metadata["packages"]
        .as_array()
        .context("Missing dependency packages")?
    {
        let name = package["name"].as_str().context("Missing package name")?;
        let version = package["version"]
            .as_str()
            .context("Missing package version")?;
        ensure!(
            name.bytes()
                .chain(version.bytes())
                .all(|b| b.is_ascii_alphanumeric() || b"_-.+".contains(&b)),
            "Invalid package identity"
        );
        let root = PathBuf::from(
            package["manifest_path"]
                .as_str()
                .context("Missing dependency root")?,
        )
        .parent()
        .context("Missing root")?
        .to_owned();
        let destination = output.join(format!("{name}-{version}"));
        std::fs::create_dir(&destination)?;
        let explicit = package["license_file"].as_str();
        let mut copied = Vec::new();
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let filename = entry.file_name().to_string_lossy().into_owned();
            let upper = filename.to_ascii_uppercase();
            if upper.starts_with("LICENSE")
                || upper.starts_with("LICENCE")
                || upper.starts_with("COPYING")
                || upper.starts_with("NOTICE")
                || explicit == Some(filename.as_str())
            {
                ensure!(
                    entry.metadata()?.len() <= 1024 * 1024,
                    "Oversized license file"
                );
                std::fs::copy(entry.path(), destination.join(&filename))?;
                copied.push(filename);
            }
        }
        // A custom license file can live below the package root.
        if let Some(relative) = explicit {
            let source = root.join(relative).canonicalize()?;
            ensure!(
                source.starts_with(root.canonicalize()?),
                "License file escapes dependency root"
            );
            if !copied.iter().any(|filename| filename == relative) {
                std::fs::copy(source, destination.join("DECLARED-LICENSE"))?;
                copied.push("DECLARED-LICENSE".into());
            }
        }
        copied.sort();
        inventory.push(
            json!({"name":name, "version":version, "license":package["license"], "notices":copied}),
        );
    }
    inventory.sort_by_key(|package| format!("{}:{}", package["name"], package["version"]));
    std::fs::write(
        output.join("inventory.json"),
        serde_json::to_vec_pretty(&inventory)?,
    )?;
    let missing = inventory
        .iter()
        .filter(|package| package["notices"].as_array().is_none_or(Vec::is_empty))
        .count();
    println!(
        "Collected {} dependency inventories; {missing} have SPDX metadata only and require release review",
        inventory.len()
    );
    Ok(())
}

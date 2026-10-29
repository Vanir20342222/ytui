use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};
fn pngs(dir: &Path, output: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            pngs(&path, output)
        } else if path.extension().is_some_and(|e| e == "png") {
            output.push(path);
        }
    }
}
#[test]
fn desktop_screenshots_are_wqhd_and_listed_in_the_manifest() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
    let mut paths = Vec::new();
    pngs(&dir, &mut paths);
    paths.retain(|path| !path.starts_with(dir.join("android")));
    assert!(!paths.is_empty());
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let mut listed = manifest["captures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    listed.sort();
    let mut actual = Vec::new();
    for path in paths {
        let mut header = [0; 24];
        fs::File::open(&path)
            .unwrap()
            .read_exact(&mut header)
            .unwrap();
        assert_eq!(&header[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            u32::from_be_bytes(header[16..20].try_into().unwrap()),
            2560,
            "{}",
            path.display()
        );
        assert_eq!(
            u32::from_be_bytes(header[20..24].try_into().unwrap()),
            1440,
            "{}",
            path.display()
        );
        actual.push(path.strip_prefix(&dir).unwrap().display().to_string());
    }
    actual.sort();
    assert_eq!(actual, listed);
}

#[test]
fn android_screenshots_keep_the_native_viewport_recorded_in_their_own_manifest() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots/android");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).unwrap()).unwrap();
    let mut paths = Vec::new();
    pngs(&dir, &mut paths);
    let mut actual = paths
        .iter()
        .map(|p| p.strip_prefix(&dir).unwrap().display().to_string())
        .collect::<Vec<_>>();
    let mut listed = Vec::new();
    for capture in manifest["captures"].as_array().unwrap() {
        let file = capture["file"].as_str().unwrap();
        assert!(!file.contains("..") && !file.starts_with('/'));
        let mut header = [0; 24];
        fs::File::open(dir.join(file))
            .unwrap()
            .read_exact(&mut header)
            .unwrap();
        assert_eq!(&header[..8], b"\x89PNG\r\n\x1a\n");
        for (key, start) in [("width", 16), ("height", 20)] {
            let size = u32::from_be_bytes(header[start..start + 4].try_into().unwrap());
            assert!((720..=8192).contains(&size));
            assert_eq!(
                u64::from(size),
                capture[key].as_u64().unwrap(),
                "{file} {key}"
            );
        }
        listed.push(file.to_owned());
    }
    actual.sort();
    listed.sort();
    assert!(!actual.is_empty());
    assert_eq!(actual, listed);
}

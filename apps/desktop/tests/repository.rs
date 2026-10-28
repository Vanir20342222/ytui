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
fn every_committed_screenshot_is_wqhd_and_listed_in_the_manifest() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/screenshots");
    let mut paths = Vec::new();
    pngs(&dir, &mut paths);
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

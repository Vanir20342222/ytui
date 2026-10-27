//! Upstream `XAttrMetadataPP` (`--xattrs`): Dublin Core and XDG extended
//! attributes (NTFS alternate data streams on Windows) from the info dict.
use crate::{Error, Result, VideoInfo};
use std::path::Path;

/// Upstream `XATTR_MAPPING`, in its write order.
const MAPPING: [(&str, &str); 7] = [
    ("user.xdg.referrer.url", "webpage_url"),
    ("user.dublincore.title", "title"),
    ("user.dublincore.date", "upload_date"),
    ("user.dublincore.contributor", "uploader"),
    ("user.dublincore.format", "format"),
    // Written late: it may approach the filesystem's xattr size limit.
    ("user.dublincore.description", "description"),
    ("com.apple.metadata:kMDItemWhereFroms", "webpage_url"),
];

const APPLE_PLIST: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<array>\n\t<string>%s</string>\n</array>\n</plist>";

/// Upstream `XAttrMetadataError.reason`.
enum Failure {
    NoSpace,
    TooLong,
    NotSupported,
}

fn classify(error: &std::io::Error) -> Failure {
    let text = error.to_string();
    match error.raw_os_error() {
        // ENOSPC (28) and EDQUOT (Linux 122, macOS 69).
        Some(28) => Failure::NoSpace,
        Some(122) if cfg!(target_os = "linux") => Failure::NoSpace,
        Some(69) if cfg!(target_os = "macos") => Failure::NoSpace,
        // E2BIG.
        Some(7) => Failure::TooLong,
        _ if text.contains("No space left") || text.contains("Disk quota exceeded") => {
            Failure::NoSpace
        }
        _ if text.contains("Argument list too long") => Failure::TooLong,
        _ => Failure::NotSupported,
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn set(path: &Path, name: &str, value: &[u8]) -> std::io::Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let path = CString::new(path.as_os_str().as_bytes())?;
    let name = CString::new(name)?;
    unsafe extern "C" {
        #[cfg(target_os = "linux")]
        fn setxattr(
            path: *const std::ffi::c_char,
            name: *const std::ffi::c_char,
            value: *const std::ffi::c_void,
            size: usize,
            flags: std::ffi::c_int,
        ) -> std::ffi::c_int;
        #[cfg(target_os = "macos")]
        fn setxattr(
            path: *const std::ffi::c_char,
            name: *const std::ffi::c_char,
            value: *const std::ffi::c_void,
            size: usize,
            position: u32,
            options: std::ffi::c_int,
        ) -> std::ffi::c_int;
    }
    // SAFETY: valid NUL-terminated strings and a live value buffer.
    let status = unsafe {
        #[cfg(target_os = "linux")]
        {
            setxattr(
                path.as_ptr(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
            )
        }
        #[cfg(target_os = "macos")]
        {
            setxattr(
                path.as_ptr(),
                name.as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
                0,
            )
        }
    };
    if status == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// Upstream `write_xattr` on Windows: `PATH:KEY` alternate data streams.
#[cfg(windows)]
fn set(path: &Path, name: &str, value: &[u8]) -> std::io::Result<()> {
    let mut stream = path.as_os_str().to_owned();
    stream.push(format!(":{name}"));
    std::fs::write(stream, value)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn set(_: &Path, _: &str, _: &[u8]) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "extended attributes are unsupported on this platform",
    ))
}

/// `XAttrMetadataPP.run`: returns the warnings for attributes that could not
/// be written; unsupported filesystems are an error.
pub fn write_metadata(path: &Path, info: &VideoInfo) -> Result<Vec<String>> {
    crate::screen::screen("[XAttrMetadata] Writing metadata to file's xattrs");
    let dict = serde_json::to_value(info).unwrap_or_default();
    let mut warnings = Vec::new();
    for (name, field) in MAPPING {
        let Some(value) = dict.get(field).filter(|value| match value {
            serde_json::Value::Null => false,
            serde_json::Value::String(text) => !text.is_empty(),
            _ => true,
        }) else {
            continue;
        };
        let text = match value {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        let text = if field == "upload_date" {
            // `hyphenate_date`.
            match (text.len() == 8 && text.bytes().all(|b| b.is_ascii_digit())).then_some(&text) {
                Some(date) => format!("{}-{}-{}", &date[..4], &date[4..6], &date[6..]),
                None => text,
            }
        } else if name == "com.apple.metadata:kMDItemWhereFroms" {
            // Colons in xattr names fail on NTFS and Linux.
            if !cfg!(target_os = "macos") {
                continue;
            }
            APPLE_PLIST.replace("%s", &text)
        } else {
            text
        };
        if cfg!(windows) && name.contains(':') {
            continue;
        }
        if let Err(error) = set(path, name, text.as_bytes()) {
            match classify(&error) {
                Failure::NoSpace => warnings.push(format!(
                    "There's no disk space left, disk quota exceeded or filesystem xattr limit exceeded. Extended attribute \"{name}\" was not written."
                )),
                Failure::TooLong => warnings.push(format!(
                    "Unable to write extended attribute \"{name}\" due to too long values."
                )),
                Failure::NotSupported => {
                    let tip = if cfg!(windows) {
                        "You need to use NTFS"
                    } else {
                        "You may have to enable them in your \"/etc/fstab\""
                    };
                    return Err(Error::Postprocess(format!(
                        "This filesystem doesn't support extended attributes. {tip}"
                    )));
                }
            }
        }
    }
    Ok(warnings)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn metadata_xattrs_follow_upstream_mapping() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("o.mp4");
        std::fs::write(&path, b"media").unwrap();
        let info: VideoInfo = serde_json::from_value(serde_json::json!({"id": "v", "title": "Title",
            "url": "https://example.test/v.mp4", "webpage_url": "https://example.test/watch",
            "ext": "mp4", "extractor": "generic", "extractor_key": "Generic", "upload_date": "20260102"}))
        .unwrap();
        assert!(write_metadata(&path, &info).unwrap().is_empty());
        let listing = std::process::Command::new("xattr")
            .args(["-p", "user.dublincore.date"])
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&listing.stdout).trim(),
            "2026-01-02"
        );
        let listing = std::process::Command::new("xattr")
            .args(["-p", "user.xdg.referrer.url"])
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&listing.stdout).trim(),
            "https://example.test/watch"
        );
    }
}

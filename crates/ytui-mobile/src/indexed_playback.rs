//! Selected adaptive resources as a bounded in-memory DASH presentation.
//! No network discovery, format changes, or signed URL persistence here.
use serde_json::Value;

fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}
fn range(value: &Value, size: u64) -> Option<(u64, u64)> {
    let (start, end) = (number(&value["start"])?, number(&value["end"])?);
    (start <= end && end < size && end.checked_sub(start)? < 2 * 1024 * 1024)
        .then_some((start, end))
}
fn codec(value: &Value) -> Option<&str> {
    value.as_str().filter(|v| {
        !v.is_empty()
            && *v != "none"
            && v.len() <= 128
            && v.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
    })
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub(crate) fn manifest(tracks: &[&Value], fallback_seconds: Option<f64>) -> Option<String> {
    if tracks.is_empty() || tracks.len() > 2 {
        return None;
    }
    let mut body = String::new();
    let mut duration_ms = 0;
    for (index, track) in tracks.iter().enumerate() {
        if track["has_drm"] == true
            || track["manifest_url"].is_string()
            || track["fragments"].as_array().is_some_and(|f| !f.is_empty())
        {
            return None;
        }
        if track["protocol"]
            .as_str()
            .is_some_and(|p| !matches!(p, "http" | "https"))
        {
            return None;
        }
        let url = track["url"].as_str()?;
        if url.chars().any(char::is_control) {
            return None;
        }
        ytui_core::http_url(url).ok()?;
        let size = number(&track["filesize"])?;
        let init = range(&track["ytui_init_range"], size)?;
        let segments = range(&track["ytui_index_range"], size)?;
        if init.1 >= segments.0 {
            return None;
        }
        let video = codec(&track["vcodec"]);
        let audio = codec(&track["acodec"]);
        // SegmentBase resources are adaptive, one elementary media type each.
        let (kind, codecs) = match (video, audio) {
            (Some(codec), None) if track["acodec"] == "none" => ("video", codec),
            (None, Some(codec)) if track["vcodec"] == "none" => ("audio", codec),
            _ => return None,
        };
        let container = match track["ext"].as_str()? {
            "mp4" | "m4a" => "mp4",
            "webm" => "webm",
            _ => return None,
        };
        let millis = number(&track["ytui_duration_ms"]).or_else(|| {
            let seconds = fallback_seconds?;
            (seconds.is_finite() && seconds > 0.0 && seconds <= 604800.0)
                .then_some((seconds * 1000.0).ceil() as u64)
        })?;
        if millis == 0 || millis > 604800000 {
            return None;
        }
        duration_ms = duration_ms.max(millis);
        let bandwidth = track["tbr"]
            .as_f64()
            .filter(|v| v.is_finite() && *v > 0.0)
            .map(|v| (v * 1000.0).clamp(1.0, 1e9) as u64)
            .unwrap_or(1_000_000);
        let dimensions = if kind == "video" {
            match (number(&track["width"]), number(&track["height"])) {
                (Some(w), Some(h)) if w > 0 && h > 0 && w <= 16384 && h <= 16384 => {
                    format!(" width=\"{w}\" height=\"{h}\"")
                }
                _ => String::new(),
            }
        } else {
            String::new()
        };
        body.push_str(&format!(
            "<AdaptationSet id=\"{index}\" contentType=\"{kind}\" mimeType=\"{kind}/{container}\"><Representation id=\"selected-{index}\" bandwidth=\"{bandwidth}\" codecs=\"{codecs}\"{dimensions}><BaseURL>{}</BaseURL><SegmentBase indexRange=\"{}-{}\"><Initialization range=\"{}-{}\"/></SegmentBase></Representation></AdaptationSet>",
            xml(url), segments.0, segments.1, init.0, init.1));
    }
    let result = format!(
        "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" mediaPresentationDuration=\"PT{}.{:03}S\" minBufferTime=\"PT0.25S\"><Period>{body}</Period></MPD>",
        duration_ms / 1000,
        duration_ms % 1000
    );
    (result.len() <= 65536).then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn video() -> Value {
        json!({"url":"https://media.test/v?one=1&two=2","ext":"mp4","filesize":10000,"vcodec":"avc1.640028","acodec":"none","height":2160,"width":3840,"ytui_init_range":{"start":"0","end":"700"},"ytui_index_range":{"start":"701","end":"999"},"ytui_duration_ms":"12345"})
    }
    #[test]
    fn selected_pair_preserves_urls_quality_and_separate_mp4_webm_ranges() {
        let v = video();
        let mut a = video();
        a["vcodec"] = "none".into();
        a["acodec"] = "opus".into();
        a["ext"] = "webm".into();
        let m = manifest(&[&v, &a], None).unwrap();
        assert!(m.contains("height=\"2160\""));
        assert!(m.contains("video/mp4"));
        assert!(m.contains("audio/webm"));
        assert!(m.contains("one=1&amp;two=2"));
        assert!(m.contains("indexRange=\"701-999\""));
        assert!(m.contains("PT12.345S"));
        assert_eq!(m.matches("<AdaptationSet ").count(), 2);
    }
    #[test]
    fn incomplete_or_unsafe_indexes_leave_the_original_source_available() {
        for (field, value) in [
            ("filesize", json!(100)),
            ("ytui_index_range", json!({"start":0,"end":20})),
            ("ytui_init_range", json!({"start":0,"end":3000000})),
            ("has_drm", json!(true)),
            ("protocol", json!("m3u8_native")),
            ("acodec", json!("mp4a.40.2")),
            ("vcodec", json!("bad\"attribute")),
            ("ytui_duration_ms", json!(0)),
        ] {
            let mut v = video();
            v[field] = value;
            assert!(manifest(&[&v], None).is_none(), "{field}");
        }
        let mut v = video();
        v.as_object_mut().unwrap().remove("ytui_init_range");
        assert!(manifest(&[&v], Some(10.0)).is_none());
        assert!(manifest(&[], Some(10.0)).is_none());
    }
}

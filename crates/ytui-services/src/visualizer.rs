//! Optional transparent mpv level analysis; no capture device or subprocess.
use serde_json::Value;
pub(crate) const FILTER: &str = "@ytui-meter:lavfi=[astats=metadata=1:reset=1:measure_perchannel=RMS_level+Peak_level:measure_overall=RMS_level+Peak_level]";
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AudioLevels {
    pub left: f64,
    pub right: f64,
    pub peak: f64,
}
/// Normalize dBFS to the visible -60..0 dB range. Silence is zero.
pub(crate) fn levels(value: &Value) -> Option<AudioLevels> {
    let read = |key: &str| {
        let v = value.get(key)?;
        let db = v.as_f64().or_else(|| v.as_str()?.parse::<f64>().ok())?;
        if db == f64::NEG_INFINITY {
            Some(0.0)
        } else if db.is_finite() {
            Some(((db + 60.0) / 60.0).clamp(0.0, 1.0))
        } else {
            None
        }
    };
    let left =
        read("lavfi.astats.1.RMS_level").or_else(|| read("lavfi.astats.Overall.RMS_level"))?;
    Some(AudioLevels {
        left,
        right: read("lavfi.astats.2.RMS_level").unwrap_or(left),
        peak: read("lavfi.astats.Overall.Peak_level").unwrap_or(left),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn levels_handle_stereo_mono_silence_missing_and_invalid_metadata() {
        let v=levels(&json!({"lavfi.astats.1.RMS_level":"-30", "lavfi.astats.2.RMS_level":"-12", "lavfi.astats.Overall.Peak_level":"-6"})).unwrap();
        assert_eq!(
            v,
            AudioLevels {
                left: 0.5,
                right: 0.8,
                peak: 0.9
            }
        );
        let silent = levels(&json!({"lavfi.astats.Overall.RMS_level":"-inf"})).unwrap();
        assert_eq!(silent, AudioLevels::default());
        for invalid in [
            json!({}),
            json!({"lavfi.astats.1.RMS_level":"NaN"}),
            json!({"lavfi.astats.1.RMS_level":"inf"}),
        ] {
            assert!(levels(&invalid).is_none());
        }
    }
}

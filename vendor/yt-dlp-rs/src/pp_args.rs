//! Upstream `--postprocessor-args`: extra FFmpeg arguments by postprocessor
//! key, resolved like `PostProcessor._configuration_args`. The CLI installs
//! the parsed arguments once for the process.
use std::{collections::HashMap, ffi::OsString, sync::OnceLock};

static ARGS: OnceLock<HashMap<String, Vec<String>>> = OnceLock::new();

/// Install the `--postprocessor-args` dictionary (lowercase keys such as
/// `merger`, `ffmpeg`, `merger+ffmpeg_i1`, `default`). Later calls are ignored.
pub fn set(args: HashMap<String, Vec<String>>) {
    let _ = ARGS.set(args);
}

fn configuration_args(main_key: &str, exe: &str, suffixes: &[String]) -> Vec<String> {
    ARGS.get()
        .map(|args| resolve(args, main_key, exe, suffixes))
        .unwrap_or_default()
}

/// Upstream `_configuration_args(main_key, argdict, exe, keys)` with
/// `cli_configuration_args`: the first key (or key group) that is given,
/// even with no arguments.
fn resolve(
    args: &HashMap<String, Vec<String>>,
    main_key: &str,
    exe: &str,
    suffixes: &[String],
) -> Vec<String> {
    let pp_key = main_key.to_lowercase();
    let exe = exe.to_lowercase();
    let root = if pp_key == exe {
        exe.clone()
    } else {
        format!("{pp_key}+{exe}")
    };
    let mut groups: Vec<Vec<String>> = suffixes
        .iter()
        .map(|suffix| vec![format!("{root}{suffix}")])
        .collect();
    if suffixes.iter().any(String::is_empty) {
        if pp_key != exe {
            groups.push(vec![pp_key, exe]);
        }
        groups.push(vec!["default".into()]);
    }
    groups
        .iter()
        .find(|group| group.iter().any(|key| args.contains_key(key)))
        .map(|group| {
            group
                .iter()
                .filter_map(|key| args.get(key))
                .flatten()
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Arguments before input `number` (1-based): `_iN`, then `_i`.
pub(crate) fn input(pp: &str, number: usize) -> Vec<OsString> {
    configuration_args(pp, "ffmpeg", &[format!("_i{number}"), "_i".into()])
        .into_iter()
        .map(OsString::from)
        .collect()
}

/// Arguments before the (first) output file: `_o1`, `_o`, then the
/// postprocessor, `ffmpeg` and `default` keys.
pub(crate) fn output(pp: &str) -> Vec<OsString> {
    configuration_args(pp, "ffmpeg", &["_o1".into(), "_o".into(), String::new()])
        .into_iter()
        .map(OsString::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_resolve_like_configuration_args() {
        let owned = |values: &[&str]| values.iter().map(|value| (*value).to_owned()).collect();
        let args = HashMap::from([
            ("merger+ffmpeg_i1".to_owned(), owned(&["-a"])),
            ("merger+ffmpeg_i".to_owned(), owned(&["-b"])),
            ("merger".to_owned(), owned(&["-c"])),
            ("ffmpeg".to_owned(), owned(&["-d"])),
            ("default".to_owned(), owned(&["-e"])),
            ("extractaudio+ffmpeg_o".to_owned(), owned(&["-f"])),
            ("metadata+ffmpeg".to_owned(), Vec::new()),
        ]);
        let input = |pp: &str, number: usize| {
            resolve(&args, pp, "ffmpeg", &[format!("_i{number}"), "_i".into()])
        };
        let output = |pp: &str| {
            resolve(
                &args,
                pp,
                "ffmpeg",
                &["_o1".into(), "_o".into(), String::new()],
            )
        };
        assert_eq!(input("Merger", 1), ["-a"]);
        assert_eq!(input("Merger", 2), ["-b"]);
        // Inputs never fall back to the postprocessor or default keys.
        assert!(input("VideoRemuxer", 1).is_empty());
        // `(main_key, exe)` concatenates both keys before `default`.
        assert_eq!(output("Merger"), ["-c", "-d"]);
        assert_eq!(output("ExtractAudio"), ["-f"]);
        assert_eq!(output("VideoRemuxer"), ["-d"]);
        // A given key without arguments still ends the search.
        assert!(output("Metadata").is_empty());
        assert_eq!(
            resolve(
                &HashMap::from([("default".to_owned(), owned(&["-e"]))]),
                "Merger",
                "ffmpeg",
                &["".into()]
            ),
            ["-e"]
        );
    }
}

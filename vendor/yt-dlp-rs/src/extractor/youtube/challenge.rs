//! Private, version-specific offline EJS boundary for `solve_js_challenges`.
//! Keep challenge data out of executable syntax and validate the entire batch.
mod runtime;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

const MAX_PLAYER: usize = 8 * 1024 * 1024;
const MAX_STDIN: usize = 16 * 1024 * 1024;
const MAX_CHALLENGES: usize = 128;
const MAX_CHALLENGE: usize = 16 * 1024;
const MAX_STDOUT: usize = 1024 * 1024;
const LIB: &str = include_str!("../../../vendor/ejs/yt.solver.lib.js");
const CORE: &str = include_str!("../../../vendor/ejs/yt.solver.core.js");

type Result<T> = std::result::Result<T, String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Kind {
    N,
    Sig,
}

#[derive(Debug, Serialize)]
struct Request {
    #[serde(rename = "type")]
    kind: Kind,
    challenges: Vec<String>,
}

pub(super) struct Batch {
    player: String,
    requests: Vec<Request>,
}

impl Batch {
    /// Deduplicate within each kind, retaining first appearance and kind order.
    pub(super) fn new(player: &str, inputs: &[(Kind, &str)]) -> Result<Self> {
        if player.is_empty() || player.len() > MAX_PLAYER {
            return Err("player source is empty or exceeds 8 MiB".into());
        }
        let mut requests: Vec<Request> = Vec::new();
        for &(kind, challenge) in inputs {
            if challenge.is_empty() || challenge.len() > MAX_CHALLENGE {
                return Err("challenge is empty or exceeds 16 KiB".into());
            }
            let index = if let Some(index) = requests.iter().position(|r| r.kind == kind) {
                index
            } else {
                requests.push(Request {
                    kind,
                    challenges: Vec::new(),
                });
                requests.len() - 1
            };
            if !requests[index].challenges.iter().any(|s| s == challenge) {
                requests[index].challenges.push(challenge.into());
            }
            if requests.iter().map(|r| r.challenges.len()).sum::<usize>() > MAX_CHALLENGES {
                return Err("batch exceeds 128 unique challenges".into());
            }
        }
        if requests.is_empty() {
            return Err("challenge batch is empty".into());
        }
        Ok(Self {
            player: player.into(),
            requests,
        })
    }

    fn script(&self) -> Result<String> {
        #[derive(Serialize)]
        struct Payload<'a> {
            #[serde(rename = "type")]
            kind: &'static str,
            player: &'a str,
            requests: &'a [Request],
        }
        let payload = serde_json::to_string(&Payload {
            kind: "player",
            player: &self.player,
            requests: &self.requests,
        })
        .map_err(|e| e.to_string())?;
        let script = format!(
            "{LIB}\nObject.assign(globalThis, lib);\n{CORE}\nconsole.log(JSON.stringify(jsc({payload})));\n"
        );
        if script.len() > MAX_STDIN {
            return Err("solver stdin exceeds 16 MiB".into());
        }
        Ok(script)
    }

    fn validate(&self, output: &[u8]) -> Result<Solutions> {
        if output.len() > MAX_STDOUT {
            return Err("solver stdout exceeds 1 MiB".into());
        }
        // Deserialize maps through a custom visitor, including escaped key aliases.
        // serde_json::Value would silently accept duplicate keys.
        let envelope: Envelope = serde_json::from_slice(output)
            .map_err(|_| "invalid solver JSON envelope".to_owned())?;
        let responses = match (envelope.kind.as_str(), envelope.responses, envelope.error) {
            ("error", None, Some(error)) => return Err(solver_error(&error)),
            ("result", Some(responses), None) => responses,
            _ => return Err("invalid solver envelope variant".into()),
        };
        if responses.len() != self.requests.len() {
            return Err("solver response count mismatch".into());
        }
        let mut solved = Vec::new();
        for (request, response) in self.requests.iter().zip(responses) {
            let data = match (response.kind.as_str(), response.data, response.error) {
                ("error", None, Some(error)) => return Err(solver_error(&error)),
                ("result", Some(data), None) => data.0,
                _ => return Err("invalid solver response variant".into()),
            };
            if data.len() != request.challenges.len()
                || request.challenges.iter().any(|key| !data.contains_key(key))
            {
                return Err("solver challenge keys mismatch".into());
            }
            for challenge in &request.challenges {
                let solution = &data[challenge];
                if solution.is_empty() || solution.len() > MAX_CHALLENGE {
                    return Err("solver result is empty or exceeds 16 KiB".into());
                }
                solved.push((request.kind, challenge.clone(), solution.clone()));
            }
        }
        Ok(Solutions(solved))
    }
}

// Struct decoding rejects repeated discriminator fields as well as unknown
// fields. Present optional fields must have their concrete type, never null.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, deserialize_with = "present")]
    responses: Option<Vec<Response>>,
    #[serde(default, deserialize_with = "present")]
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, deserialize_with = "present")]
    data: Option<UniqueMap>,
    #[serde(default, deserialize_with = "present")]
    error: Option<String>,
}

fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> std::result::Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

struct UniqueMap(BTreeMap<String, String>);
impl<'de> Deserialize<'de> for UniqueMap {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Visitor;
        impl<'de> serde::de::Visitor<'de> for Visitor {
            type Value = UniqueMap;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("unique string result map")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                let mut values = BTreeMap::new();
                let mut keys = HashSet::new();
                while let Some((key, value)) = map.next_entry::<String, String>()? {
                    if !keys.insert(key.clone()) {
                        return Err(serde::de::Error::custom("duplicate solver result key"));
                    }
                    values.insert(key, value);
                }
                Ok(UniqueMap(values))
            }
        }
        d.deserialize_map(Visitor)
    }
}

fn solver_error(error: &str) -> String {
    // Diagnostics from untrusted source must be bounded and single line.
    format!(
        "solver error: {}",
        error
            .chars()
            .take(512)
            .flat_map(char::escape_default)
            .collect::<String>()
    )
}

pub(super) struct Solutions(Vec<(Kind, String, String)>);
impl Solutions {
    pub(super) fn get(&self, kind: Kind, challenge: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, c, _)| *k == kind && c == challenge)
            .map(|(_, _, result)| result.as_str())
    }
}

/// Configuration is inert until an eligible URL actually needs a solution.
/// Canonical executable resolution and version validation happen once, lazily.
pub(super) struct Runtime {
    path: std::path::PathBuf,
    checked: std::sync::OnceLock<Result<runtime::DenoRuntime>>,
}

impl Runtime {
    pub(super) fn new(path: std::path::PathBuf) -> Self {
        Self {
            path,
            checked: std::sync::OnceLock::new(),
        }
    }
    pub(super) fn checked(&self) -> Result<&runtime::DenoRuntime> {
        self.checked
            .get_or_init(|| runtime::DenoRuntime::new(&self.path))
            .as_ref()
            .map_err(Clone::clone)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn grouping_deduplicates_per_kind_in_first_seen_order() {
        let batch = Batch::new(
            "player",
            &[
                (Kind::Sig, "a"),
                (Kind::N, "a"),
                (Kind::Sig, "a"),
                (Kind::Sig, "b"),
            ],
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&batch.requests).unwrap(),
            json!([
                {"type":"sig","challenges":["a","b"]}, {"type":"n","challenges":["a"]}
            ])
        );
        let output = br#"{"type":"result","responses":[{"type":"result","data":{"a":"x","b":"y"}},{"type":"result","data":{"a":"a"}}]}"#;
        let solved = batch.validate(output).unwrap();
        assert_eq!(solved.get(Kind::Sig, "a"), Some("x"));
        assert_eq!(solved.get(Kind::N, "a"), Some("a"));
        assert_eq!(solved.get(Kind::N, "b"), None);
    }

    #[test]
    fn serialized_data_cannot_escape_the_invocation() {
        let player = "\"}); throw Error('injected'); //\n\0\\\u{2028}é";
        let challenge = "\"\n\0\\\u{2029}é";
        let batch = Batch::new(player, &[(Kind::N, challenge)]).unwrap();
        let script = batch.script().unwrap();
        let prefix = format!(
            "{LIB}\nObject.assign(globalThis, lib);\n{CORE}\nconsole.log(JSON.stringify(jsc("
        );
        let payload = script
            .strip_prefix(&prefix)
            .unwrap()
            .strip_suffix(")));\n")
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(parsed["player"], player);
        assert_eq!(parsed["requests"][0]["challenges"][0], challenge);
        assert!(parsed.get("output_preprocessed").is_none());
    }

    #[test]
    fn validates_complete_envelopes_and_exact_keys() {
        let batch = Batch::new("player", &[(Kind::N, "a")]).unwrap();
        for invalid in [
            "",
            "not json",
            "{}",
            "[]",
            r#"{"type":"result","responses":[]} trailing"#,
            r#"{"type":"result","type":"result","responses":[]}"#,
            r#"{"type":"result","responses":[],"error":null}"#,
            r#"{"type":"result","responses":[],"extra":1}"#,
            r#"{"type":"result","responses":[]}"#,
            r#"{"type":"error","error":"bad"}"#,
            r#"{"type":"result","responses":[{"type":"error","error":"bad"}]}"#,
            r#"{"type":"result","responses":[{"type":"result","data":{"b":"x"}}]}"#,
            r#"{"type":"result","responses":[{"type":"result","data":{"a":"x","b":"y"}}]}"#,
            r#"{"type":"result","responses":[{"type":"result","data":{"a":""}}]}"#,
            r#"{"type":"result","responses":[{"type":"result","data":{"a":null}}]}"#,
            r#"{"type":"result","responses":[{"type":"result","data":{"a":7}}]}"#,
            r#"{"type":"result","responses":[{"type":"result","data":{"a":"x","a":"y"}}]}"#,
            r#"{"type":"result","responses":[{"type":"result","data":{"a":"x","\u0061":"y"}}]}"#,
            r#"{"type":"result","responses":[{"type":"result","type":"error","data":{"a":"x"}}]}"#,
            r#"{"type":"result","responses":[{"type":"unknown","data":{"a":"x"}}]}"#,
        ] {
            assert!(batch.validate(invalid.as_bytes()).is_err(), "{invalid}");
        }
        assert!(batch.validate(&[0xff]).is_err());
        let output = br#"{"type":"result","responses":[{"type":"result","data":{"a":"x"}}]}"#;
        assert_eq!(batch.validate(output).unwrap().get(Kind::N, "a"), Some("x"));
        let repeated = format!(
            "{} {}",
            String::from_utf8_lossy(output),
            String::from_utf8_lossy(output)
        );
        assert!(batch.validate(repeated.as_bytes()).is_err());
    }

    #[test]
    fn input_and_output_budgets_are_errors() {
        assert!(Batch::new("", &[(Kind::N, "a")]).is_err());
        assert!(Batch::new("player", &[]).is_err());
        assert!(Batch::new("player", &[(Kind::N, "")]).is_err());
        assert!(Batch::new(&"x".repeat(MAX_PLAYER + 1), &[(Kind::N, "a")]).is_err());
        assert!(Batch::new("player", &[(Kind::N, &"x".repeat(MAX_CHALLENGE + 1))]).is_err());
        let strings: Vec<_> = (0..129).map(|n| n.to_string()).collect();
        let inputs: Vec<_> = strings.iter().map(|s| (Kind::N, s.as_str())).collect();
        assert!(Batch::new("player", &inputs).is_err());
        let batch = Batch::new("player", &[(Kind::N, "a")]).unwrap();
        assert!(batch.validate(&vec![b' '; MAX_STDOUT + 1]).is_err());
        let output = json!({"type":"result","responses":[{"type":"result","data":{"a":"x".repeat(MAX_CHALLENGE+1)}}]}).to_string();
        assert!(batch.validate(output.as_bytes()).is_err());
        // JSON control-character escaping can exceed stdin even within player cap.
        let batch = Batch::new(&"\0".repeat(MAX_PLAYER), &[(Kind::N, "a")]).unwrap();
        assert!(batch.script().is_err());
    }
}

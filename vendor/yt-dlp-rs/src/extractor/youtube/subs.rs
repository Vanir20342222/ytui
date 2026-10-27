//! Upstream `YoutubeIE._real_extract`'s captions: every client response's
//! caption tracks (`process_language` for each subtitle format), their
//! translations and the original audio language they reveal.
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

/// `_SUBTITLE_FORMATS`.
const SUBTITLE_FORMATS: [&str; 7] = ["json3", "srv1", "srv2", "srv3", "ttml", "srt", "vtt"];

/// `_get_text(data, max_runs=1)`.
fn first_run_text(value: Option<&Value>) -> Option<String> {
    let value = value?;
    if let Some(text) = value
        .get("simpleText")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_owned());
    }
    let runs = value
        .get("runs")
        .and_then(Value::as_array)
        .filter(|runs| !runs.is_empty())
        .or_else(|| value.as_array())?;
    runs.first()
        .and_then(|run| run.get("text"))
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// Python truthiness of a JSON value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(object)) => !object.is_empty(),
    }
}

/// `parse_qs`: blank values dropped, repeated keys kept in order.
fn parse_qs(query: &str) -> Vec<(String, Vec<String>)> {
    let mut found: Vec<(String, Vec<String>)> = Vec::new();
    for (name, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if value.is_empty() {
            continue;
        }
        match found.iter_mut().find(|(known, _)| *known == name) {
            Some((_, values)) => values.push(value.into_owned()),
            None => found.push((name.into_owned(), vec![value.into_owned()])),
        }
    }
    found
}

/// `update_url_query(url, query)`: `parse_qs`, `dict.update` (updated keys
/// keep their place, an empty list removes) and `urlencode(doseq=True)`.
fn update_url_query(url: &str, updates: &[(&str, Option<&str>)]) -> String {
    let (base, rest) = url.split_once('?').unwrap_or((url, ""));
    let (query, fragment) = rest
        .split_once('#')
        .map_or((rest, None), |(q, f)| (q, Some(f)));
    let mut pairs = parse_qs(query);
    for (key, value) in updates {
        let values = value
            .map(|value| vec![value.to_owned()])
            .unwrap_or_default();
        match pairs.iter_mut().find(|(known, _)| known == key) {
            Some((_, existing)) => *existing = values,
            None => pairs.push(((*key).to_owned(), values)),
        }
    }
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, values) in &pairs {
        for value in values {
            serializer.append_pair(key, value);
        }
    }
    let query = serializer.finish();
    let mut out = base.to_owned();
    if !query.is_empty() {
        out.push('?');
        out.push_str(&query);
    }
    if let Some(fragment) = fragment {
        out.push('#');
        out.push_str(fragment);
    }
    out
}

/// `get_lang_code`: the `vssId` without its leading dot, dots as dashes,
/// else `languageCode`.
fn lang_code(track: &Value) -> Option<String> {
    let vss = track
        .get("vssId")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let vss = vss.strip_prefix('.').unwrap_or(vss).replace('.', "-");
    if !vss.is_empty() {
        return Some(vss);
    }
    track
        .get("languageCode")
        .and_then(Value::as_str)
        .filter(|code| !code.is_empty())
        .map(str::to_owned)
}

/// `process_language`.
fn process_language(
    container: &mut Map<String, Value>,
    base_url: &str,
    lang_code: &str,
    sub_name: Option<&str>,
    client: &str,
    query: &[(&str, &str)],
) {
    let entry = container
        .entry(lang_code.to_owned())
        .or_insert_with(|| Value::Array(Vec::new()));
    let Value::Array(tracks) = entry else {
        return;
    };
    for format in SUBTITLE_FORMATS {
        let mut updates: Vec<(&str, Option<&str>)> = query
            .iter()
            .map(|(key, value)| (*key, Some(*value)))
            .collect();
        updates.push(("fmt", Some(format)));
        // `xosf=1` gives unwanted positioning data.
        updates.push(("xosf", None));
        let updated = update_url_query(base_url, &updates);
        let url = url::Url::parse("https://www.youtube.com")
            .and_then(|base| base.join(&updated))
            .map_or(updated, String::from);
        tracks.push(json!({
            "ext": format,
            "url": url,
            "name": sub_name,
            "impersonate": true,
            "__yt_dlp_client": client,
        }));
    }
}

/// The captions' results.
pub(super) struct Captions {
    pub subtitles: Map<String, Value>,
    pub automatic: Map<String, Value>,
    /// Languages `set_audio_lang_from_orig_subs_lang` assigned, in order.
    pub audio_languages: Vec<String>,
}

/// Upstream's caption loop over the client responses (`player_responses`
/// with streaming data and a track list renderer).
pub(super) fn extract(
    player_responses: &[Value],
    clients: &[(&str, &Value)],
    automatic: Map<String, Value>,
    translated: bool,
) -> Captions {
    let mut result = Captions {
        subtitles: Map::new(),
        automatic,
        audio_languages: Vec::new(),
    };
    // `translationLanguages` from every response (the initial one too).
    let mut translations: Vec<(String, Option<String>)> = Vec::new();
    for response in player_responses {
        for language in response
            .pointer("/captions/playerCaptionsTracklistRenderer/translationLanguages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if !truthy(language.get("languageCode")) || !truthy(language.get("languageName")) {
                continue;
            }
            let Some(code) = language.get("languageCode").and_then(Value::as_str) else {
                continue;
            };
            let name = first_run_text(language.get("languageName"));
            match translations.iter_mut().find(|(known, _)| known == code) {
                Some((_, existing)) => *existing = name,
                None => translations.push((code.to_owned(), name)),
            }
        }
    }
    let responses: Vec<(&str, &Value)> = clients
        .iter()
        .filter(|(_, response)| {
            truthy(response.get("streamingData"))
                && truthy(response.pointer("/captions/playerCaptionsTracklistRenderer"))
        })
        .copied()
        .collect();
    let tracks_of = |response: &Value| -> Vec<Value> {
        response
            .pointer("/captions/playerCaptionsTracklistRenderer/captionTracks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let all: Vec<Value> = responses
        .iter()
        .flat_map(|(_, response)| tracks_of(response))
        .filter(Value::is_object)
        .collect();
    let is_asr = |track: &Value| track.get("kind").and_then(Value::as_str) == Some("asr");
    let mut need_subs: BTreeSet<String> = all
        .iter()
        .filter(|track| !is_asr(track))
        .filter_map(lang_code)
        .collect();
    let mut need_caps: BTreeSet<String> = all
        .iter()
        .filter(|track| is_asr(track))
        .filter_map(lang_code)
        .map(|code| code.strip_prefix("a-").map_or(code.clone(), str::to_owned))
        .collect();
    let set_audio = |languages: &mut Vec<String>, code: &str| {
        if !languages.iter().any(|known| known == code) {
            languages.push(code.to_owned());
        }
    };
    for (client, response) in responses {
        for track in tracks_of(response) {
            let Some(base_url) = track
                .get("baseUrl")
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
            else {
                continue;
            };
            let query = base_url.split_once('?').map_or("", |(_, query)| query);
            let qs = parse_qs(query.split('#').next().unwrap_or_default());
            let code = lang_code(&track);
            // A PO token is required by the `xpe`/`xpv` experiments; none
            // can be provided.
            let requires_pot =
                qs.iter()
                    .find(|(key, _)| key == "exp")
                    .is_some_and(|(_, values)| {
                        values.iter().any(|value| value == "xpe" || value == "xpv")
                    });
            if requires_pot {
                crate::screen::debug(&format!(
                    "Some {client} client subtitles require a PO Token which was not provided. They will be discarded since they are not downloadable as-is. You can manually pass a Subtitles PO Token for this client with --extractor-args \"youtube:po_token={client}.subs+XXX\" . For more information, refer to  https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide"
                ));
                break;
            }
            let orig_lang = qs
                .iter()
                .find(|(key, _)| key == "lang")
                .and_then(|(_, values)| values.last().cloned());
            let lang_name = first_run_text(track.get("name"));
            let manual = !is_asr(&track);
            let translatable = truthy(track.get("isTranslatable"));
            if manual {
                let Some(code) = code.as_deref() else {
                    continue;
                };
                process_language(
                    &mut result.subtitles,
                    base_url,
                    code,
                    lang_name.as_deref(),
                    client,
                    &[],
                );
                if !translatable {
                    continue;
                }
            }
            for (trans_code, trans_name) in &translations {
                if trans_code.is_empty() {
                    continue;
                }
                let original = trans_code.clone();
                let mut trans_code = trans_code.clone();
                let mut trans_name = trans_name.clone().unwrap_or_default();
                if manual && trans_code != "und" {
                    if !translated {
                        continue;
                    }
                    trans_code.push('-');
                    trans_code.push_str(code.as_deref().unwrap_or_default());
                    if let Some(lang_name) = &lang_name {
                        trans_name.push_str(&format!(" from {lang_name}"));
                    }
                }
                if code.as_deref() == Some(format!("a-{original}").as_str()) {
                    set_audio(&mut result.audio_languages, &original);
                    process_language(
                        &mut result.automatic,
                        base_url,
                        &format!("{trans_code}-orig"),
                        Some(&format!("{trans_name} (Original)")),
                        client,
                        &[],
                    );
                }
                // Setting `tlang=lang` returns damaged subtitles.
                let tlang = [("tlang", trans_code.as_str())];
                process_language(
                    &mut result.automatic,
                    base_url,
                    &trans_code,
                    Some(&trans_name),
                    client,
                    if orig_lang.as_deref() == Some(original.as_str()) {
                        &[]
                    } else {
                        &tlang
                    },
                );
            }
            // Automatic captions for languages missing from the
            // translations (such as Cantonese).
            let code = code.map(|code| code.strip_prefix("a-").map_or(code.clone(), str::to_owned));
            let Some(code) = code.filter(|code| !code.is_empty()) else {
                continue;
            };
            if manual || result.automatic.contains_key(&code) {
                continue;
            }
            let name = lang_name.map(|name| {
                name.strip_suffix(" (auto-generated)")
                    .map_or(name.clone(), str::to_owned)
            });
            if translatable {
                set_audio(&mut result.audio_languages, &code);
                process_language(
                    &mut result.automatic,
                    base_url,
                    &format!("{code}-orig"),
                    Some(&format!("{} (Original)", name.as_deref().unwrap_or("None"))),
                    client,
                    &[],
                );
            }
            process_language(
                &mut result.automatic,
                base_url,
                &code,
                name.as_deref(),
                client,
                &[],
            );
        }
        need_subs.retain(|code| !result.subtitles.contains_key(code));
        need_caps.retain(|code| !result.automatic.contains_key(code));
        if need_subs.is_empty() && need_caps.is_empty() {
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_updates_keep_positions_and_drop_empty_lists() {
        assert_eq!(
            update_url_query(
                "https://e.test/t?v=x&fmt=a&xosf=1&lang=en",
                &[("tlang", Some("de")), ("fmt", Some("vtt")), ("xosf", None)]
            ),
            "https://e.test/t?v=x&fmt=vtt&lang=en&tlang=de"
        );
    }
}

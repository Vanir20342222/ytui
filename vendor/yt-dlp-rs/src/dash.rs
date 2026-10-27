//! Pure, bounded static single-period DASH plans. No network or SIDX discovery.
use crate::{ByteRange, Error, Fragment, Result, StreamKind, VideoFormat};
use roxmltree::{Document, Node, ParsingOptions};
use std::collections::BTreeMap;
use url::Url;

const LIMIT: usize = 10_000;
const NS: u128 = 1_000_000_000;
fn bad(message: impl Into<String>) -> Error {
    Error::UnsupportedManifest(message.into())
}
fn number(text: &str) -> Result<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(bad("invalid DASH unsigned integer"));
    }
    text.parse().map_err(|_| bad("DASH integer overflow"))
}
fn positive(text: &str) -> Result<u64> {
    let value = number(text)?;
    if value == 0 {
        return Err(bad("DASH duration/timescale must be positive"));
    }
    Ok(value)
}
fn add(a: u128, b: u128) -> Result<u128> {
    a.checked_add(b).ok_or_else(|| bad("DASH timing overflow"))
}
fn mul(a: u128, b: u128) -> Result<u128> {
    a.checked_mul(b).ok_or_else(|| bad("DASH timing overflow"))
}
fn ceil(a: u128, b: u128) -> Result<u128> {
    Ok(a / b + u128::from(!a.is_multiple_of(b)))
}

/// Exact restricted ISO 8601 day/hour/minute/second duration, in nanoseconds.
fn duration(text: &str) -> Result<u128> {
    let mut rest = text
        .strip_prefix('P')
        .ok_or_else(|| bad("invalid DASH ISO duration"))?;
    let mut time = false;
    let mut last = 0;
    let mut seen = false;
    let mut total = 0;
    while !rest.is_empty() {
        if let Some(tail) = rest.strip_prefix('T') {
            if time {
                return Err(bad("invalid DASH duration time marker"));
            }
            time = true;
            rest = tail;
            if rest.is_empty() {
                return Err(bad("empty DASH duration time"));
            }
            continue;
        }
        let end = rest
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .ok_or_else(|| bad("missing DASH duration unit"))?;
        let amount = &rest[..end];
        let unit = rest.as_bytes()[end];
        let (rank, scale) = match (time, unit) {
            (false, b'D') => (1, 86_400),
            (true, b'H') => (2, 3600),
            (true, b'M') => (3, 60),
            (true, b'S') => (4, 1),
            _ => return Err(bad("unsupported DASH duration unit")),
        };
        if rank <= last {
            return Err(bad("duplicate or unordered DASH duration unit"));
        }
        last = rank;
        let nanos = if let Some((whole, fraction)) = amount.split_once('.') {
            if unit != b'S' || fraction.is_empty() || fraction.len() > 9 {
                return Err(bad("unsupported DASH fractional duration"));
            }
            let fraction = number(fraction)?;
            add(
                mul(u128::from(number(whole)?), NS)?,
                u128::from(fraction)
                    * 10u128.pow(9 - amount.split_once('.').unwrap().1.len() as u32),
            )?
        } else {
            mul(u128::from(number(amount)?), NS)?
        };
        total = add(total, mul(nanos, scale)?)?;
        seen = true;
        rest = &rest[end + 1..];
    }
    if !seen {
        return Err(bad("empty DASH duration"));
    }
    Ok(total)
}

fn children<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Vec<Node<'a, 'input>> {
    node.children()
        .filter(|n| n.is_element() && n.tag_name().name() == name)
        .collect()
}
fn one<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Result<Option<Node<'a, 'input>>> {
    let nodes = children(node, name);
    if nodes.len() > 1 {
        return Err(bad(format!(
            "multiple DASH {name} elements are unsupported"
        )));
    }
    Ok(nodes.into_iter().next())
}
fn uri(base: &Url, value: &str) -> Result<Url> {
    if value.is_empty() || value.contains(['\r', '\n']) || value.len() > 16_384 {
        return Err(bad("empty or oversized DASH URI"));
    }
    let url = base.join(value)?;
    if url.as_str().len() > 16_384 {
        return Err(bad("oversized resolved DASH URI"));
    }
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.fragment().is_some()
    {
        return Err(bad("DASH resources require HTTP(S) URLs without fragments"));
    }
    Ok(url)
}
fn base_url(node: Node<'_, '_>, inherited: &Url, explicit: bool) -> Result<(Url, bool)> {
    match one(node, "BaseURL")? {
        Some(base) => Ok((uri(inherited, base.text().unwrap_or("").trim())?, true)),
        None => Ok((inherited.clone(), explicit)),
    }
}
fn range(text: &str) -> Result<ByteRange> {
    let (start, end) = text
        .split_once('-')
        .ok_or_else(|| bad("invalid DASH byte range"))?;
    let offset = number(start)?;
    let end = number(end)?;
    let length = end
        .checked_sub(offset)
        .and_then(|n| n.checked_add(1))
        .filter(|n| *n > 0)
        .ok_or_else(|| bad("invalid DASH byte range endpoints"))?;
    offset
        .checked_add(length)
        .ok_or_else(|| bad("DASH byte range overflow"))?;
    Ok(ByteRange { offset, length })
}
fn fragment(
    base: &Url,
    source: Option<&str>,
    bytes: Option<&str>,
    duration: Option<f64>,
) -> Result<Fragment> {
    let url = match source {
        Some(source) => uri(base, source)?,
        None if bytes.is_some() && !base.path().ends_with('/') => base.clone(),
        _ => return Err(bad("DASH fragment has no resource URI")),
    };
    // Upstream `location_key`: `url` for absolute HTTP(S) locations.
    let absolute = |source: &str| {
        let lower = source.get(..8).unwrap_or(source).to_ascii_lowercase();
        lower.starts_with("http://") || lower.starts_with("https://")
    };
    Ok(Fragment {
        url,
        byte_range: bytes.map(range).transpose()?,
        encryption: None,
        path: source.filter(|source| !absolute(source)).map(str::to_owned),
        duration,
    })
}
/// Upstream's text representation keys: the MPD as `manifest_url` and, when
/// segmented, the base of relative fragment paths.
fn text_extra(manifest: &Url, base: Option<Url>) -> serde_json::Map<String, serde_json::Value> {
    let mut extra = serde_json::Map::new();
    extra.insert(crate::info_order::FORMAT_SHAPE.into(), "dash-text".into());
    extra.insert("manifest_url".into(), manifest.as_str().into());
    if let Some(base) = base {
        extra.insert("fragment_base_url".into(), base.as_str().into());
    }
    extra
}

#[derive(Clone)]
struct Entry {
    t: Option<u64>,
    d: u64,
    r: i64,
}
#[derive(Clone, Default)]
struct Address {
    kind: Option<String>,
    attrs: BTreeMap<String, String>,
    timeline: Option<Vec<Entry>>,
    initialization: Option<(Option<String>, Option<String>)>,
    segments: Option<Vec<(Option<String>, Option<String>)>>,
}
impl Address {
    fn inherit(&self, node: Node<'_, '_>) -> Result<Self> {
        let template = one(node, "SegmentTemplate")?;
        let list = one(node, "SegmentList")?;
        if template.is_some() && list.is_some() {
            return Err(bad("ambiguous DASH addressing"));
        }
        let Some(source) = template.or(list) else {
            return Ok(self.clone());
        };
        let kind = source.tag_name().name();
        if self.kind.as_deref().is_some_and(|parent| parent != kind) {
            return Err(bad("mixed inherited DASH addressing schemes"));
        }
        let mut inherited = self.clone();
        inherited.kind = Some(kind.into());
        for attr in source.attributes() {
            if attr.namespace().is_some() {
                return Err(bad("namespaced DASH addressing attributes are unsupported"));
            }
            if !matches!(
                attr.name(),
                "timescale"
                    | "duration"
                    | "startNumber"
                    | "presentationTimeOffset"
                    | "media"
                    | "initialization"
            ) {
                return Err(bad(format!(
                    "unsupported DASH addressing attribute {}",
                    attr.name()
                )));
            }
            inherited
                .attrs
                .insert(attr.name().into(), attr.value().into());
        }
        for child in source.children().filter(Node::is_element) {
            if !matches!(
                child.tag_name().name(),
                "SegmentTimeline" | "Initialization" | "SegmentURL"
            ) {
                return Err(bad("unsupported DASH addressing element"));
            }
        }
        if let Some(timeline) = one(source, "SegmentTimeline")? {
            let mut entries = Vec::new();
            for entry in timeline.children().filter(Node::is_element) {
                if entry.tag_name().name() != "S"
                    || entry
                        .attributes()
                        .any(|a| !matches!(a.name(), "t" | "d" | "r"))
                {
                    return Err(bad("unsupported DASH timeline element"));
                }
                let r = entry
                    .attribute("r")
                    .unwrap_or("0")
                    .parse::<i64>()
                    .map_err(|_| bad("invalid DASH repeat"))?;
                if r < -1 {
                    return Err(bad("invalid DASH negative repeat"));
                }
                entries.push(Entry {
                    t: entry.attribute("t").map(number).transpose()?,
                    d: positive(
                        entry
                            .attribute("d")
                            .ok_or_else(|| bad("DASH timeline duration is required"))?,
                    )?,
                    r,
                });
            }
            if entries.is_empty() {
                return Err(bad("empty DASH timeline"));
            }
            inherited.timeline = Some(entries);
        }
        if let Some(init) = one(source, "Initialization")? {
            if init
                .attributes()
                .any(|a| !matches!(a.name(), "sourceURL" | "range"))
            {
                return Err(bad("unsupported DASH initialization attribute"));
            }
            inherited.initialization = Some((
                init.attribute("sourceURL").map(str::to_owned),
                init.attribute("range").map(str::to_owned),
            ));
        }
        let urls = children(source, "SegmentURL");
        if kind == "SegmentTemplate" && !urls.is_empty() {
            return Err(bad("SegmentURL in DASH template"));
        }
        if kind == "SegmentList"
            && (inherited.attrs.contains_key("media")
                || inherited.attrs.contains_key("initialization"))
        {
            return Err(bad("template attributes in DASH SegmentList"));
        }
        if !urls.is_empty() {
            let mut segments = Vec::new();
            for url in urls {
                if url
                    .attributes()
                    .any(|a| !matches!(a.name(), "media" | "mediaRange"))
                {
                    return Err(bad("DASH index/SIDX attributes are unsupported"));
                }
                segments.push((
                    url.attribute("media").map(str::to_owned),
                    url.attribute("mediaRange").map(str::to_owned),
                ));
            }
            inherited.segments = Some(segments);
        }
        Ok(inherited)
    }
    fn integer(&self, name: &str, default: u64, nonzero: bool) -> Result<u64> {
        self.attrs
            .get(name)
            .map(|v| if nonzero { positive(v) } else { number(v) })
            .unwrap_or(Ok(default))
    }
}

fn expand(
    template: &str,
    id: Option<&str>,
    bandwidth: Option<u64>,
    number_value: u64,
    time: u64,
) -> Result<String> {
    expand_template(template, id, bandwidth, number_value, time, false)
}

fn expand_template(
    template: &str,
    id: Option<&str>,
    bandwidth: Option<u64>,
    number_value: u64,
    time: u64,
    initialization: bool,
) -> Result<String> {
    let mut rest = template;
    let mut output = String::new();
    let mut uses_number = false;
    let mut uses_time = false;
    while let Some(start) = rest.find('$') {
        output.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        if let Some(tail) = rest.strip_prefix('$') {
            output.push('$');
            rest = tail;
            continue;
        }
        let end = rest
            .find('$')
            .ok_or_else(|| bad("unterminated DASH template token"))?;
        let token = &rest[..end];
        let (name, padding) = token
            .split_once('%')
            .map_or((token, None), |(name, padding)| (name, Some(padding)));
        uses_number |= name == "Number";
        uses_time |= name == "Time";
        if initialization && (uses_number || uses_time) {
            return Err(bad(
                "DASH initialization cannot contain Number or Time tokens",
            ));
        }
        if uses_number && uses_time {
            return Err(bad(
                "DASH media templates cannot combine Number and Time tokens",
            ));
        }
        let value = match name {
            "RepresentationID" if padding.is_none() => id
                .ok_or_else(|| bad("DASH RepresentationID token requires an id"))?
                .to_owned(),
            "Number" | "Time" | "Bandwidth" => {
                let value = match name {
                    "Number" => number_value,
                    "Time" => time,
                    _ => bandwidth.ok_or_else(|| bad("DASH Bandwidth token requires bandwidth"))?,
                };
                let width = match padding {
                    None => 0,
                    Some(padding) => {
                        let width = padding
                            .strip_prefix('0')
                            .and_then(|s| s.strip_suffix('d'))
                            .ok_or_else(|| bad("unsupported DASH numeric formatting"))?;
                        let width = number(width)?;
                        if width == 0 || width > 20 {
                            return Err(bad("DASH numeric format width must be 1..20"));
                        }
                        width as usize
                    }
                };
                format!("{value:0width$}")
            }
            _ => return Err(bad("unsupported DASH template token")),
        };
        output.push_str(&value);
        if output.len() > 16_384 {
            return Err(bad("oversized DASH template URL"));
        }
        rest = &rest[end + 1..];
    }
    output.push_str(rest);
    if output.len() > 16_384 {
        return Err(bad("oversized DASH template URL"));
    }
    Ok(output)
}

/// Segment numbers, start times and durations (timescale units).
fn times(address: &Address, duration_ns: Option<u128>) -> Result<Vec<(u64, u64, u64)>> {
    let scale = address.integer("timescale", 1, true)?;
    let pto = address.integer("presentationTimeOffset", 0, false)?;
    let first_number = address.integer("startNumber", 1, false)?;
    let period_end = duration_ns
        .map(|duration| add(mul(u128::from(pto), NS)?, mul(duration, u128::from(scale))?))
        .transpose()?;
    let mut times = Vec::new();
    let mut current = 0u64;
    if let Some(entries) = &address.timeline {
        for (index, entry) in entries.iter().enumerate() {
            let start = entry.t.unwrap_or(current);
            if start < pto {
                return Err(bad(
                    "DASH timeline requires unsupported pre-period trimming",
                ));
            }
            if start < current {
                return Err(bad("overlapping or backwards DASH timeline"));
            }
            let count = if entry.r >= 0 {
                u128::from(entry.r as u64) + 1
            } else {
                let end = if let Some(next) = entries.get(index + 1) {
                    let next_t = next
                        .t
                        .ok_or_else(|| bad("negative DASH repeat requires explicit next start"))?;
                    let difference = next_t
                        .checked_sub(start)
                        .ok_or_else(|| bad("backwards DASH repeat boundary"))?;
                    if !difference.is_multiple_of(entry.d) {
                        return Err(bad("DASH repeat overlaps next timeline entry"));
                    }
                    mul(u128::from(next_t), NS)?
                } else {
                    period_end.ok_or_else(|| bad("unbounded DASH negative repeat"))?
                };
                let begin = mul(u128::from(start), NS)?;
                if end <= begin {
                    return Err(bad("empty DASH negative repeat"));
                }
                ceil(end - begin, mul(u128::from(entry.d), NS)?)?
            };
            if count > LIMIT as u128 || times.len() + count as usize > LIMIT {
                return Err(bad("DASH fragment limit exceeded"));
            }
            current = start;
            // Upstream lists every S entry, even past a finite Period end.
            for _ in 0..count {
                let number = first_number
                    .checked_add(times.len() as u64)
                    .ok_or_else(|| bad("DASH segment number overflow"))?;
                times.push((number, current, entry.d));
                current = current
                    .checked_add(entry.d)
                    .ok_or_else(|| bad("DASH timeline overflow"))?;
            }
        }
    } else {
        let segment_duration = address.integer("duration", 0, true)?;
        if segment_duration == 0 {
            return Err(bad("DASH template requires duration or timeline"));
        }
        let duration = duration_ns
            .filter(|value| *value > 0)
            .ok_or_else(|| bad("DASH duration template requires a finite period duration"))?;
        let count = ceil(
            mul(duration, u128::from(scale))?,
            mul(NS, u128::from(segment_duration))?,
        )?;
        if count > LIMIT as u128 {
            return Err(bad("DASH fragment limit exceeded"));
        }
        for index in 0..count as u64 {
            let number = first_number
                .checked_add(index)
                .ok_or_else(|| bad("DASH segment number overflow"))?;
            let time = pto
                .checked_add(
                    index
                        .checked_mul(segment_duration)
                        .ok_or_else(|| bad("DASH timeline overflow"))?,
                )
                .ok_or_else(|| bad("DASH timeline overflow"))?;
            times.push((number, time, segment_duration));
        }
    }
    Ok(times)
}

fn plans(
    address: &Address,
    base: &Url,
    explicit_base: bool,
    duration_ns: Option<u128>,
    id: Option<&str>,
    bandwidth: Option<u64>,
) -> Result<Vec<Fragment>> {
    if !explicit_base
        && (address
            .initialization
            .as_ref()
            .is_some_and(|(source, _)| source.is_none())
            || address
                .segments
                .as_ref()
                .is_some_and(|segments| segments.iter().any(|(source, _)| source.is_none())))
    {
        return Err(bad(
            "DASH omitted resource URI requires an explicit resource BaseURL",
        ));
    }
    if address.kind.is_none() {
        if !explicit_base || base.path().ends_with('/') {
            return Err(bad(
                "DASH representation has no explicit direct resource or addressing",
            ));
        }
        return Ok(vec![Fragment {
            url: base.clone(),
            byte_range: None,
            encryption: None,
            path: None,
            duration: None,
        }]);
    }
    let mut fragments = Vec::new();
    if address.kind.as_deref() == Some("SegmentTemplate") {
        let schedule = times(address, duration_ns)?;
        let media = address
            .attrs
            .get("media")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| bad("DASH template has no media URL"))?;
        if let Some(initialization) = address.attrs.get("initialization") {
            if address.initialization.is_some() {
                return Err(bad("ambiguous DASH initialization"));
            }
            let init = expand_template(initialization, id, bandwidth, 0, 0, true)?;
            fragments.push(fragment(base, Some(&init), None, None)?);
        } else if let Some((source, range)) = &address.initialization {
            fragments.push(fragment(base, source.as_deref(), range.as_deref(), None)?);
        }
        // Like upstream, initialization is optional; each fragment's duration
        // is `d / timescale` (the constant `duration` without a timeline).
        let scale = address.integer("timescale", 1, true)? as f64;
        for (number, time, length) in schedule {
            let media = expand(media, id, bandwidth, number, time)?;
            fragments.push(fragment(
                base,
                Some(&media),
                None,
                Some(length as f64 / scale),
            )?);
        }
    } else {
        address.integer("timescale", 1, true)?;
        address.integer("startNumber", 1, false)?;
        address.integer("presentationTimeOffset", 0, false)?;
        if let Some(value) = address.attrs.get("duration") {
            positive(value)?;
        }
        // Like upstream, initialization is optional.
        if let Some((source, bytes)) = &address.initialization {
            fragments.push(fragment(base, source.as_deref(), bytes.as_deref(), None)?);
        }
        let segments = address
            .segments
            .as_ref()
            .filter(|segments| !segments.is_empty())
            .ok_or_else(|| bad("empty DASH SegmentList"))?;
        let timeline = match address.timeline {
            Some(_) => Some(times(address, duration_ns)?),
            None => None,
        };
        if timeline
            .as_ref()
            .is_some_and(|timeline| timeline.len() != segments.len())
        {
            return Err(bad("DASH timeline and SegmentList counts disagree"));
        }
        // Upstream includes every SegmentURL regardless of the Period end.
        if let Some(duration) = address
            .attrs
            .get("duration")
            .filter(|_| address.timeline.is_none())
        {
            positive(duration)?;
        }
        // Upstream: timeline durations, else the constant `duration`.
        let scale = address.integer("timescale", 1, true)? as f64;
        let constant = address
            .attrs
            .get("duration")
            .and_then(|value| positive(value).ok())
            .map(|value| value as f64 / scale);
        for (index, (source, bytes)) in segments.iter().enumerate() {
            let duration = match &timeline {
                Some(timeline) => Some(timeline[index].2 as f64 / scale),
                None => constant,
            };
            fragments.push(fragment(
                base,
                source.as_deref(),
                bytes.as_deref(),
                duration,
            )?);
        }
    }
    if fragments.len() > LIMIT {
        return Err(bad("DASH fragment limit exceeded"));
    }
    Ok(fragments)
}

#[cfg(test)]
pub(crate) fn parse(text: &str, manifest: &Url) -> Result<Vec<VideoFormat>> {
    Ok(parse_with_subtitles(text, manifest)?.0)
}

/// Subtitles by language, in manifest order.
pub(crate) type ManifestSubtitles = Vec<(String, crate::SubtitleFormat)>;

/// Upstream `_parse_mpd_formats_and_subtitles`: the formats and, from text
/// representations with a direct resource, the subtitles by language
/// (`und` without one). Segmented text tracks are not represented.
pub(crate) fn parse_with_subtitles(
    text: &str,
    manifest: &Url,
) -> Result<(Vec<VideoFormat>, ManifestSubtitles)> {
    if !matches!(manifest.scheme(), "http" | "https")
        || manifest.host_str().is_none()
        || manifest.fragment().is_some()
    {
        return Err(bad(
            "DASH manifest requires an HTTP(S) URL without a fragment",
        ));
    }
    if text.contains("<!DOCTYPE") {
        return Err(bad("DASH DTDs are unsupported"));
    }
    let document = Document::parse_with_options(
        text,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: 100_000,
            ..ParsingOptions::default()
        },
    )
    .map_err(|error| bad(format!("invalid DASH XML: {error}")))?;
    let root = document.root_element();
    if root.tag_name().name() != "MPD"
        || !matches!(
            root.tag_name().namespace(),
            None | Some("urn:mpeg:dash:schema:mpd:2011")
        )
    {
        return Err(bad("expected a DASH MPD root"));
    }
    if root.attribute("type").is_some_and(|kind| kind != "static") {
        return Err(bad("dynamic DASH is unsupported"));
    }
    for node in root.descendants().filter(Node::is_element) {
        if matches!(
            node.tag_name().name(),
            "SegmentTemplate"
                | "SegmentList"
                | "SegmentTimeline"
                | "S"
                | "Initialization"
                | "SegmentURL"
        ) && node
            .attributes()
            .any(|attribute| attribute.namespace().is_some())
        {
            return Err(bad("namespaced DASH addressing attributes are unsupported"));
        }
        if node.ancestors().take(66).count() > 65 {
            return Err(bad("DASH XML nesting exceeds 64 element levels"));
        }
        if matches!(
            node.tag_name().name(),
            "EssentialProperty" | "SubRepresentation" | "Location" | "PatchLocation"
        ) || node.attributes().any(|a| {
            matches!(
                a.name(),
                "dependencyId"
                    | "dependencyLevel"
                    | "scalableEnhancement"
                    | "indexRange"
                    | "indexRangeExact"
            )
        }) {
            return Err(bad(
                "DASH dependent representations, essential properties, location remapping and indexes are unsupported",
            ));
        }
        let parent = node.parent_element().map(|parent| parent.tag_name().name());
        let valid_parent = match node.tag_name().name() {
            "Period" => parent == Some("MPD"),
            "AdaptationSet" => parent == Some("Period"),
            "Representation" => parent == Some("AdaptationSet"),
            "BaseURL" | "SegmentTemplate" | "SegmentList" => matches!(
                parent,
                Some("MPD" | "Period" | "AdaptationSet" | "Representation")
            ),
            "SegmentTimeline" | "Initialization" => {
                matches!(parent, Some("SegmentTemplate" | "SegmentList"))
            }
            "SegmentURL" => parent == Some("SegmentList"),
            "S" => parent == Some("SegmentTimeline"),
            _ => true,
        };
        if !valid_parent {
            return Err(bad("misplaced DASH addressing or representation element"));
        }
        if node.tag_name().namespace() != root.tag_name().namespace() {
            return Err(bad("unsupported DASH extension namespace"));
        }
        if matches!(
            node.tag_name().name(),
            "ContentProtection" | "SegmentBase" | "RepresentationIndex"
        ) {
            return Err(bad("DASH encryption and SIDX addressing are unsupported"));
        }
        if node.attributes().any(|a| {
            a.namespace() == Some("http://www.w3.org/1999/xlink")
                || a.namespace() == Some("http://www.w3.org/XML/1998/namespace")
                    && a.name() == "base"
        }) {
            return Err(bad("DASH xlink/xml:base is unsupported"));
        }
    }
    let periods = children(root, "Period");
    if periods.len() != 1 {
        return Err(bad("DASH requires exactly one Period"));
    }
    let period = periods[0];
    let start = period
        .attribute("start")
        .map(duration)
        .transpose()?
        .unwrap_or(0);
    // Parse both declarations even if the Period supplies its own duration.
    // This single-period subset requires the presentation to end with that Period.
    let presentation_duration = root
        .attribute("mediaPresentationDuration")
        .map(duration)
        .transpose()?;
    let period_duration = period.attribute("duration").map(duration).transpose()?;
    if let Some(total) = presentation_duration {
        if start > total {
            return Err(bad("DASH Period starts after presentation end"));
        }
        if let Some(length) = period_duration
            && add(start, length)? != total
        {
            return Err(bad("DASH Period end disagrees with presentation duration"));
        }
    }
    let duration_ns = match (period_duration, presentation_duration) {
        (Some(length), _) => Some(length),
        (None, Some(total)) => Some(total - start),
        (None, None) => None,
    };
    if duration_ns == Some(0) {
        return Err(bad("empty DASH period"));
    }
    let (root_base, root_explicit) = base_url(root, manifest, false)?;
    let (period_base, period_explicit) = base_url(period, &root_base, root_explicit)?;
    let address = Address::default().inherit(root)?.inherit(period)?;
    let mut formats = Vec::new();
    let mut subtitles = Vec::new();
    let mut total_fragments = 0usize;
    // Upstream `stream_numbers[mpd_url]`: video, audio and image
    // representations count in manifest order.
    let mut stream_number = 0usize;
    for adaptation in children(period, "AdaptationSet") {
        let (adaptation_base, adaptation_explicit) =
            base_url(adaptation, &period_base, period_explicit)?;
        let address = address.inherit(adaptation)?;
        for representation in children(adaptation, "Representation") {
            if formats.len() == 256 {
                return Err(bad("DASH representation limit exceeded"));
            }
            let (base, explicit) = base_url(representation, &adaptation_base, adaptation_explicit)?;
            let address = address.inherit(representation)?;
            let mut attrs = BTreeMap::new();
            for node in [root, period, adaptation, representation] {
                for attr in node.attributes() {
                    if attr.namespace().is_none() {
                        attrs.insert(attr.name(), attr.value());
                    }
                }
                if let Some(language) =
                    node.attribute(("http://www.w3.org/XML/1998/namespace", "lang"))
                {
                    attrs.insert("lang", language);
                }
            }
            let mime = attrs
                .get("mimeType")
                .copied()
                .ok_or_else(|| bad("DASH representation has no MIME type"))?;
            // Upstream: `contentType`, else the MIME type's major type; other
            // types are classified by their `parse_codecs` result.
            let codecs = crate::codecs::parse(attrs.get("codecs").copied());
            let declared = attrs
                .get("contentType")
                .copied()
                .unwrap_or_else(|| mime.split_once('/').map_or(mime, |(kind, _)| kind));
            let content_type = match declared {
                "video" | "audio" | "text" => declared,
                _ if matches!(mime, "image/avif" | "image/jpeg") => {
                    stream_number += 1;
                    continue;
                }
                _ => match &codecs {
                    Some(codecs) if codecs.vcodec != "none" => "video",
                    Some(codecs) if codecs.acodec != "none" => "audio",
                    Some(codecs) if codecs.scodec.is_some() => "text",
                    _ if matches!(
                        crate::extractor::direct::mimetype2ext_or(mime, None).as_str(),
                        "tt" | "dfxp" | "ttml" | "xml" | "json"
                    ) =>
                    {
                        "text"
                    }
                    _ => {
                        crate::screen::warning(&format!(
                            "Unknown MIME type {mime} in DASH manifest"
                        ));
                        continue;
                    }
                },
            };
            if content_type == "text" {
                let language = attrs.get("lang").map_or("und", |lang| *lang).to_owned();
                let ext = crate::extractor::direct::mimetype2ext_or(mime, None);
                if address.kind.is_none() {
                    // "Assuming direct URL to unfragmented media."
                    if explicit && !base.path().ends_with('/') {
                        subtitles.push((
                            language,
                            crate::SubtitleFormat {
                                extra: text_extra(manifest, None),
                                fragments: Vec::new(),
                                name: None,
                                url: base.clone(),
                                ext,
                                protocol: None,
                            },
                        ));
                    }
                } else {
                    // Segmented text: the manifest URL with its fragments.
                    let bandwidth = attrs
                        .get("bandwidth")
                        .and_then(|value| positive(value).ok());
                    let fragments = plans(
                        &address,
                        &base,
                        explicit,
                        duration_ns,
                        representation.attribute("id"),
                        bandwidth,
                    )?;
                    subtitles.push((
                        language,
                        crate::SubtitleFormat {
                            extra: text_extra(
                                manifest,
                                Some(if explicit {
                                    base.clone()
                                } else {
                                    base.join(".")?
                                }),
                            ),
                            fragments,
                            name: None,
                            url: manifest.clone(),
                            ext,
                            protocol: Some("http_dash_segments".into()),
                        },
                    ));
                }
                continue;
            }
            // Selection kinds follow upstream's codec fields (`none` marks a
            // missing stream); without codecs the stream counts as complete.
            let kind = match &codecs {
                Some(codecs) if codecs.vcodec == "none" => StreamKind::AudioOnly,
                Some(codecs) if codecs.acodec == "none" => StreamKind::VideoOnly,
                _ => StreamKind::Standalone,
            };
            let bandwidth = attrs
                .get("bandwidth")
                .map(|value| positive(value))
                .transpose()?;
            // Upstream `mimetype2ext(mime_type)`, also naming the `EXT_dash` container.
            let ext = crate::extractor::direct::mimetype2ext_or(mime, None);
            let tbr = bandwidth.map(|value| value as f64 / 1000.0);
            let dimension = |name| {
                attrs
                    .get(name)
                    .map(|value| {
                        u32::try_from(positive(value)?).map_err(|_| bad("DASH dimension overflow"))
                    })
                    .transpose()
            };
            let fragments = plans(
                &address,
                &base,
                explicit,
                duration_ns,
                representation.attribute("id"),
                bandwidth,
            )?;
            total_fragments = total_fragments
                .checked_add(fragments.len())
                .filter(|count| *count <= LIMIT)
                .ok_or_else(|| bad("DASH total fragment limit exceeded"))?;
            // Upstream: the representation ID, else the content type.
            let format_id = match representation.attribute("id") {
                Some(id) => id.to_owned(),
                None => content_type.to_owned(),
            };
            // Upstream `**parse_codecs(codecs)`: no codec keys without one.
            // Upstream keys: the MPD as `manifest_url`, the representation
            // base for relative fragment paths, and `_merge_mpd_periods`'s
            // `is_dash_periods`.
            let mut extra = serde_json::Map::new();
            extra.insert(crate::info_order::FORMAT_SHAPE.into(), "dash".into());
            extra.insert("manifest_url".into(), manifest.as_str().into());
            if address.kind.is_some() {
                // Without a BaseURL, upstream's base is the MPD's directory.
                let directory;
                let fragment_base = if explicit {
                    base.as_str()
                } else {
                    directory = base.join(".")?;
                    directory.as_str()
                };
                extra.insert("fragment_base_url".into(), fragment_base.into());
            }
            extra.insert("is_dash_periods".into(), true.into());
            formats.push(VideoFormat {
                http_headers: None,
                extra,
                protocol: Some("http_dash_segments".into()),
                vcodec: codecs.as_ref().map(|codecs| codecs.vcodec.clone()),
                acodec: codecs.as_ref().map(|codecs| codecs.acodec.clone()),
                dynamic_range: codecs
                    .as_ref()
                    .and_then(|codecs| codecs.dynamic_range)
                    .map(str::to_owned),
                video_ext: None,
                audio_ext: None,
                vbr: None,
                source_preference: None,
                quality: None,
                // Upstream `int_or_none(frameRate)` (fractions stay unknown)
                // and `int_or_none(audioSamplingRate)`.
                fps: attrs
                    .get("frameRate")
                    .and_then(|value| value.trim().parse::<i64>().ok())
                    .map(crate::info::PyNumber::Int),
                asr: attrs
                    .get("audioSamplingRate")
                    .and_then(|value| value.trim().parse().ok()),
                audio_channels: None,
                resolution: None,
                format: None,
                aspect_ratio: None,
                format_id,
                url: manifest.clone(),
                // Upstream `mimetype2ext(mime_type)`.
                ext: ext.clone(),
                stream_kind: kind,
                audio_group: (kind != StreamKind::Standalone).then(|| "dash-period-0".into()),
                // Upstream drops the `mul`/`und`/`zxx`/`mis` language codes.
                language: attrs
                    .get("lang")
                    .filter(|lang| !matches!(**lang, "mul" | "und" | "zxx" | "mis"))
                    .map(|value| (*value).to_owned()),
                format_note: Some(format!("DASH {content_type}")),
                audio_default: false,
                audio_autoselect: false,
                downloader_options: None,
                manifest_stream_number: Some(stream_number),
                format_index: None,
                container: Some(format!("{ext}_dash")),
                abr: if kind == StreamKind::AudioOnly {
                    tbr
                } else {
                    None
                },
                tbr,
                width: dimension("width")?,
                height: dimension("height")?,
                filesize: None,
                fragments,
            });
            stream_number += 1;
        }
    }
    if formats.is_empty() {
        return Err(bad("DASH has no supported representations"));
    }
    Ok((formats, subtitles))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> Url {
        Url::parse("https://example.com/path/manifest.mpd?token=1").unwrap()
    }
    fn wrap(body: &str) -> String {
        format!(
            "<MPD xmlns=\"urn:mpeg:dash:schema:mpd:2011\" type=\"static\" mediaPresentationDuration=\"PT2.5S\"><Period>{body}</Period></MPD>"
        )
    }
    #[test]
    fn direct_text_representations_become_subtitles() {
        let text = wrap(&format!(
            "{}<AdaptationSet contentType=\"text\" mimeType=\"text/vtt\" lang=\"en\"><Representation id=\"s\" bandwidth=\"1\"><BaseURL>en.vtt</BaseURL></Representation></AdaptationSet>\
             <AdaptationSet mimeType=\"application/ttml+xml\"><Representation id=\"t\" bandwidth=\"1\"><BaseURL>t.ttml</BaseURL></Representation></AdaptationSet>",
            template("")
                .split("<Period>")
                .nth(1)
                .unwrap()
                .split("</Period>")
                .next()
                .unwrap()
        ));
        let (formats, subtitles) = parse_with_subtitles(&text, &manifest()).unwrap();
        assert_eq!(formats.len(), 1);
        let subtitles: Vec<_> = subtitles
            .iter()
            .map(|(language, format)| (language.as_str(), format.url.as_str(), format.ext.as_str()))
            .collect();
        assert_eq!(
            subtitles,
            [
                ("en", "https://example.com/path/en.vtt", "vtt"),
                ("und", "https://example.com/path/t.ttml", "ttml")
            ]
        );
    }
    fn template(extra: &str) -> String {
        wrap(&format!(
            "<AdaptationSet mimeType=\"video/mp4\" codecs=\"avc1.640028\"><SegmentTemplate timescale=\"10\" duration=\"10\" initialization=\"init-$RepresentationID$.mp4\" media=\"part-$Number%05d$-$Bandwidth$-$$.m4s\" {extra}/><Representation id=\"v\" bandwidth=\"2000000\" width=\"1920\" height=\"1080\"/></AdaptationSet>"
        ))
    }
    fn urls(format: &VideoFormat) -> Vec<&str> {
        format.fragments.iter().map(|f| f.url.as_str()).collect()
    }
    #[test]
    fn duration_template_uses_integer_ceil_tokens_offsets_and_representation_metadata() {
        let formats = parse(
            &template("presentationTimeOffset=\"7\" startNumber=\"4\""),
            &manifest(),
        )
        .unwrap();
        let video = &formats[0];
        // Upstream names DASH formats by representation ID.
        assert_eq!(video.format_id, "v");
        assert_eq!(video.url, manifest());
        assert_eq!(video.stream_kind, StreamKind::VideoOnly);
        assert_eq!(video.audio_group.as_deref(), Some("dash-period-0"));
        assert_eq!(video.tbr, Some(2000.0));
        assert_eq!((video.width, video.height), (Some(1920), Some(1080)));
        assert_eq!(
            urls(video),
            [
                "https://example.com/path/init-v.mp4",
                "https://example.com/path/part-00004-2000000-$.m4s",
                "https://example.com/path/part-00005-2000000-$.m4s",
                "https://example.com/path/part-00006-2000000-$.m4s"
            ]
        );
    }
    #[test]
    fn exact_duration_arithmetic_includes_nanoseconds_days_hours_and_minutes() {
        assert_eq!(
            duration("P1DT2H3M4.123456789S").unwrap(),
            93_784_123_456_789
        );
        assert_eq!(duration("PT0.000000001S").unwrap(), 1);
        let text = template("").replace("PT2.5S", "PT0.300000001S").replace(
            "timescale=\"10\" duration=\"10\"",
            "timescale=\"10\" duration=\"1\"",
        );
        assert_eq!(parse(&text, &manifest()).unwrap()[0].fragments.len(), 5);
        for text in [
            "P",
            "PT",
            "P1Y",
            "P1M",
            "PT1.5H",
            "PT1.1234567890S",
            "PT-1S",
            "PT1S1M",
        ] {
            assert!(duration(text).is_err(), "{text}");
        }
    }
    #[test]
    fn unknown_codecs_and_types_follow_upstream_classification() {
        let text = template("");
        // Unknown codecs only warn; the representation stays a format.
        let formats = parse(&text.replace("avc1.640028", "unknown"), &manifest()).unwrap();
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].stream_kind, StreamKind::Standalone);
        // application/mp4 is classified by its codecs; text tracks are skipped.
        let application = text.replace("video/mp4", "application/mp4");
        assert_eq!(parse(&application, &manifest()).unwrap()[0].format_id, "v");
        assert!(parse(&text.replace("video/mp4", "text/vtt"), &manifest()).is_err());
    }

    #[test]
    fn base_and_template_inheritance_and_language_are_deterministic() {
        let text = "<MPD><BaseURL>https://cdn.example/root/</BaseURL><SegmentTemplate timescale=\"100\" initialization=\"init-$RepresentationID$.mp4\" media=\"$Time$.m4s\"><SegmentTimeline><S t=\"10\" d=\"20\" r=\"1\"/></SegmentTimeline></SegmentTemplate><Period><BaseURL>period/</BaseURL><AdaptationSet mimeType=\"audio/mp4\" lang=\"en\"><BaseURL>audio/</BaseURL><Representation id=\"a\" bandwidth=\"128000\"><BaseURL>variant/</BaseURL></Representation></AdaptationSet></Period></MPD>";
        let formats = parse(text, &manifest()).unwrap();
        let audio = &formats[0];
        assert_eq!(audio.ext, "m4a");
        assert_eq!(audio.language.as_deref(), Some("en"));
        // Without codecs upstream knows only the total bitrate.
        assert_eq!(audio.tbr, Some(128.0));
        assert_eq!(audio.abr, None);
        assert_eq!(
            urls(audio),
            [
                "https://cdn.example/root/period/audio/variant/init-a.mp4",
                "https://cdn.example/root/period/audio/variant/10.m4s",
                "https://cdn.example/root/period/audio/variant/30.m4s"
            ]
        );
    }
    #[test]
    fn timelines_expand_finite_and_bounded_negative_repeats_without_float_rounding() {
        let body = "<AdaptationSet mimeType=\"video/webm\"><SegmentTemplate timescale=\"10\" initialization=\"init.webm\" media=\"$Time$.webm\"><SegmentTimeline><S t=\"5\" d=\"5\" r=\"-1\"/><S t=\"20\" d=\"10\" r=\"1\"/></SegmentTimeline></SegmentTemplate><Representation/></AdaptationSet>";
        let formats = parse(&wrap(body).replace("PT2.5S", "PT4S"), &manifest()).unwrap();
        assert_eq!(formats[0].fragments.len(), 6);
        assert!(urls(&formats[0])[4].ends_with("20.webm"));
        let text = wrap(body)
            .replace("<S t=\"20\" d=\"10\" r=\"1\"/>", "")
            .replace("PT2.5S", "PT1.6S");
        assert_eq!(parse(&text, &manifest()).unwrap()[0].fragments.len(), 4);
        let unbounded = text.replace(" mediaPresentationDuration=\"PT1.6S\"", "");
        assert!(parse(&unbounded, &manifest()).is_err());
        assert!(parse(&wrap(body).replace("t=\"20\"", "t=\"21\""), &manifest()).is_err());
    }
    #[test]
    fn negative_repeat_period_end_honors_presentation_time_offset() {
        let text = wrap(
            "<AdaptationSet mimeType=\"video/mp4\"><SegmentTemplate timescale=\"10\" presentationTimeOffset=\"100\" initialization=\"init.mp4\" media=\"$Time$.m4s\"><SegmentTimeline><S t=\"100\" d=\"10\" r=\"-1\"/></SegmentTimeline></SegmentTemplate><Representation/></AdaptationSet>",
        );
        let formats = parse(&text, &manifest()).unwrap();
        assert_eq!(formats[0].fragments.len(), 4);
        assert!(urls(&formats[0])[3].ends_with("120.m4s"));
        assert!(parse(&text.replace("<S t=\"100\"", "<S t=\"0\""), &manifest()).is_err());
        assert!(parse(&text.replace("<S t=\"100\" ", "<S "), &manifest()).is_err());
    }
    #[test]
    fn inherited_segment_list_ranges_are_inclusive_and_can_use_resource_base() {
        let text = wrap(
            "<AdaptationSet mimeType=\"audio/mp4\"><BaseURL>combined.mp4</BaseURL><SegmentList><Initialization range=\"0-9\"/><SegmentURL mediaRange=\"10-29\"/><SegmentURL mediaRange=\"30-34\"/></SegmentList><Representation id=\"a\"/></AdaptationSet>",
        );
        let formats = parse(&text, &manifest()).unwrap();
        let fragments = &formats[0].fragments;
        assert_eq!(fragments.len(), 3);
        assert!(
            fragments
                .iter()
                .all(|f| f.url.as_str() == "https://example.com/path/combined.mp4")
        );
        assert_eq!(
            fragments[0].byte_range,
            Some(ByteRange {
                offset: 0,
                length: 10
            })
        );
        assert_eq!(
            fragments[1].byte_range,
            Some(ByteRange {
                offset: 10,
                length: 20
            })
        );
        assert_eq!(
            fragments[2].byte_range,
            Some(ByteRange {
                offset: 30,
                length: 5
            })
        );
        assert!(
            parse(
                &text.replace("<BaseURL>combined.mp4</BaseURL>", ""),
                &manifest()
            )
            .is_err()
        );
        for range in ["1-0", "-1", "0-18446744073709551615", "1-2-3"] {
            assert!(parse(&text.replace("0-9", range), &manifest()).is_err());
        }
    }
    #[test]
    fn direct_resources_require_explicit_non_directory_base_and_never_download_manifest() {
        let text = wrap(
            "<AdaptationSet mimeType=\"application/mp4\" contentType=\"video\" codecs=\"avc1.4,mp4a.40.2\"><Representation><BaseURL>movie.mp4</BaseURL></Representation></AdaptationSet>",
        );
        let formats = parse(&text, &manifest()).unwrap();
        assert_eq!(formats[0].stream_kind, StreamKind::Standalone);
        assert!(formats[0].audio_group.is_none());
        assert_eq!(urls(&formats[0]), ["https://example.com/path/movie.mp4"]);
        assert!(
            parse(
                &text.replace("<BaseURL>movie.mp4</BaseURL>", ""),
                &manifest()
            )
            .is_err()
        );
        assert!(parse(&text.replace("movie.mp4", "directory/"), &manifest()).is_err());
    }
    #[test]
    fn saved_ffmpeg_timeline_duration_list_and_range_manifests_have_two_typed_plans() {
        for text in [
            include_str!("../tests/fixtures/dash/media/manifest.mpd"),
            include_str!("../tests/fixtures/dash/media/number.mpd"),
            include_str!("../tests/fixtures/dash/media/list.mpd"),
            include_str!("../tests/fixtures/dash/media/ranges.mpd"),
        ] {
            let formats = parse(text, &manifest()).unwrap();
            assert_eq!(formats.len(), 2);
            assert_eq!(formats[0].stream_kind, StreamKind::VideoOnly);
            assert_eq!(formats[1].stream_kind, StreamKind::AudioOnly);
            assert!(formats.iter().all(|format| !format.fragments.is_empty()));
            assert_eq!(formats[0].format_id, "0");
            assert_eq!(formats[1].format_id, "1");
        }
    }
    #[test]
    fn unsupported_or_ambiguous_manifests_fail_before_producing_any_formats() {
        let text = template("");
        for changed in [
            text.replace("type=\"static\"", "type=\"dynamic\""),
            text.replace("</MPD>", "<Period/></MPD>"),
            text.replace("<Representation ", "<ContentProtection/><Representation "),
            text.replace("<Representation ", "<SegmentBase/><Representation "),
            text.replace(
                "<Representation ",
                "<BaseURL>a/</BaseURL><BaseURL>b/</BaseURL><Representation ",
            ),
            text.replace("<Representation ", "<SegmentList/><Representation "),
            text.replace(
                "<Period>",
                "<Period xmlns:xlink=\"http://www.w3.org/1999/xlink\" xlink:href=\"other.xml\">",
            ),
            text.replace("<Period>", "<Period xml:base=\"other/\">"),
            format!("<!DOCTYPE MPD [<!ENTITY x 'bad'>]>{text}"),
        ] {
            assert!(parse(&changed, &manifest()).is_err(), "{changed}");
        }
        assert!(
            parse(
                &text.replace("init-$RepresentationID$.mp4", "file:///init.mp4"),
                &manifest()
            )
            .is_err()
        );
    }
    #[test]
    fn initialization_sequence_tokens_and_combined_media_tokens_are_rejected_without_misreading_escapes()
     {
        for token in ["$Number$", "$Number%05d$", "$Time$", "$Time%02d$"] {
            assert!(
                parse(
                    &template("")
                        .replace("init-$RepresentationID$.mp4", &format!("init-{token}.mp4")),
                    &manifest()
                )
                .is_err()
            );
        }
        assert_eq!(
            expand_template(
                "init-$$Number$$-$$Time$$-$RepresentationID$-$Bandwidth$.mp4",
                Some("video"),
                Some(1000),
                0,
                0,
                true
            )
            .unwrap(),
            "init-$Number$-$Time$-video-1000.mp4"
        );
        assert!(expand("$Number$-$Time$", None, None, 1, 2).is_err());
        assert_eq!(
            expand("$$Number$$-$Time$", None, None, 1, 2).unwrap(),
            "$Number$-2"
        );
        let text = template("presentationTimeOffset=\"7\"")
            .replace("part-$Number%05d$-$Bandwidth$-$$.m4s", "part-$Time$.m4s");
        let formats = parse(&text, &manifest()).unwrap();
        assert!(urls(&formats[0])[1].ends_with("part-7.m4s"));
        assert!(urls(&formats[0])[3].ends_with("part-27.m4s"));
    }

    #[test]
    fn malformed_templates_timing_and_arithmetic_overflow_fail_closed() {
        for value in [
            "$Unknown$",
            "$Number%021d$",
            "$Number%5d$",
            "$RepresentationID%02d$",
            "$Number",
            "$Number%00d$",
        ] {
            assert!(expand(value, Some("id"), Some(1), 1, 1).is_err(), "{value}");
        }
        let text = template("");
        for (from, to) in [
            ("duration=\"10\"", "duration=\"0\""),
            ("timescale=\"10\"", "timescale=\"0\""),
            ("duration=\"10\"", "duration=\"bad\""),
            ("id=\"v\"", ""),
            ("bandwidth=\"2000000\"", ""),
            (
                "duration=\"10\"",
                "duration=\"1\" startNumber=\"18446744073709551615\"",
            ),
        ] {
            assert!(parse(&text.replace(from, to), &manifest()).is_err(), "{to}");
        }
        let timeline = wrap(
            "<AdaptationSet mimeType=\"video/mp4\"><SegmentTemplate initialization=\"i.mp4\" media=\"$Time$.m4s\"><SegmentTimeline><S t=\"18446744073709551615\" d=\"1\"/></SegmentTimeline></SegmentTemplate><Representation/></AdaptationSet>",
        );
        assert!(parse(&timeline, &manifest()).is_err());
    }
    #[test]
    fn dependent_representations_essential_properties_remapping_and_deep_xml_fail_closed() {
        let text = template("");
        for attribute in [
            "dependencyId=\"base\"",
            "dependencyLevel=\"1\"",
            "scalableEnhancement=\"true\"",
            "indexRange=\"0-10\"",
        ] {
            assert!(
                parse(
                    &text.replace("id=\"v\"", &format!("id=\"v\" {attribute}")),
                    &manifest()
                )
                .is_err()
            );
        }
        for element in [
            "<EssentialProperty schemeIdUri=\"unknown\"/>",
            "<SubRepresentation/>",
            "<SegmentURL media=\"ignored.m4s\"/>",
            "<Location>other.mpd</Location>",
            "<PatchLocation>other.mpd</PatchLocation>",
        ] {
            assert!(
                parse(
                    &text.replace("<Representation ", &format!("{element}<Representation ")),
                    &manifest()
                )
                .is_err()
            );
        }
        let deep = text
            .replace(
                "<Representation ",
                &format!("{}<Representation ", "<Extension>".repeat(70)),
            )
            .replace(
                "</AdaptationSet>",
                &format!("{}</AdaptationSet>", "</Extension>".repeat(70)),
            );
        assert!(parse(&deep, &manifest()).is_err());
    }

    #[test]
    fn entries_past_a_finite_period_end_and_missing_initialization_are_kept_like_upstream() {
        // Upstream lists every S entry and SegmentURL without Period checks.
        let text = wrap("<AdaptationSet mimeType=\"video/mp4\"><SegmentTemplate initialization=\"init.mp4\" media=\"$Time$.m4s\"><SegmentTimeline><S t=\"0\" d=\"1\" r=\"5\"/></SegmentTimeline></SegmentTemplate><Representation/></AdaptationSet>").replace("PT2.5S", "PT2S");
        assert_eq!(parse(&text, &manifest()).unwrap()[0].fragments.len(), 7);
        let list = wrap("<AdaptationSet mimeType=\"video/mp4\"><SegmentList timescale=\"10\" duration=\"10\" presentationTimeOffset=\"100\"><Initialization sourceURL=\"init.mp4\"/><SegmentURL media=\"one.m4s\"/><SegmentURL media=\"two.m4s\"/><SegmentURL media=\"three.m4s\"/></SegmentList><Representation/></AdaptationSet>").replace("PT2.5S", "PT2S");
        assert_eq!(parse(&list, &manifest()).unwrap()[0].fragments.len(), 4);
        // Initialization is optional for lists and templates.
        let bare = list.replace("<Initialization sourceURL=\"init.mp4\"/>", "");
        let fragments = &parse(&bare, &manifest()).unwrap()[0].fragments;
        assert_eq!(fragments.len(), 3);
        assert!(fragments[0].url.as_str().ends_with("one.m4s"));
        let template = text.replace(" initialization=\"init.mp4\"", "");
        assert_eq!(parse(&template, &manifest()).unwrap()[0].fragments.len(), 6);
    }

    #[test]
    fn foreign_namespaced_address_attributes_never_become_dash_attributes() {
        for (from, to) in [
            ("media=\"", "x:media=\""),
            ("duration=\"10\"", "x:duration=\"10\""),
            ("initialization=\"", "x:initialization=\""),
        ] {
            let text = template("")
                .replace(
                    "<SegmentTemplate ",
                    "<SegmentTemplate xmlns:x=\"urn:foreign\" ",
                )
                .replace(from, to);
            assert!(parse(&text, &manifest()).is_err(), "{to}");
        }
        for text in [
            "<SegmentList><Initialization xmlns:x=\"urn:foreign\" x:sourceURL=\"init.mp4\"/><SegmentURL media=\"one.m4s\"/></SegmentList>",
            "<SegmentList><Initialization sourceURL=\"init.mp4\"/><SegmentURL xmlns:x=\"urn:foreign\" x:media=\"one.m4s\"/></SegmentList>",
            "<SegmentTemplate initialization=\"init.mp4\" media=\"$Time$.m4s\"><SegmentTimeline><S xmlns:x=\"urn:foreign\" x:t=\"0\" d=\"1\"/></SegmentTimeline></SegmentTemplate>",
        ] {
            assert!(parse(&wrap(&format!("<AdaptationSet mimeType=\"video/mp4\">{text}<Representation/></AdaptationSet>")), &manifest()).is_err());
        }
    }

    #[test]
    fn explicit_period_duration_requires_a_valid_consistent_presentation_end() {
        let text = template("")
            .replace("<Period>", "<Period start=\"PT0.1S\" duration=\"PT0.3S\">")
            .replace("PT2.5S", "PT0.4S");
        assert!(parse(&text, &manifest()).is_ok());
        for total in ["invalid", "PT0.3S", "PT0.5S", "PT0.05S"] {
            assert!(
                parse(
                    &text.replace(
                        "mediaPresentationDuration=\"PT0.4S\"",
                        &format!("mediaPresentationDuration=\"{total}\"")
                    ),
                    &manifest()
                )
                .is_err(),
                "{total}"
            );
        }
        assert!(
            parse(
                &text.replace("start=\"PT0.1S\"", "start=\"PT1S\""),
                &manifest()
            )
            .is_err()
        );
        assert!(
            parse(
                &text.replace(" mediaPresentationDuration=\"PT0.4S\"", ""),
                &manifest()
            )
            .is_ok()
        );
    }

    #[test]
    fn aggregate_fragment_and_representation_limits_are_enforced() {
        let rep = "<Representation mimeType=\"video/mp4\"><BaseURL>resource.mp4</BaseURL></Representation>";
        assert!(
            parse(
                &wrap(&format!(
                    "<AdaptationSet>{}</AdaptationSet>",
                    rep.repeat(256)
                )),
                &manifest()
            )
            .is_ok()
        );
        assert!(
            parse(
                &wrap(&format!(
                    "<AdaptationSet>{}</AdaptationSet>",
                    rep.repeat(257)
                )),
                &manifest()
            )
            .is_err()
        );
        let text = template("").replace("PT2.5S", "PT10000S");
        assert!(parse(&text, &manifest()).is_err());
        let text = text.replace("PT10000S", "PT5000S").replace(
            "</AdaptationSet>",
            "<Representation id=\"other\" bandwidth=\"2000000\"/></AdaptationSet>",
        );
        assert!(parse(&text, &manifest()).is_err());
    }
}

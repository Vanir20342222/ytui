//! Upstream `ModifyChaptersPP` chapter arithmetic: marking chapters to
//! remove, merging cuts while re-timing the remaining (and sponsor)
//! chapters, and the concat demuxer options that cut the media.
use crate::info::{Chapter, PyNumber};
use std::{cmp::Ordering, collections::BinaryHeap, ffi::OsString, path::Path};

/// `_TINY_CHAPTER_DURATION`.
const TINY_CHAPTER_DURATION: f64 = 1.0;

/// A `_categories` entry: category, start, end and category name.
pub(crate) type Category = (String, PyNumber, PyNumber, String);

/// A chapter, sponsor chapter or cut during arrangement (the upstream dict
/// with its private keys).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Segment {
    pub start: PyNumber,
    pub end: PyNumber,
    pub title: Option<String>,
    /// A SponsorBlock chapter's `type` (action type).
    pub action: Option<String>,
    pub remove: bool,
    /// `_categories`, present (possibly empty) for sponsor chapters.
    pub categories: Option<Vec<Category>>,
    was_cut: bool,
    cut_idx: Option<usize>,
}

impl Segment {
    /// A `sponsorblock_chapters` entry (`SponsorBlockPP`'s `to_chapter`).
    pub(crate) fn from_sponsor_json(value: &serde_json::Value) -> Option<Self> {
        let number =
            |value: &serde_json::Value| serde_json::from_value::<PyNumber>(value.clone()).ok();
        let categories = value["_categories"]
            .as_array()?
            .iter()
            .map(|category| {
                Some((
                    category[0].as_str()?.to_owned(),
                    number(&category[1])?,
                    number(&category[2])?,
                    category[3].as_str()?.to_owned(),
                ))
            })
            .collect::<Option<Vec<_>>>()?;
        Some(Self {
            start: number(&value["start_time"])?,
            end: number(&value["end_time"])?,
            title: value["title"].as_str().map(str::to_owned),
            action: value["type"].as_str().map(str::to_owned),
            remove: false,
            categories: Some(categories),
            was_cut: false,
            cut_idx: None,
        })
    }

    /// A sponsor chapter's category (`c['category']`).
    pub(crate) fn category(&self) -> Option<&str> {
        self.categories
            .as_ref()?
            .first()
            .map(|category| category.0.as_str())
    }

    pub(crate) fn chapter(start: PyNumber, end: PyNumber, title: Option<String>) -> Self {
        Self {
            start,
            end,
            title,
            action: None,
            remove: false,
            categories: None,
            was_cut: false,
            cut_idx: None,
        }
    }
}

/// Sponsor fields of a renamed sponsor chapter.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sponsor {
    pub category: String,
    pub categories: Vec<String>,
    pub name: String,
    pub category_names: Vec<String>,
}

/// A resulting chapter.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Arranged {
    pub start: PyNumber,
    pub end: PyNumber,
    pub title: Option<String>,
    pub action: Option<String>,
    pub sponsor: Option<Sponsor>,
}

impl Arranged {
    pub(crate) fn chapter(&self) -> Chapter {
        let mut extra = serde_json::Map::new();
        if let Some(sponsor) = &self.sponsor {
            extra.insert("category".into(), sponsor.category.clone().into());
            if let Some(action) = &self.action {
                extra.insert("type".into(), action.clone().into());
            }
            extra.insert("categories".into(), sponsor.categories.clone().into());
            extra.insert("name".into(), sponsor.name.clone().into());
            extra.insert(
                "category_names".into(),
                sponsor.category_names.clone().into(),
            );
        }
        Chapter {
            start_time: Some(self.start),
            end_time: Some(self.end),
            title: self.title.clone(),
            extra,
        }
    }
}

fn float(value: PyNumber) -> f64 {
    value.as_f64()
}

/// Python `a + b` and `a - b` for `int`/`float`.
fn add(a: PyNumber, b: PyNumber) -> PyNumber {
    match (a, b) {
        (PyNumber::Int(a), PyNumber::Int(b)) => a
            .checked_add(b)
            .map_or(PyNumber::Float(a as f64 + b as f64), PyNumber::Int),
        (a, b) => PyNumber::Float(float(a) + float(b)),
    }
}

fn sub(a: PyNumber, b: PyNumber) -> PyNumber {
    match (a, b) {
        (PyNumber::Int(a), PyNumber::Int(b)) => a
            .checked_sub(b)
            .map_or(PyNumber::Float(a as f64 - b as f64), PyNumber::Int),
        (a, b) => PyNumber::Float(float(a) - float(b)),
    }
}

/// Python `max(a, b)`/`min(a, b)`: the first of equal values.
fn max(a: PyNumber, b: PyNumber) -> PyNumber {
    if float(b) > float(a) { b } else { a }
}

fn min(a: PyNumber, b: PyNumber) -> PyNumber {
    if float(b) < float(a) { b } else { a }
}

/// A priority-queue entry ordered by `(start_time, index)`, like upstream's
/// heap tuples (later pushes break remaining ties).
struct Entry {
    start: f64,
    index: usize,
    sequence: usize,
    segment: Segment,
}

impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reversed: `BinaryHeap` is a max-heap.
        other
            .start
            .total_cmp(&self.start)
            .then(other.index.cmp(&self.index))
            .then(other.sequence.cmp(&self.sequence))
    }
}

impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for Entry {}

#[derive(Default)]
struct Queue {
    heap: BinaryHeap<Entry>,
    sequence: usize,
}

impl Queue {
    fn push(&mut self, index: usize, segment: Segment) {
        self.sequence += 1;
        self.heap.push(Entry {
            start: float(segment.start),
            index,
            sequence: self.sequence,
            segment,
        });
    }

    fn pop(&mut self) -> Option<(usize, Segment)> {
        self.heap.pop().map(|entry| (entry.index, entry.segment))
    }
}

/// `_remove_marked_arrange_sponsors`: the re-timed chapters and the merged
/// cuts. `title` renders a sponsor chapter's title from its category names
/// (upstream `--sponsorblock-chapter-title`).
#[cfg(test)]
pub(crate) fn arrange(
    chapters: Vec<Segment>,
    title: &dyn Fn(&Sponsor) -> String,
) -> (Vec<Arranged>, Vec<Segment>) {
    arrange_titled(chapters, &|sponsor, _| title(sponsor))
}

/// [`arrange`] whose title callback also gets the chapter dict
/// (`start_time`, `end_time`, `title`), as `evaluate_outtmpl` does.
pub(crate) fn arrange_titled(
    chapters: Vec<Segment>,
    title: &dyn Fn(&Sponsor, &serde_json::Value) -> String,
) -> (Vec<Arranged>, Vec<Segment>) {
    let mut cuts: Vec<Segment> = Vec::new();
    let mut arranged: Vec<Segment> = Vec::new();
    fn append_cut(cuts: &mut Vec<Segment>, cut: Segment) -> usize {
        debug_assert!(cut.remove, "Not a cut is appended to cuts");
        match cuts.last_mut() {
            Some(last) if float(last.end) >= float(cut.start) => {
                last.end = max(last.end, cut.end);
            }
            _ => cuts.push(cut),
        }
        cuts.len() - 1
    }
    // Cuts within the chapter reduce its duration; overlapping cuts are
    // merged by `append_cut`, so the first cut's index is enough.
    fn excess_duration(cuts: &[Segment], chapter: &mut Segment) -> PyNumber {
        let mut index = chapter.cut_idx.take().unwrap_or(cuts.len());
        let mut excess = PyNumber::Int(0);
        while index < cuts.len() {
            let cut = &cuts[index];
            if float(cut.start) >= float(chapter.end) {
                break;
            }
            if float(cut.end) > float(chapter.start) {
                excess = add(excess, min(cut.end, chapter.end));
                excess = sub(excess, max(cut.start, chapter.start));
            }
            index += 1;
        }
        excess
    }
    fn append_chapter(cuts: &[Segment], arranged: &mut Vec<Segment>, mut chapter: Segment) {
        debug_assert!(!chapter.remove, "Cut is appended to chapters");
        let excess = excess_duration(cuts, &mut chapter);
        let length = sub(sub(chapter.end, chapter.start), excess);
        // Completely covered by cuts or sponsors.
        if float(length) <= 0.0 {
            return;
        }
        let start = arranged.last().map_or(PyNumber::Int(0), |last| last.end);
        chapter.start = start;
        chapter.end = add(start, length);
        arranged.push(chapter);
    }
    let finish = |cuts: &mut Vec<Segment>, arranged: &mut Vec<Segment>, segment: Segment| {
        if segment.remove {
            append_cut(cuts, segment);
        } else {
            append_chapter(cuts, arranged, segment);
        }
    };
    let mut queue = Queue::default();
    for (index, chapter) in chapters.into_iter().enumerate() {
        queue.push(index, chapter);
    }
    let Some((mut current_index, mut current)) = queue.pop() else {
        return (Vec::new(), cuts);
    };
    while let Some((index, mut chapter)) = queue.pop() {
        // Non-overlapping chapters or cuts are appended directly.
        if float(current.end) <= float(chapter.start) {
            finish(&mut cuts, &mut arranged, current);
            (current_index, current) = (index, chapter);
            continue;
        }
        if current.remove {
            if chapter.remove {
                // (cut, cut): extend.
                current.end = max(current.end, chapter.end);
            } else if float(current.end) < float(chapter.end) {
                // (cut, sponsor/normal): chop the beginning of the later one.
                chapter.start = current.end;
                chapter.was_cut = true;
                queue.push(index, chapter);
            }
        } else if chapter.remove {
            // (sponsor/normal, cut).
            current.was_cut = true;
            if float(current.end) <= float(chapter.end) {
                current.end = chapter.start;
                append_chapter(&cuts, &mut arranged, current);
                (current_index, current) = (index, chapter);
                continue;
            }
            // The cut lies within the current chapter: a sponsor chapter is
            // split when its categories before and after the cut differ.
            if let Some(categories) = current.categories.clone() {
                let mut after = current.clone();
                after.start = chapter.end;
                let mut before = Vec::new();
                let mut later = Vec::new();
                for category in categories {
                    if float(category.1) < float(chapter.start) {
                        before.push(category.clone());
                    }
                    if float(category.2) > float(chapter.end) {
                        later.push(category);
                    }
                }
                after.categories = Some(later);
                current.categories = Some(before);
                if current.categories != after.categories {
                    queue.push(current_index, after);
                    current.end = chapter.start;
                    append_chapter(&cuts, &mut arranged, current);
                    (current_index, current) = (index, chapter);
                    continue;
                }
            }
            // An outstanding cut reduces the next appended chapter.
            let cut = append_cut(&mut cuts, chapter);
            current.cut_idx.get_or_insert(cut);
        } else if current.categories.is_some() && chapter.categories.is_none() {
            // (sponsor, normal): chop the normal chapter's beginning.
            if float(current.end) < float(chapter.end) {
                chapter.start = current.end;
                chapter.was_cut = true;
                queue.push(index, chapter);
            }
        } else {
            // (normal, sponsor) and (sponsor, sponsor).
            debug_assert!(chapter.categories.is_some(), "Normal chapters overlap");
            current.was_cut = true;
            chapter.was_cut = true;
            if float(current.end) > float(chapter.end) {
                let mut after = current.clone();
                after.start = chapter.end;
                queue.push(current_index, after);
            } else if float(chapter.end) > float(current.end) {
                let mut after = chapter.clone();
                after.start = current.end;
                queue.push(current_index, after);
                chapter.end = current.end;
            }
            if let Some(categories) = &current.categories {
                let mut merged = categories.clone();
                merged.extend(chapter.categories.take().unwrap_or_default());
                chapter.categories = Some(merged);
            }
            if current.cut_idx.is_some() {
                chapter.cut_idx = current.cut_idx;
            }
            current.end = chapter.start;
            append_chapter(&cuts, &mut arranged, current);
            (current_index, current) = (index, chapter);
        }
    }
    finish(&mut cuts, &mut arranged, current);
    (remove_tiny_rename_sponsors(arranged, title), cuts)
}

/// Python `orderedSet`.
fn ordered(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = Vec::new();
    for value in values {
        if !seen.contains(&value) {
            seen.push(value);
        }
    }
    seen
}

/// `_remove_tiny_rename_sponsors`: tiny chapters left by cuts merge into a
/// neighbour; sponsor chapters get their category fields and title.
fn remove_tiny_rename_sponsors(
    mut chapters: Vec<Segment>,
    title: &dyn Fn(&Sponsor, &serde_json::Value) -> String,
) -> Vec<Arranged> {
    let mut result: Vec<Arranged> = Vec::new();
    let count = chapters.len();
    for index in 0..count {
        let chapter = chapters[index].clone();
        if (chapter.was_cut || chapter.categories.is_some())
            && float(sub(chapter.end, chapter.start)) < TINY_CHAPTER_DURATION
        {
            match result.last_mut() {
                // Prepend to the next chapter if possible.
                None if index < count - 1 => {
                    chapters[index + 1].start = chapter.start;
                    continue;
                }
                None => {}
                Some(previous) => {
                    if index < count - 1 {
                        let previous_is_sponsor = previous.sponsor.is_some();
                        let next_is_sponsor = chapters[index + 1].categories.is_some();
                        // Prefer normals next to normals, sponsors to sponsors.
                        if (chapter.categories.is_none() && previous_is_sponsor && !next_is_sponsor)
                            || (chapter.categories.is_some()
                                && !previous_is_sponsor
                                && next_is_sponsor)
                        {
                            chapters[index + 1].start = chapter.start;
                            continue;
                        }
                    }
                    previous.end = chapter.end;
                    continue;
                }
            }
        }
        let mut arranged = Arranged {
            start: chapter.start,
            end: chapter.end,
            title: chapter.title,
            action: chapter.action,
            sponsor: None,
        };
        if let Some(categories) = chapter
            .categories
            .filter(|categories| !categories.is_empty())
        {
            let shortest = categories
                .iter()
                .reduce(|best, category| {
                    let length = |c: &Category| float(sub(c.2, c.1));
                    if length(category) < length(best) {
                        category
                    } else {
                        best
                    }
                })
                .expect("nonempty categories");
            let sponsor = Sponsor {
                category: shortest.0.clone(),
                categories: ordered(categories.iter().map(|c| c.0.clone())),
                name: shortest.3.clone(),
                category_names: ordered(categories.iter().map(|c| c.3.clone())),
            };
            let dict = serde_json::json!({
                "start_time": arranged.start,
                "end_time": arranged.end,
                "title": arranged.title,
            });
            arranged.title = Some(title(&sponsor, &dict));
            // Merge identically named sponsors.
            if let Some(previous) = result.last_mut()
                && previous.sponsor.is_some()
                && previous.title == arranged.title
            {
                previous.end = arranged.end;
                continue;
            }
            arranged.sponsor = Some(sponsor);
        }
        result.push(arranged);
    }
    result
}

/// Upstream's default `--sponsorblock-chapter-title`,
/// `[SponsorBlock]: %(category_names)l`.
pub(crate) fn default_sponsor_title(sponsor: &Sponsor) -> String {
    format!("[SponsorBlock]: {}", sponsor.category_names.join(", "))
}

/// One concat demuxer entry's `inpoint`/`outpoint` (`%.6f`).
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ConcatOptions {
    pub inpoint: Option<String>,
    pub outpoint: Option<String>,
}

/// `_make_concat_opts`.
pub(crate) fn concat_options(cuts: &[Segment], duration: f64) -> Vec<ConcatOptions> {
    let mut options = vec![ConcatOptions::default()];
    for cut in cuts {
        let last = options.last_mut().expect("at least one entry");
        // No zero-duration chunk at the beginning.
        if float(cut.start) == 0.0 {
            last.inpoint = Some(format!("{:.6}", float(cut.end)));
            continue;
        }
        last.outpoint = Some(format!("{:.6}", float(cut.start)));
        // Nor at the end.
        if float(cut.end) < duration {
            options.push(ConcatOptions {
                inpoint: Some(format!("{:.6}", float(cut.end))),
                outpoint: None,
            });
        }
    }
    options
}

/// `FFmpegPostProcessor._quote_for_ffmpeg`.
pub(crate) fn quote_for_ffmpeg(text: &str) -> String {
    let mut text = text.replace('\'', "'\\''").replace("'''", "'");
    text = match text.strip_prefix('\'') {
        Some(rest) => rest.to_owned(),
        None => format!("'{text}"),
    };
    match text.strip_suffix('\'') {
        Some(rest) => rest.to_owned(),
        None => format!("{text}'"),
    }
}

/// `_concat_spec` for one file repeated with each entry's options.
pub(crate) fn concat_spec(file: &str, options: &[ConcatOptions]) -> String {
    let mut spec = String::from("ffconcat version 1.0\n");
    for entry in options {
        spec.push_str(&format!("file {}\n", quote_for_ffmpeg(file)));
        if let Some(inpoint) = &entry.inpoint {
            spec.push_str(&format!("inpoint {inpoint}\n"));
        }
        if let Some(outpoint) = &entry.outpoint {
            spec.push_str(&format!("outpoint {outpoint}\n"));
        }
    }
    spec
}

const DEADLINE: std::time::Duration = std::time::Duration::from_secs(600);

fn ffmpeg_error(message: impl Into<String>) -> crate::Error {
    crate::Error::Postprocess(message.into())
}

/// Run a tool with `merge::execute` from `output`'s directory.
fn run(executable: &Path, args: &[OsString], directory: &Path) -> crate::Result<Vec<u8>> {
    crate::merge::execute(
        &executable.canonicalize()?,
        args,
        &directory.canonicalize()?,
        std::time::Instant::now(),
        DEADLINE,
        &mut || std::ops::ControlFlow::Continue(()),
    )
}

/// `_get_real_video_duration`: ffprobe's `format.duration`.
pub(crate) fn probe_duration(ffprobe: &Path, input: &Path, container: &str) -> crate::Result<f64> {
    let directory = input.parent().unwrap_or(Path::new("."));
    let args: Vec<OsString> = [
        "-hide_banner",
        "-v",
        "error",
        "-show_format",
        "-print_format",
        "json",
        "-protocol_whitelist",
        "file",
        "-f",
        crate::video::ffmpeg::demuxer(container)?,
        "-i",
    ]
    .into_iter()
    .map(OsString::from)
    .chain([input.canonicalize()?.into_os_string()])
    .collect();
    let output = run(ffprobe, &args, directory)?;
    serde_json::from_slice::<serde_json::Value>(&output)
        .ok()
        .and_then(|probe| probe["format"]["duration"].as_str()?.parse::<f64>().ok())
        .filter(|duration| *duration != 0.0)
        .ok_or_else(|| {
            ffmpeg_error("Unable to determine video duration: ffprobe returned empty duration")
        })
}

/// `stream_copy_opts(copy, ext=ext)`.
fn stream_copy(copy: bool, ext: &str) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-map", "0", "-dn", "-ignore_unknown"]
        .map(OsString::from)
        .into();
    if copy {
        args.extend(["-c", "copy"].map(OsString::from));
    }
    if matches!(ext, "mp4" | "mov" | "m4a") {
        args.extend(["-c:s", "mov_text"].map(OsString::from));
    }
    args
}

/// `force_keyframes`: re-encode `input` with keyframes at `timestamps`.
pub(crate) fn force_keyframes(
    ffmpeg: &Path,
    pp: &str,
    input: &Path,
    container: &str,
    ext: &str,
    output: &Path,
    timestamps: &[PyNumber],
) -> crate::Result<()> {
    let mut times: Vec<f64> = Vec::new();
    for time in timestamps.iter().map(|time| float(*time)) {
        if !times.contains(&time) {
            times.push(time);
        }
    }
    if times.first() == Some(&0.0) {
        times.remove(0);
    }
    let directory = output.parent().unwrap_or(Path::new("."));
    let mut args: Vec<OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "repeat+info",
        "-y",
        "-f",
    ]
    .map(OsString::from)
    .into();
    args.push(crate::video::ffmpeg::demuxer(container)?.into());
    args.extend(["-protocol_whitelist", "file"].map(OsString::from));
    args.extend(crate::pp_args::input(pp, 1));
    args.push("-i".into());
    args.push(input.canonicalize()?.into_os_string());
    args.extend(stream_copy(false, ext));
    args.push("-force_key_frames".into());
    args.push(
        times
            .iter()
            .map(|time| format!("{time:.6}"))
            .collect::<Vec<_>>()
            .join(",")
            .into(),
    );
    args.extend(["-movflags", "+faststart", "-f"].map(OsString::from));
    args.push(crate::embed_metadata::muxer(ext).into());
    args.extend(crate::pp_args::output(pp));
    args.push(std::path::absolute(output)?.into_os_string());
    run(ffmpeg, &args, directory)?;
    Ok(())
}

/// `remove_chapters`' `concat_files`: `input` repeated with `options`
/// through the concat demuxer, stream-copied to `output` (whose extension
/// picks the subtitle muxer when `muxer` is `None`).
pub(crate) fn concat(
    ffmpeg: &Path,
    pp: &str,
    input: &Path,
    ext: &str,
    muxer: Option<&str>,
    output: &Path,
    options: &[ConcatOptions],
) -> crate::Result<()> {
    let directory = output.parent().unwrap_or(Path::new("."));
    let spec = std::path::absolute(output)?.with_extension(format!("{ext}.concat"));
    let file = format!("file:{}", input.canonicalize()?.display());
    std::fs::write(&spec, concat_spec(&file, options))?;
    let mut args: Vec<OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "repeat+info",
        "-y",
        "-f",
        "concat",
        "-safe",
        "0",
        "-protocol_whitelist",
        "file",
    ]
    .map(OsString::from)
    .into();
    args.extend(crate::pp_args::input(pp, 1));
    args.push("-i".into());
    args.push(spec.clone().into_os_string());
    args.extend(stream_copy(true, ext));
    args.extend(["-movflags", "+faststart"].map(OsString::from));
    if let Some(muxer) = muxer {
        args.extend(["-f".into(), muxer.into()]);
    }
    args.extend(crate::pp_args::output(pp));
    args.push(std::path::absolute(output)?.into_os_string());
    let result = run(ffmpeg, &args, directory);
    // Upstream deletes the spec silently.
    let _ = std::fs::remove_file(&spec);
    result?;
    if std::fs::metadata(output).map_or(true, |metadata| metadata.len() == 0) {
        return Err(ffmpeg_error(
            "FFmpeg produced no output with the chapters removed",
        ));
    }
    Ok(())
}

/// `FFmpegConcatPP._get_codecs`: the streams' `codec_name`s.
fn codecs(ffprobe: &Path, input: &Path) -> crate::Result<Vec<String>> {
    let directory = input.parent().unwrap_or(Path::new("."));
    let args: Vec<OsString> = [
        "-hide_banner",
        "-v",
        "error",
        "-show_streams",
        "-print_format",
        "json",
        "-protocol_whitelist",
        "file",
        "-i",
    ]
    .into_iter()
    .map(OsString::from)
    .chain([input.canonicalize()?.into_os_string()])
    .collect();
    let output = run(ffprobe, &args, directory)?;
    let probe: serde_json::Value =
        serde_json::from_slice(&output).map_err(|_| ffmpeg_error("invalid ffprobe JSON"))?;
    let names: Vec<String> = probe["streams"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|stream| stream["codec_name"].as_str().map(str::to_owned))
        .collect();
    crate::screen::debug(&format!("Codecs = {}", names.join(", ")));
    Ok(names)
}

/// `FFmpegConcatPP.concat_files` for several files: identical streams are
/// required, then the concat demuxer stream-copies them into `output`.
pub(crate) fn concat_files(
    ffmpeg: &Path,
    ffprobe: &Path,
    inputs: &[std::path::PathBuf],
    output: &Path,
) -> crate::Result<()> {
    let mut seen: Vec<Vec<String>> = Vec::new();
    for input in inputs {
        let names = codecs(ffprobe, input)?;
        if !seen.contains(&names) {
            seen.push(names);
        }
    }
    if seen.len() > 1 {
        return Err(ffmpeg_error(
            "The files have different streams/codecs and cannot be concatenated. Either select different formats or --recode-video them to a common format",
        ));
    }
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let workspace = tempfile::Builder::new()
        .prefix(".yt-dlp-rs-concat-")
        .tempdir_in(parent)?;
    let ext = output
        .extension()
        .map(|ext| ext.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let spec = workspace.path().join("files.concat");
    let mut text = String::from("ffconcat version 1.0\n");
    for input in inputs {
        let file = format!("file:{}", input.canonicalize()?.display());
        text.push_str(&format!("file {}\n", quote_for_ffmpeg(&file)));
    }
    std::fs::write(&spec, text)?;
    let temporary = workspace.path().join(format!("concat.{ext}"));
    let mut args: Vec<OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "repeat+info",
        "-y",
        "-f",
        "concat",
        "-safe",
        "0",
        "-protocol_whitelist",
        "file",
    ]
    .map(OsString::from)
    .into();
    args.extend(crate::pp_args::input("Concat", 1));
    args.push("-i".into());
    args.push(spec.into_os_string());
    args.extend(stream_copy(true, &ext));
    args.extend(["-movflags", "+faststart", "-f"].map(OsString::from));
    args.push(crate::embed_metadata::muxer(&ext).into());
    args.extend(crate::pp_args::output("Concat"));
    args.push(temporary.clone().into_os_string());
    run(ffmpeg, &args, workspace.path())?;
    if std::fs::metadata(&temporary).map_or(true, |metadata| metadata.len() == 0) {
        return Err(ffmpeg_error("FFmpeg produced no concatenated output"));
    }
    std::fs::rename(&temporary, output)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn number(value: &Value) -> PyNumber {
        serde_json::from_value(value.clone()).unwrap()
    }

    fn segment(value: &Value) -> Segment {
        Segment {
            start: number(&value["start_time"]),
            end: number(&value["end_time"]),
            title: value["title"].as_str().map(str::to_owned),
            action: None,
            remove: value["remove"].as_bool().unwrap_or(false),
            categories: value["_categories"].as_array().map(|categories| {
                categories
                    .iter()
                    .map(|category| {
                        (
                            category[0].as_str().unwrap().to_owned(),
                            number(&category[1]),
                            number(&category[2]),
                            category[3].as_str().unwrap().to_owned(),
                        )
                    })
                    .collect()
            }),
            was_cut: false,
            cut_idx: None,
        }
    }

    fn fixture() -> Value {
        serde_json::from_str(include_str!(
            "../tests/fixtures/modify_chapters/upstream.json"
        ))
        .unwrap()
    }

    /// Upstream's own `TestModifyChaptersPP` inputs and results.
    #[test]
    fn arrangement_matches_upstream_test_cases() {
        let fixture = fixture();
        let cases = fixture["arrange"].as_array().unwrap();
        assert_eq!(cases.len(), 45);
        for case in cases {
            let chapters = case["chapters"]
                .as_array()
                .unwrap()
                .iter()
                .map(segment)
                .collect();
            let title = |sponsor: &Sponsor| match case["title"].as_str().unwrap() {
                "[SponsorBlock]: %(name)s" => format!("[SponsorBlock]: {}", sponsor.name),
                "[SponsorBlock]: %(category_names)l" => default_sponsor_title(sponsor),
                other => panic!("unexpected title template {other}"),
            };
            let (arranged, cuts) = arrange(chapters, &title);
            let actual: Vec<Value> = arranged
                .iter()
                .map(|chapter| serde_json::json!([chapter.start, chapter.end, chapter.title]))
                .collect();
            assert_eq!(Value::from(actual), case["expected"], "{case}");
            let cuts: Vec<Value> = cuts
                .iter()
                .map(|cut| serde_json::json!([cut.start, cut.end]))
                .collect();
            assert_eq!(Value::from(cuts), case["cuts"], "{case}");
        }
    }

    #[test]
    fn concat_options_and_quoting_match_upstream() {
        let fixture = fixture();
        for case in fixture["concat"].as_array().unwrap() {
            let cuts: Vec<Segment> = case["cuts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|cut| {
                    let mut segment = Segment::chapter(number(&cut[0]), number(&cut[1]), None);
                    segment.remove = true;
                    segment
                })
                .collect();
            let actual: Vec<Value> = concat_options(&cuts, case["duration"].as_f64().unwrap())
                .into_iter()
                .map(|entry| {
                    let mut map = serde_json::Map::new();
                    if let Some(inpoint) = entry.inpoint {
                        map.insert("inpoint".into(), inpoint.into());
                    }
                    if let Some(outpoint) = entry.outpoint {
                        map.insert("outpoint".into(), outpoint.into());
                    }
                    Value::Object(map)
                })
                .collect();
            assert_eq!(Value::from(actual), case["expected"]);
        }
        for case in fixture["quote"].as_array().unwrap() {
            assert_eq!(
                quote_for_ffmpeg(case[0].as_str().unwrap()),
                case[1].as_str().unwrap()
            );
        }
    }
}

//! Upstream `F4mFD` (`yt_dlp/downloader/f4m.py`): an HDS stream from its
//! F4M manifest and bootstrap info, written as FLV with each fragment's
//! `mdat` payload. Live streams (which re-read the bootstrap) are not
//! supported.
use crate::{DownloadOptions, DownloadProgress, Error, Fragment, Result, VideoInfo};
use base64::Engine;
use reqwest::blocking::Client;
use std::{io::Read, ops::ControlFlow, path::Path};
use url::Url;

const NS: &str = "http://ns.adobe.com/f4m/1.0";

fn reported(message: impl Into<String>) -> Error {
    Error::Reported(message.into())
}

/// `FlvReader`: big-endian reads that fail like `DataTruncatedError`.
struct FlvReader<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> FlvReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }

    fn read_bytes(&mut self, count: usize) -> Result<&'a [u8]> {
        let available = self.data.len() - self.position;
        if available < count {
            return Err(reported(format!(
                "FlvReader error: need {count} bytes while only {available} bytes got"
            )));
        }
        let bytes = &self.data[self.position..self.position + count];
        self.position += count;
        Ok(bytes)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.read_bytes(1)?[0])
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(
            self.read_bytes(4)?.try_into().expect("4 bytes"),
        ))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(
            self.read_bytes(8)?.try_into().expect("8 bytes"),
        ))
    }

    fn string(&mut self) -> Result<()> {
        while self.read_bytes(1)? != b"\0" {}
        Ok(())
    }

    /// `read_box_info`: `(type, data)`.
    fn box_info(&mut self) -> Result<(&'a [u8], &'a [u8])> {
        let size = u64::from(self.u32()?);
        let kind = self.read_bytes(4)?;
        let (size, header) = if size == 1 {
            (self.u64()?, 16)
        } else {
            (size, 8)
        };
        // Python's `read(negative)` reads the rest.
        let length = match size.checked_sub(header) {
            Some(length) => usize::try_from(length).unwrap_or(usize::MAX),
            None => self.data.len() - self.position,
        };
        Ok((kind, self.read_bytes(length)?))
    }

    fn asrt(&mut self) -> Result<Vec<(u32, u32)>> {
        self.u8()?;
        self.read_bytes(3)?;
        for _ in 0..self.u8()? {
            self.string()?;
        }
        let count = self.u32()?;
        let mut runs = Vec::new();
        for _ in 0..count {
            runs.push((self.u32()?, self.u32()?));
        }
        Ok(runs)
    }

    /// `read_afrt`: the fragment runs' `first` numbers.
    fn afrt(&mut self) -> Result<Vec<u32>> {
        self.u8()?;
        self.read_bytes(3)?;
        self.u32()?;
        for _ in 0..self.u8()? {
            self.string()?;
        }
        let count = self.u32()?;
        let mut firsts = Vec::new();
        for _ in 0..count {
            firsts.push(self.u32()?);
            self.u64()?;
            if self.u32()? == 0 {
                self.u8()?;
            }
        }
        Ok(firsts)
    }
}

/// `read_bootstrap_info`'s segment runs, fragment-run firsts and `live`.
struct Bootstrap {
    segments: Vec<Vec<(u32, u32)>>,
    fragments: Vec<Vec<u32>>,
    live: bool,
}

fn bootstrap(data: &[u8]) -> Result<Bootstrap> {
    let assertion = || reported("AssertionError");
    let (kind, abst) = FlvReader::new(data).box_info()?;
    if kind != b"abst" {
        return Err(assertion());
    }
    let mut reader = FlvReader::new(abst);
    reader.u8()?;
    reader.read_bytes(3)?;
    reader.u32()?;
    let live = reader.u8()? & 0x20 != 0;
    reader.u32()?;
    reader.u64()?;
    reader.u64()?;
    reader.string()?;
    for _ in 0..reader.u8()? {
        reader.string()?;
    }
    for _ in 0..reader.u8()? {
        reader.string()?;
    }
    reader.string()?;
    reader.string()?;
    let mut segments = Vec::new();
    for _ in 0..reader.u8()? {
        let (kind, data) = reader.box_info()?;
        if kind != b"asrt" {
            return Err(assertion());
        }
        segments.push(FlvReader::new(data).asrt()?);
    }
    let mut fragments = Vec::new();
    for _ in 0..reader.u8()? {
        let (kind, data) = reader.box_info()?;
        if kind != b"afrt" {
            return Err(assertion());
        }
        fragments.push(FlvReader::new(data).afrt()?);
    }
    Ok(Bootstrap {
        segments,
        fragments,
        live,
    })
}

/// `build_fragments_list`: `(segment, fragment)` numbers.
fn fragments_list(boot: &Bootstrap) -> Result<Vec<(u32, u64)>> {
    let index_error = || reported("list index out of range");
    let runs = boot.segments.first().ok_or_else(index_error)?;
    let first = *boot
        .fragments
        .first()
        .and_then(|runs| runs.first())
        .ok_or_else(index_error)?;
    let mut next = u64::from(first);
    let mut list = Vec::new();
    for &(segment, count) in runs {
        if list.len() as u64 + u64::from(count) > 100_000 {
            return Err(Error::UnsupportedManifest(
                "HDS fragment count must be at most 100000".into(),
            ));
        }
        for _ in 0..count {
            list.push((segment, next));
            next += 1;
        }
    }
    Ok(list)
}

/// The FLV header and the optional metadata script tag.
fn flv_header(metadata: Option<&[u8]>) -> Vec<u8> {
    let mut out = b"FLV\x01\x05\x00\x00\x00\x09\x00\x00\x00\x00".to_vec();
    if let Some(metadata) = metadata.filter(|metadata| !metadata.is_empty()) {
        out.push(0x12);
        out.extend_from_slice(&(metadata.len() as u32).to_be_bytes()[1..]);
        out.extend_from_slice(&[0; 7]);
        out.extend_from_slice(metadata);
        out.extend_from_slice(&(11 + metadata.len() as u32).to_be_bytes());
    }
    out
}

/// A fragment's `mdat` box data (the first one).
pub(crate) fn mdat(data: &[u8]) -> Result<&[u8]> {
    let mut reader = FlvReader::new(data);
    loop {
        let (kind, payload) = reader.box_info()?;
        if kind == b"mdat" {
            return Ok(payload);
        }
    }
}

fn get(client: &Client, url: &Url, info: &VideoInfo) -> Result<(Url, Vec<u8>)> {
    // The fragment requests' headers (the page as Referer).
    let mut request = client.get(url.clone());
    if !info.is_direct() {
        request = request.header(reqwest::header::REFERER, info.webpage_url.as_str());
    }
    let mut response = request
        .send()
        .inspect(|_| crate::screen::after_request())
        .map_err(Error::Http)
        .and_then(crate::retry::check_response)?;
    let final_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    Ok((final_url, body))
}

fn text(node: roxmltree::Node<'_, '_>) -> String {
    node.children()
        .take_while(|child| !child.is_element())
        .filter_map(|child| child.text())
        .collect()
}

fn child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    name: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children().find(|child| {
        child.is_element()
            && child.tag_name().name() == name
            && child.tag_name().namespace() == Some(NS)
    })
}

/// `base64.b64decode` (non-alphabet characters are discarded).
fn b64decode(text: &str) -> Result<Vec<u8>> {
    let cleaned: String = text
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
        .collect();
    base64::engine::general_purpose::STANDARD
        .decode(cleaned)
        .map_err(|_| reported("Incorrect padding"))
}

/// `F4mFD.real_download`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn download(
    client: &Client,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    keep_fragments: bool,
    test: bool,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    crate::screen::screen("[f4m] Downloading f4m manifest");
    let (manifest_url, body) = get(client, &info.url, info)?;
    let manifest = crate::extractor::fix_xml_ampersands(&String::from_utf8_lossy(&body));
    let document = roxmltree::Document::parse_with_options(
        manifest.trim(),
        roxmltree::ParsingOptions {
            allow_dtd: true,
            ..Default::default()
        },
    )
    .map_err(|error| reported(format!("Failed to parse f4m manifest: {error}")))?;
    let root = document.root_element();
    // `_get_unencrypted_media`.
    let elements = |name: &str| {
        root.children()
            .filter(|node| {
                node.is_element()
                    && node.tag_name().name() == name
                    && node.tag_name().namespace() == Some(NS)
            })
            .collect::<Vec<_>>()
    };
    let mut media = elements("media");
    if media.is_empty() {
        return Err(reported("No media found"));
    }
    for header in elements("drmAdditionalHeader")
        .into_iter()
        .chain(elements("drmAdditionalHeaderSet"))
    {
        if header.attribute("id").is_none() {
            return Err(reported("Missing ID in f4m DRM"));
        }
    }
    media.retain(|node| {
        node.attribute("drmAdditionalHeaderId").is_none()
            && node.attribute("drmAdditionalHeaderSetId").is_none()
    });
    if media.is_empty() {
        return Err(reported("Unsupported DRM"));
    }
    let bitrate = |node: &roxmltree::Node<'_, '_>| -> Result<i64> {
        match node.attribute("bitrate") {
            None => Ok(-1),
            Some(value) => value.trim().parse().map_err(|_| {
                reported(format!(
                    "invalid literal for int() with base 10: {}",
                    crate::batch::py_repr(value)
                ))
            }),
        }
    };
    let mut ranked = Vec::new();
    for node in &media {
        ranked.push((bitrate(node)?, *node));
    }
    let requested = info.tbr;
    let chosen = if requested.is_none() || ranked.len() == 1 {
        // `sorted(...)[-1]`: the last of the highest bitrates.
        ranked.sort_by_key(|(bitrate, _)| *bitrate);
        ranked.last().map(|(_, node)| *node)
    } else {
        ranked
            .iter()
            .find(|(bitrate, _)| Some(*bitrate as f64) == requested)
            .map(|(_, node)| *node)
    }
    .ok_or_else(|| reported("StopIteration"))?;
    let base = child(root, "baseURL")
        .map(|node| text(node).trim().to_owned())
        .filter(|base| !base.is_empty())
        .map(|base| Url::parse(&base))
        .transpose()?
        .unwrap_or_else(|| manifest_url.clone());
    let media_url = chosen
        .attribute("url")
        .ok_or_else(|| reported("KeyError: 'url'"))?;
    let base_url = base.join(media_url)?;
    let node = child(root, "bootstrapInfo").ok_or_else(|| reported("No bootstrap info found"))?;
    let boot = match node.attribute("url").filter(|url| !url.is_empty()) {
        Some(url) => bootstrap(&get(client, &base.join(url)?, info)?.1)?,
        None => bootstrap(&b64decode(&text(node))?)?,
    };
    if boot.live {
        return Err(Error::UnsupportedManifest(
            "live HDS streams are not supported natively".into(),
        ));
    }
    let metadata = child(chosen, "metadata")
        .map(|node| b64decode(&text(node)))
        .transpose()?;
    let mut list = fragments_list(&boot)?;
    if test {
        list.truncate(1);
    }
    let akamai_pv = child(root, "pv-2.0").map(text).filter(|pv| !pv.is_empty());
    let mut fragments = Vec::new();
    for (segment, fragment) in list {
        let mut url = base_url.clone();
        url.set_path(&format!("{}Seg{segment}-Frag{fragment}", base_url.path()));
        let mut query = Vec::new();
        if let Some(existing) = base_url.query().filter(|query| !query.is_empty()) {
            query.push(existing.to_owned());
        }
        if let Some(pv) = &akamai_pv {
            query.push(pv.trim_matches(';').to_owned());
        }
        if let Some(extra) = info
            .extra
            .get("extra_param_to_segment_url")
            .and_then(serde_json::Value::as_str)
            .filter(|extra| !extra.is_empty())
        {
            query.push(extra.to_owned());
        }
        url.set_query((!query.is_empty()).then(|| query.join("&")).as_deref());
        fragments.push(Fragment {
            encryption: None,
            url,
            byte_range: None,
            path: None,
            duration: None,
        });
    }
    // `_prepare_frag_download`.
    crate::screen::screen(&format!("[f4m] Total fragments: {}", fragments.len()));
    let mut plan = info.clone();
    plan.fragments = fragments;
    plan.protocol = Some("f4m".into());
    crate::fragment::download_f4m(
        client,
        &plan,
        destination,
        options,
        keep_fragments,
        &flv_header(metadata.as_deref()),
        callback,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxes_and_headers_follow_upstream() {
        let mut fragment = Vec::new();
        fragment.extend_from_slice(&12u32.to_be_bytes());
        fragment.extend_from_slice(b"afra1234");
        fragment.extend_from_slice(&11u32.to_be_bytes());
        fragment.extend_from_slice(b"mdatabc");
        assert_eq!(mdat(&fragment).unwrap(), b"abc");
        assert_eq!(
            mdat(b"\0\0\0").unwrap_err().to_string(),
            "FlvReader error: need 4 bytes while only 3 bytes got"
        );
        assert_eq!(
            flv_header(None),
            b"FLV\x01\x05\x00\x00\x00\x09\x00\x00\x00\x00"
        );
        assert_eq!(
            flv_header(Some(b"m")),
            b"FLV\x01\x05\x00\x00\x00\x09\x00\x00\x00\x00\x12\x00\x00\x01\x00\x00\x00\x00\x00\x00\x00m\x00\x00\x00\x0c"
        );
    }
}

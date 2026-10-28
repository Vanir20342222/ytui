//! Compatibility adapter for the audio-only public API.
use crate::{
    AudioDownload, AudioOptions, DownloadOptions, DownloadProgress, PostprocessOptions, Result,
    VideoInfo, YoutubeDl,
};
use std::{ops::ControlFlow, path::Path};

pub(crate) fn download(
    downloader: &YoutubeDl,
    info: &VideoInfo,
    destination: &Path,
    options: DownloadOptions,
    audio: &AudioOptions,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<AudioDownload> {
    let result = crate::postprocess::download(
        downloader,
        info,
        destination,
        options,
        &PostprocessOptions {
            existing: None,
            metadata_parser: None,
            no_post_overwrites: false,
            modify_chapters: None,
            ignore_errors: false,
            thumbnail: None,
            metadata: false,
            chapters: false,
            infojson: None,
            fixup: None,
            audio: Some(audio.clone()),
            video: None,
            embed: None,
            keep_video: audio.keep_video,
        },
        callback,
    )?;
    Ok(AudioDownload {
        path: result.path,
        bytes: result.bytes,
        kept_video: result.kept_video,
    })
}

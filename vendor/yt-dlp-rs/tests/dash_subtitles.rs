//! DASH text representations as subtitles: a direct BaseURL is downloaded
//! as it is, a segmented track through DashSegmentsFD (fragments appended).
//! Outputs were checked against pinned upstream with the same manifest.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    thread,
};

const MPD: &str = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT4S" minBufferTime="PT1S" profiles="urn:mpeg:dash:profile:isoff-on-demand:2011">
<Period><AdaptationSet mimeType="video/mp4" contentType="video"><Representation id="v1" bandwidth="1000" width="640" height="360" codecs="avc1.4d401e"><SegmentList duration="2" timescale="1"><Initialization sourceURL="init.mp4"/><SegmentURL media="v1.m4s"/></SegmentList></Representation></AdaptationSet>
<AdaptationSet contentType="text" mimeType="text/vtt" lang="en"><Representation id="en" bandwidth="1"><BaseURL>en.vtt</BaseURL></Representation></AdaptationSet>
<AdaptationSet contentType="text" mimeType="text/vtt" lang="fr"><Representation id="fr" bandwidth="1"><SegmentList duration="2" timescale="1"><SegmentURL media="s1.vtt"/><SegmentURL media="s2.vtt"/></SegmentList></Representation></AdaptationSet></Period></MPD>"#;
const EN: &str = "WEBVTT\n\n00:00:00.500 --> 00:00:01.500\nDash cue\n";
const S1: &str = "WEBVTT\n\n00:00:00.500 --> 00:00:01.500\nOne\n";
const S2: &str = "WEBVTT\n\n00:00:02.500 --> 00:00:03.500\nTwo\n";

fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (mime, body): (&str, &[u8]) = match path.as_str() {
                "/v.mpd" => ("application/dash+xml", MPD.as_bytes()),
                "/en.vtt" => ("text/vtt", EN.as_bytes()),
                "/s1.vtt" => ("text/vtt", S1.as_bytes()),
                "/s2.vtt" => ("text/vtt", S2.as_bytes()),
                _ => ("video/mp4", b"segment"),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    origin
}

#[test]
fn text_tracks_are_listed_and_downloaded() {
    let origin = serve();
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--no-progress", "-o", "o.%(ext)s"])
        .args(["--write-subs", "--sub-langs", "all", "--skip-download"])
        .arg(format!("{origin}/v.mpd"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains(
        "[info] Writing video subtitles to: o.fr.vtt\n[dashsegments] Total fragments: 2\n\
         [download] Destination: o.fr.vtt\n[download] Download completed\n"
    ));
    assert_eq!(
        std::fs::read_to_string(directory.path().join("o.en.vtt")).unwrap(),
        EN
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("o.fr.vtt")).unwrap(),
        format!("{S1}{S2}")
    );
}

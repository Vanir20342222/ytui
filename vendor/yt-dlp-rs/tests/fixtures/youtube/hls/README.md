# Saved synthetic YouTube HLS discovery fixtures

These files describe deterministic localhost-only responses, not captured live
YouTube availability. Both player responses identify `BaW_jenozKc` and contain no
direct media formats. The HLS master supplies two muxed alternatives; each media
playlist has two plain TS segment references. Itag path values are synthetic
identity markers for this comparison, not asserted live format availability.

`dashManifestUrl` points to a forbidden route. Pinned upstream WEB policy permits
HLS without a GVS token but requires one for DASH; the offline comparison must
parse HLS and skip DASH with the token provider returning `None`, without
`missing_pot` overrides.

`player-response-n.json` uses the independently saved EJS input/output vector
`IlLiA21ny7gqA2m4p37` → `9nRTxrbM1f0yHg` from the sibling challenges fixtures.
The comparison supplies a typed fixed solver response through actual upstream
challenge plumbing, asserting the rewritten manifest path and preserved
`/file/index.m3u8` suffix. It does not execute EJS or claim solver acquisition
parity.

Reference: official yt-dlp commit
`897341241f2bfb2439fe0367f4c7c9ccd29a3828`, particularly
`yt_dlp/extractor/youtube/_base.py` WEB protocol token policies and
`yt_dlp/extractor/youtube/_video.py` manifest discovery/path-n handling.

`expected.json` is the shared normalized metadata contract consumed by Rust
fixture tests and `tools/compare_youtube_hls.py`. Comparison preserves ordered
formats, URLs, dimensions, bandwidth, stream kind and language, mapping upstream
itag IDs 95/96 to declaration IDs `hls-0`/`hls-1`. Only the exact localhost origin
is normalized. Container/extension, fragment plans, transport, selection and
ranking are excluded: native TS concatenation and upstream's MP4 label/remuxing
have different semantics.

# Upstream projects

The application is MIT licensed. Dependencies retain their own licenses; see
Cargo.lock and each dependency's packaged license. No external executables are
bundled by install.sh or its install-rust.sh compatibility alias.

- aagedal/yt-dlp-rs: native extraction/transfer library, Unlicense, pinned to
  10223c019ef6ec5552627b90251d8e967404e440. Its bundled EJS assets have separate
  notices at https://github.com/aagedal/yt-dlp-rs/tree/10223c019ef6ec5552627b90251d8e967404e440/vendor/ejs.
- rookie 0.5.6: MIT. Its crate manifest omits a license field; the distributed
  MIT-LICENSE.txt was inspected. Copyright (c) 2023–2024 thewh1teagle and others.
- Ratatui, ratatui-image and Crossterm: MIT.
- Native development-only capture/testing: portable-pty 0.9.0 and vt100 0.15.2
  are MIT; fontdue 0.9.5 is MIT OR Apache-2.0 OR Zlib. The capture command reads
  an installed monospaced font; font files are not bundled in the repository.
- youtui / ytmapi-rs, ytmusic-tui and youtube-tui: API/UX research references;
  ytui uses its own provider and UI implementation rather than vendoring these apps.
- cliamp: MIT, referenced for service behavior. cliamp is an optional external
  application; ytui's radio tab is native Rust.
- yt-dlp, mpv, FFmpeg and Deno: optional/required system tools according to the
  selected features; not distributed as part of this repository's Rust binary.

Before distributing prebuilt binary releases, generate a complete transitive
license bundle and include the pinned native extractor's EJS notices. Source-only
build instructions and Cargo dependency downloads preserve the original packages.

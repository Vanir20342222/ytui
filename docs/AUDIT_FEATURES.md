# Alpha.12 UI workflows

Press **F2** for Actions; **`,`** opens general settings. All global shortcuts can
be reviewed/remapped in Keyboard bindings. No bottom command line is needed.

- **S / Library → Save or remove selected video/song locally** toggles a local
  bookmark. **Saved locally** in Library or the feed picker opens it; **/** filters
  titles, creators and video IDs. Reviewing works offline; playback needs a
  connection. These entries survive restart/account disconnect.
- **o → Set sleep timer** accepts `30m`, `1h` or `90s`, up to 24 hours. Its countdown
  appears in the player strip. Status/cancel actions share that menu; expiry stops
  playback and queue advance, retaining the queue and keeping ytui open. It resets
  after restarting the app. See [research and verification](FEATURE_RESEARCH_2026-10-24.md).

- **Account → Connect browser** discovers profile directories, then opens a review
  form. Enter in that form explicitly imports/validates the chosen profile.
  Complete YouTube/Music consent or bot verification in the browser first.
- **Downloads → Download full collection as individual jobs** loads the collection
  completely before queuing anything. Enter `all` or available-entry ranges such
  as `1,3:7`. Limits are 32 pages, 2,000 entries and 16 MiB; an incomplete result
  queues nothing. Missing/private entries depend on what the provider reports.
  Each entry has a saved status; **Download groups** opens its detail list.
  **r** retries an entry; **o** lists files; Enter selects its queue row.
- Existing completed files are reused when expanding the same collection again.
  A shared item already in a different group is reported and not downloaded twice.
  The loaded-page action still intentionally downloads only loaded rows.
- **Downloads → Files in selected download / playlist** selects every recorded
  output, including files from a collection that later failed. Enter plays local
  media; **o** requests **Open with** through the desktop portal. Missing files and
  sidecars cannot be played. Finished files never turn the whole failed job green.
- **Download to a recent folder** reopens the download form for review. Twelve
  destinations are saved locally; choosing one never moves existing files.
- **Player → Available formats, audio tracks & captions** has three tabs (Left/Right):
  actual extractor formats, loaded mpv tracks, and available VTT/SRT captions.
  Format changes restart at the current position/pause; loaded tracks switch in
  place. A source/extractor may expose only a subset, and missing choices are
  shown as unavailable. Settings quality remains a maximum height, not proof the
  source offers that resolution. An advanced exact selector is visible in download
  options and takes precedence over the preset quality/format choice.
- **SponsorBlock & ad handling status** explains current mode, categories, fetched
  marks and replay. **B** temporarily toggles skipping; **U** replays a skipped part.
  API failure does not stop playback. This does not promise universal SSAI removal.
- **Settings → Inspect / Clear thumbnail cache** shows RAM/disk use and clears only
  owned cache files. Transient images retry automatically; unavailable images have
  a bounded state. **F5** retries images without defeating a provider cooldown.
- **Setup & tool capabilities** checks installed executables with deadlines.
  **Export safe support report**, or `ytui support --output NEW.json`, writes only
  an allowlisted report and refuses to overwrite an existing file. Version output
  includes the build SHA and `-dirty` when compiled from modified source.
- MPRIS media controls operate the current app/queue. A missing session bus leaves
  terminal controls usable. Actual desktop/Wayland/key/audible acceptance remains
  separate from the private-bus regression.
- **General settings → General → Language** saves English (`en`), German (`de`)
  or system language. System checks LC_ALL, LC_MESSAGES, then LANG at startup;
  other locales use English. Existing configurations keep English. Navigation,
  all settings/action labels and keyboard-help descriptions are translated before
  layout; Actions accepts English and German search. Provider content, technical
  values, diagnostics and some status/guidance prose retain their original text.
  This is an initial German interface, not a claim that every sentence is translated.
- **Player → Find lyrics (LRCLIB)** opens a review form prefilled from the playing
  item (or selection). Edit the song title and artist; Enter sends **only those two
  fields** to LRCLIB through the configured proxy, without browser cookies/history.
  Nothing is looked up automatically. Choose a returned candidate and press Enter.
  Availability/correct matching depends on LRCLIB; an empty/error result leaves
  playback alone.
- **Open local lyrics** works offline with UTF-8 LRC/plain text up to 512 KiB.
  Repeated timestamps and global LRC offsets are supported. Timed lines follow the
  bound playing item; changing tracks pauses highlighting. Up/Down/PgUp/PgDn scroll
  manually, **f/Home** follows, Backspace returns to online candidates, Esc closes
  and cancels a pending lookup. Lyrics are not saved into downloaded media.
- Asynchronous cache/capability/VPN-list panels have request identities: closing a
  panel or opening a newer request prevents an old result from reopening it.

SOCKS5/5h uses remote DNS for Rust browsing/downloads. mpv requires HTTP(S) proxy
or normal OS VPN routing; unsupported SOCKS playback fails before a direct request.
No feature installs tools, accepts consent, uploads cookies or changes a VPN.

The archive/update/rollback instructions are in [RELEASE_CANDIDATE.md](RELEASE_CANDIDATE.md).
Executed checks and remaining gates are in [AUDIT_IMPLEMENTATION.md](AUDIT_IMPLEMENTATION.md).

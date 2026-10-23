# Alpha.12 UI workflows

Press **F2** for Actions; **`,`** opens general settings. All global shortcuts can
be reviewed/remapped in Keyboard bindings. No bottom command line is needed.

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

SOCKS5/5h uses remote DNS for Rust browsing/downloads. mpv requires HTTP(S) proxy
or normal OS VPN routing; unsupported SOCKS playback fails before a direct request.
No feature installs tools, accepts consent, uploads cookies or changes a VPN.

The archive/update/rollback instructions are in [RELEASE_CANDIDATE.md](RELEASE_CANDIDATE.md).
Executed checks and remaining gates are in [AUDIT_IMPLEMENTATION.md](AUDIT_IMPLEMENTATION.md).

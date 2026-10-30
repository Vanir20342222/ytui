# Content-first Android restyle (M9)

The owner supplied `ytui-youtube-restyle-agent-prompt.md` and
`youtube-style-and-feel.md` on 2026-10-30 and requested implementation. Scope is
presentation only: preserve playback, Rust, provider/network/cache/account/data
contracts, navigation actions and all existing options. Keep ytui branding and
shared palettes. No YouTube logos, proprietary fonts or new service features.

## Baseline and decisions

The brief names `427842c`, superseded by mobile.8 playback and the merged PR27 UI.
Work starts at current main `05903aa`, preserving those changes. The screenshot
ZIP named in the brief was not supplied. Equivalent exact-source mobile.9
qualification captures (app source `553fe49`, unchanged app code at `05903aa`)
are retained at `/workspace/ytui-restyle-evidence/before`. This is retained prior
emulator evidence, not a fresh baseline run or a comparison against the missing
ZIP. Pixel 7 API35 suite and 2560x1600 tablet captures are the existing profiles.

The owner explicitly kept the YouTube label. Theme-default follow-up is pending;
keep existing explicit selections. The current mini player exists and will be
restyled. MediaCard exposes creator and existing view/date metadata, but no
channel-avatar field: use a neutral creator initial, without network requests.
The old brief's video −5/+15 illustration differs from current configurable
Media3 controls. Preserve current seek behavior and amounts.

## Work groups

1. Shared spacing, shapes, type, neutral action and contrast roles.
2. P0: app bar/search, full-bleed feed, music covers, unified chip row, navigation.
3. P1: focused player/seek appearance, Media3 control layout, sheets, settings,
   account and dialogs. Preserve content descriptions and test tags.
4. P2: responsive grid and wide search rows; accessibility and screenshots.

Each group receives a commit. New visual assertions verify intended geometry;
existing behavior assertions and strict startup deadlines remain intact. Final
results, screenshot locations and unverified requirements are recorded below.

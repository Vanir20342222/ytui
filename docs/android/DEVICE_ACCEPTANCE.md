# Native Android UI device acceptance (M9)

The first physical Tab S9 preview failed input, theme and account navigation.
Use the native Compose build (`0.2.0-alpha.12-mobile.2` or newer) for re-testing.
Check **⋮ → About** for the version and source revision; use the exact APK linked
from the corresponding successful Android workflow run, not an earlier artifact.

This checklist supplements emulator tests. Do not send cookies, account files,
authorization headers or signed media URLs in a bug report. Keep wanted app data
before changing installations; see [signing/update limitations](SIGNING.md).

On the Samsung Tab S9, then the Nothing Phone (3a):

1. Tap Search, confirm the system keyboard opens, type words including Unicode,
   edit/delete them and submit using the keyboard Search key. Back should hide the
   keyboard before closing a view. Repeat editing in Settings' proxy field
   without applying a test proxy, and in Downloads' URL field.
2. Open **Settings**, select a visibly different theme and close/reopen Settings.
   The current screen should repaint immediately and retain the choice after an
   app restart. Test Android light/dark following with the System choice.
3. Open **Sign in / Account** directly, and from a protected feed's account action.
   Verify the cookie-document picker and native header field are reachable.
   Opening/canceling the sign-in page or rotating it must not import implicitly.
   Import only your own session when you intend to connect. Android cannot read
   another browser's private database; Google may reject embedded sign-in.
4. Browse/search YouTube and Music, open collections/categories, scroll into
   continuation pages and return with Back. A provider failure should preserve
   existing results and avoid continuous retry during a service cooldown.
5. Play a public video on the device's normal network. Confirm visible moving
   video and actual sound, seek, pause/resume, quality changes, captions and audio
   tracks where available. Check a muted preview and return to prior playback.
6. Repeat with Music and Radio. Verify volume persists across station changes,
   station identity remains visible when ICY song titles change, and finite Music
   collections advance. Check queue/Previous/Next and lockscreen/headset controls.
7. Select Vinyl, VU and Orbit under Music/Radio. Animation should stop on pause;
   meters should react to decoded sound. Check reduced motion and backgrounding.
8. Download an individual item and a dedicated playlist URL. Verify every entry,
   pause/retry/remove, offline playback after disconnecting the source network,
   and exporting a completed file. MP3/FLAC transcoding is not offered on Android.
9. Open queue, recently played, lyrics, sleep timer and About. Check lyric editing,
   local LRC import, timing offset and tap-to-seek against the current song.
10. Repeat navigation in portrait, landscape and split screen. Catalog controls,
    player, dialogs, keyboard and system bars must remain reachable without
    obscured fields or clipped touch targets.

Record device/Android version, APK source revision, step and observed behavior.
Emulator fixture success does not establish real YouTube/account access, audible
output, OEM battery behavior or completion of the [remaining parity work](PARITY.md).

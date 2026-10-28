# Android APK signing and updates

Development APKs from the Android workflow use debug identities. Android requires
the same signing identity to update an installed package while retaining its
private account/library/cache. A different key requires a separate application
identity or uninstalling, which removes private state. Never uninstall a wanted
installation just to bypass this check.

The manual **Android signed APK** workflow builds a non-debuggable release APK
with an owner-supplied identity. It runs only from `main`, requires successful
Android, Rust and Secret scan runs for that exact revision, and uploads an APK
and inspection report. It does not publish a GitHub Release or mark M9 complete.
Physical-device/live and capability gates in [PARITY.md](PARITY.md) remain.

## Prepare the identity privately

Generate a release keystore on a trusted local machine, outside the repository,
and retain an encrypted backup of both the file and its passwords. For example,
this command prompts for passwords:

```bash
keytool -genkeypair -keystore /private/path/ytui-release.jks -storetype JKS \
  -alias ytui -keyalg RSA -keysize 3072 -validity 10000
```

Losing this
identity prevents future updates to installations signed with it. Do not use the
disposable key used to test the build, and do not commit a keystore or password.

In the repository's `android-release` GitHub Environment, configure these secrets:

| Secret | Value |
| --- | --- |
| `YTUI_ANDROID_KEYSTORE_BASE64` | The keystore encoded as one base64 line |
| `YTUI_ANDROID_KEY_ALIAS` | Its key alias |
| `YTUI_ANDROID_STORE_PASSWORD` | Keystore password |
| `YTUI_ANDROID_KEY_PASSWORD` | Key password, possibly the same as store password |

Use GitHub's private secret entry or `gh secret set --env android-release` with
standard input. Never paste the values into an issue, chat, command argument or
workflow log. Configure environment protection and restrict repository write
access to the people allowed to sign builds. Signing requires no Google API key,
YouTube account or browser cookies.

## Build and verify

After the selected main revision's checks pass, dispatch **Android signed APK**
from `main` with an increasing `version_code` (1–2100000000) and a version name
such as `0.2.0-alpha.12-mobile.2`. Download its `ytui-android-signed-<code>-<sha>`
artifact. The report includes the APK SHA256, public certificate fingerprints,
source revision, both ABIs, 16 KiB alignment and license checks. Confirm the
certificate matches the previous signed APK before installing an update.

For a local release build, privately set `YTUI_ANDROID_KEYSTORE` to the keystore
path and the three alias/password environment variables above. Set
`YTUI_ANDROID_VERSION_CODE`, `YTUI_ANDROID_VERSION_NAME`, and run:

```bash
YTUI_ANDROID_VARIANT=release ./tools/android/build.sh
./tools/android/inspect-apk.sh apps/android/app/build/outputs/apk/release/app-release.apk
```

No release build falls back to debug signing or silently produces an unsigned
APK. The keystore path is read locally; passwords are not written to Gradle
properties. Rust libraries/notices build before the signing secrets are loaded.
The CI copy exists only in the runner's private temporary directory
and is removed when the signing step ends. Only the APK and public inspection
report are uploaded. About in the app shows its version/code and Rust revision.

The first stable owner signing identity has not been configured or validated on
the requested devices here. A successful build with a disposable test identity
does not establish that release setup or in-place device updates are complete.

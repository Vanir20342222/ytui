#!/usr/bin/env bash
# Build the shared Rust engine and package the native Android UI; installs nothing.
set -euo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
variant="${YTUI_ANDROID_VARIANT:-debug}"
case "$variant" in
    debug) gradle_variant=Debug ;;
    release)
        gradle_variant=Release
        for signing_var in YTUI_ANDROID_KEYSTORE YTUI_ANDROID_KEY_ALIAS YTUI_ANDROID_STORE_PASSWORD YTUI_ANDROID_KEY_PASSWORD; do
            [[ -n "${!signing_var:-}" ]] || { echo "Release build requires $signing_var; see docs/android/SIGNING.md." >&2; exit 1; }
        done
        ;;
    *) echo 'YTUI_ANDROID_VARIANT must be debug or release.' >&2; exit 1 ;;
esac
sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
[[ -n "$sdk" && -d "$sdk" ]] || { echo 'Set ANDROID_HOME to an installed Android SDK.' >&2; exit 1; }
ndk="${ANDROID_NDK_HOME:-$sdk/ndk/27.2.12479018}"
host=linux-x86_64
[[ "$(uname -s)" != Darwin ]] || host=darwin-x86_64
bin="$ndk/toolchains/llvm/prebuilt/$host/bin"
[[ -d "$bin" ]] || { echo 'Install NDK 27.2.12479018 or set ANDROID_NDK_HOME.' >&2; exit 1; }
abis="${YTUI_ANDROID_ABIS:-arm64-v8a x86_64}"
cd "$root"
export YTUI_ANDROID_REVISION="$(git rev-parse HEAD)"
for abi in $abis; do
    case "$abi" in
        arm64-v8a) target=aarch64-linux-android; prefix=aarch64-linux-android ;;
        x86_64) target=x86_64-linux-android; prefix=x86_64-linux-android ;;
        *) echo "Unsupported ABI: $abi" >&2; exit 1 ;;
    esac
    linker_key="CARGO_TARGET_$(echo "$target" | tr 'a-z-' 'A-Z_')_LINKER"
    flags_key="CARGO_TARGET_$(echo "$target" | tr 'a-z-' 'A-Z_')_RUSTFLAGS"
    cc_key="CC_${target//-/_}"; ar_key="AR_${target//-/_}"; cxx_key="CXX_${target//-/_}"
    env "$linker_key=$bin/${prefix}26-clang" "$cc_key=$bin/${prefix}26-clang" "$ar_key=$bin/llvm-ar" "$cxx_key=$bin/${prefix}26-clang++" \
        "$flags_key=-C link-arg=-Wl,-z,max-page-size=16384" \
        cargo build -p ytui-mobile --target "$target" --release --locked -j "${YTUI_BUILD_JOBS:-2}"
    mkdir -p "$root/target/android-jni/$abi"
    "$bin/llvm-strip" --strip-unneeded "$root/target/$target/release/libytui_mobile.so" -o "$root/target/android-jni/$abi/libytui_mobile.so"
done
[[ "${YTUI_ANDROID_RUST_ONLY:-0}" != 1 ]] || exit 0
cargo metadata --locked --format-version 1 > "$root/target/android-dependencies.json"
notices_stage="$(mktemp -d "$root/target/android-notices.XXXXXX")"
cargo run -p ytui-mobile --example mobile_license_bundle --locked -- "$root/target/android-dependencies.json" "$notices_stage/notices"
mkdir -p "$root/target/android-assets/notices"
cp -R -- "$notices_stage/notices/." "$root/target/android-assets/notices/"
cp -- "$root/LICENSE" "$root/target/android-assets/ytui-LICENSE"

"$root/apps/android/gradlew" -p "$root/apps/android" --no-daemon ":app:assemble$gradle_variant" ":app:test${gradle_variant}UnitTest" ":app:lint$gradle_variant"

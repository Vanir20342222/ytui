#!/usr/bin/env bash
# Fail if an APK is missing either requested ABI, 16 KiB ELF alignment or signing.
set -euo pipefail
root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)"
apk="${1:-$root/apps/android/app/build/outputs/apk/debug/app-debug.apk}"
sdk="${ANDROID_HOME:?Set ANDROID_HOME}"
ndk="${ANDROID_NDK_HOME:-$sdk/ndk/27.2.12479018}"
host=linux-x86_64
[[ "$(uname -s)" != Darwin ]] || host=darwin-x86_64
readelf="$ndk/toolchains/llvm/prebuilt/$host/bin/llvm-readelf"
"$sdk/build-tools/35.0.0/apksigner" verify "$apk"
"$sdk/build-tools/35.0.0/zipalign" -c -P 16 -v 4 "$apk" > /dev/null
python3 - "$apk" "$readelf" <<'PY'
import pathlib,subprocess,sys,tempfile,zipfile
with zipfile.ZipFile(sys.argv[1]) as archive, tempfile.TemporaryDirectory() as temp:
    for abi in ['arm64-v8a','x86_64']:
        name=f'lib/{abi}/libytui_mobile.so'
        data=archive.read(name)
        target=pathlib.Path(temp)/f'{abi}.so';target.write_bytes(data)
        headers=subprocess.check_output([sys.argv[2],'-lW',str(target)],text=True)
        loads=[line for line in headers.splitlines() if line.strip().startswith('LOAD ')]
        if not loads or any(int(line.split()[-1],16)<16384 for line in loads):
            raise SystemExit(f'{name}: requires 16 KiB ELF alignment')
        symbols=subprocess.check_output([sys.argv[2],'-Ws',str(target)],text=True)
        for symbol in ['GameActivity_onCreate','Java_dev_ytui_android_NativeBridge_request','Java_dev_ytui_android_NativeBridge_meter']:
            if symbol not in symbols: raise SystemExit(f'Missing {abi} symbol: {symbol}')
    if 'assets/ytui-LICENSE' not in archive.namelist():raise SystemExit('Missing application license')
    if 'assets/notices/inventory.json' not in archive.namelist():raise SystemExit('Missing Rust dependency notices')
print('Verified signatures, ZIP alignment, both Rust ABIs, 16 KiB ELF segments and notices.')
PY
sha256sum "$apk"

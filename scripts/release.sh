#!/usr/bin/env bash
# Prepares a version for publishing in dist/atlas-code-<version>/ (and a .zip):
#   - the APK signed with the release key (not the debug one),
#   - Atlas Bridge for the platform it runs on (on Windows, the .exe),
#   - LICENSE, README and THIRD-PARTY-LICENSES.txt (the licenses of every dependency),
#   - SHA256SUMS.
#
# The key lives OUTSIDE the repository: $ATLAS_RELEASE_DIR (by default ~/.atlas-release) with `release.jks` and
# `keystore-password.txt`. Without it, no update can be published that installs over the previous one:
# keep a copy somewhere safe.
#
# Usage: scripts/release.sh
set -euo pipefail
cd "$(dirname "$0")/.."

KEYDIR="${ATLAS_RELEASE_DIR:-$HOME/.atlas-release}"
if [ ! -f "$KEYDIR/release.jks" ] || [ ! -f "$KEYDIR/keystore-password.txt" ]; then
    echo "the release key is missing in $KEYDIR (release.jks and keystore-password.txt)" >&2
    exit 1
fi
VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
PYTHON="${PYTHON:-python}"

# Android: defaults of an Android Studio installation on Windows, if not given.
if [ -z "${ANDROID_HOME:-}" ] && [ -n "${LOCALAPPDATA:-}" ]; then
    export ANDROID_HOME="$LOCALAPPDATA/Android/Sdk"
fi
if [ -z "${ANDROID_NDK_ROOT:-}" ]; then
    ANDROID_NDK_ROOT=$(ls -d "$ANDROID_HOME"/ndk/* | sort -V | tail -1)
    export ANDROID_NDK_ROOT
fi
export ANDROID_NDK_HOME="$ANDROID_NDK_ROOT"
if [ -z "${JAVA_HOME:-}" ] && [ -d "/c/Program Files/Android/Android Studio/jbr" ]; then
    export JAVA_HOME='C:\Program Files\Android\Android Studio\jbr'
fi
export CARGO_APK_RELEASE_KEYSTORE="$KEYDIR/release.jks"
CARGO_APK_RELEASE_KEYSTORE_PASSWORD=$(cat "$KEYDIR/keystore-password.txt")
export CARGO_APK_RELEASE_KEYSTORE_PASSWORD

echo "== tests"
cargo test -q --workspace --exclude atlas-xr
echo "== APK $VERSION"
cargo apk build -p atlas-xr --release
echo "== Atlas Bridge"
cargo build --release -p atlas-bridge

# The APK must be signed with the release key, not the debug one.
APKSIGNER=$(ls "$ANDROID_HOME"/build-tools/*/apksigner* | sort -V | tail -1)
CERT=$("$APKSIGNER" verify --print-certs target/release/apk/AtlasCode.apk)
if echo "$CERT" | grep -q "Android Debug"; then
    echo "the APK is signed with the debug key: not publishing it" >&2
    exit 1
fi
echo "$CERT" | grep -m1 "certificate DN"

OUT="dist/atlas-code-$VERSION"
rm -rf "$OUT"
mkdir -p "$OUT"
cp target/release/apk/AtlasCode.apk "$OUT/AtlasCode-$VERSION.apk"
if [ -f target/release/atlas-bridge.exe ]; then
    cp target/release/atlas-bridge.exe "$OUT/AtlasBridge-$VERSION-windows.exe"
elif [ "$(uname)" = "Darwin" ]; then
    cp target/release/atlas-bridge "$OUT/AtlasBridge-$VERSION-macos"
fi
cp LICENSE "$OUT/LICENSE.txt"
cp README.MD "$OUT/README.md"
"$PYTHON" scripts/third_party_licenses.py > "$OUT/THIRD-PARTY-LICENSES.txt"
(cd "$OUT" && sha256sum -- * > SHA256SUMS)
"$PYTHON" -c "import shutil,sys; shutil.make_archive(sys.argv[1], 'zip', 'dist', sys.argv[2])" "dist/atlas-code-$VERSION" "atlas-code-$VERSION"
echo "== done: $OUT and dist/atlas-code-$VERSION.zip"
cat "$OUT/SHA256SUMS"

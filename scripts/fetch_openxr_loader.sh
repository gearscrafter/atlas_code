#!/usr/bin/env bash
# Downloads the OpenXR loader for Android (Khronos) and puts it where `cargo-apk`
# packages it: crates/atlas-xr/libs/arm64-v8a/libopenxr_loader.so
#
# It comes from Maven Central and its SHA-256 hash is checked, so that an altered
# download or a different version does not go unnoticed.
#
# Requires: curl and unzip.
set -euo pipefail

VERSION="1.1.63"
SHA256="0b72f11b0009e1cb4d3a4d2136b7a773cab7db3691ac7a6c65a32e5a7a43b1c5"
URL="https://repo1.maven.org/maven2/org/khronos/openxr/openxr_loader_for_android/${VERSION}/openxr_loader_for_android-${VERSION}.aar"

root="$(cd "$(dirname "$0")/.." && pwd)"
dest="$root/crates/atlas-xr/libs/arm64-v8a"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

echo "Descargando OpenXR loader ${VERSION}..."
curl -fsSL -o "$work/loader.aar" "$URL"
unzip -oq "$work/loader.aar" 'jni/arm64-v8a/libopenxr_loader.so' -d "$work"

actual="$(sha256sum "$work/jni/arm64-v8a/libopenxr_loader.so" | cut -d' ' -f1)"
if [ "$actual" != "$SHA256" ]; then
    echo "ERROR: the hash does not match." >&2
    echo "  esperado: $SHA256" >&2
    echo "  obtenido: $actual" >&2
    exit 1
fi

mkdir -p "$dest"
cp "$work/jni/arm64-v8a/libopenxr_loader.so" "$dest/"
echo "Listo: $dest/libopenxr_loader.so (sha256 verificado)"

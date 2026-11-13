#!/usr/bin/env bash
# Compiles the GLSL shaders in crates/atlas-xr/shaders to SPIR-V (.spv).
#
# The .spv files are versioned next to the source so that building the app does not require
# a shader compiler. Only run this when a .vert/.frag changes.
#
# Uses `glslc` from the PATH, or the NDK's if ANDROID_NDK_ROOT is set.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
dir="$root/crates/atlas-xr/shaders"

glslc="${GLSLC:-glslc}"
if ! command -v "$glslc" >/dev/null 2>&1 && [ -n "${ANDROID_NDK_ROOT:-}" ]; then
    for candidate in "$ANDROID_NDK_ROOT"/shader-tools/*/glslc "$ANDROID_NDK_ROOT"/shader-tools/*/glslc.exe; do
        [ -x "$candidate" ] && glslc="$candidate" && break
    done
fi
command -v "$glslc" >/dev/null 2>&1 || [ -x "$glslc" ] || { echo "glslc not found (set GLSLC or ANDROID_NDK_ROOT)" >&2; exit 1; }

for src in "$dir"/*.vert "$dir"/*.frag; do
    out="$src.spv"
    "$glslc" --target-env=vulkan1.1 -O "$src" -o "$out"
    echo "compilado: $(basename "$out") ($(wc -c < "$out") bytes)"
done

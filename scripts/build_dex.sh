#!/usr/bin/env bash
# Compiles the Java helper (crates/atlas-xr/java) to a .dex that the app embeds and loads at
# runtime. cargo-apk does not compile Java, so the .dex is versioned (like the .spv files):
# this script only needs to run again when the Java code changes.
#
# Requires: a JDK (javac) and the Android SDK (android.jar and d8).
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
sdk="${ANDROID_HOME:-$LOCALAPPDATA/Android/Sdk}"
jdk="${JAVA_HOME:-/c/Program Files/Android/Android Studio/jbr}"
jdk="$(cd "$jdk" 2>/dev/null && pwd || echo "$jdk")"

platform="$(ls "$sdk/platforms" | grep -E '^android-[0-9]+$' | sort -t- -k2 -n | tail -1)"
android_jar="$sdk/platforms/$platform/android.jar"
build_tools="$(ls "$sdk/build-tools" | sort -V | tail -1)"
d8="$sdk/build-tools/$build_tools/d8"
[ -f "$d8" ] || d8="$d8.bat"

out="$root/target/dex"
rm -rf "$out"
mkdir -p "$out/classes"

"$jdk/bin/javac" --release 8 -Xlint:-options -cp "$android_jar" -d "$out/classes" \
  "$root"/crates/atlas-xr/java/com/atlascode/xr/*.java

# d8 takes the .class files; the result is classes.dex with the app's minimum API (29).
classes=$(find "$out/classes" -name '*.class')
"$d8" --min-api 29 --lib "$android_jar" --output "$out" $classes

cp "$out/classes.dex" "$root/crates/atlas-xr/assets/atlas_input.dex"
echo "Listo: crates/atlas-xr/assets/atlas_input.dex ($(wc -c < "$root/crates/atlas-xr/assets/atlas_input.dex") bytes)"

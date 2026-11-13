#!/usr/bin/env bash
# Takes a capture of what the Quest shows right now (the foreground app
# plus the passthrough) and saves it on the PC.
#
# Uso: scripts/quest_capture.sh [archivo_destino.jpg]
#
# Requires `adb` and the headset on (or with proximity simulated: see the README).
# The capture is made by the system service `com.oculus.metacam`; it comes out at 3840x2160.
set -euo pipefail

ADB="${ADB:-adb}"
dest="${1:-captura_quest.jpg}"
dir=/sdcard/Oculus/Screenshots

before="$("$ADB" shell ls "$dir" | wc -l)"
"$ADB" shell am startservice -n com.oculus.metacam/.capture.CaptureService -a TAKE_SCREENSHOT >/dev/null

for _ in $(seq 1 15); do
    sleep 1
    [ "$("$ADB" shell ls "$dir" | wc -l)" -gt "$before" ] && break
done

latest="$("$ADB" shell ls -t "$dir" | head -1 | tr -d '\r')"
[ -n "$latest" ] || { echo "No capture was generated" >&2; exit 1; }
"$ADB" pull "$dir/$latest" "$dest" >/dev/null
echo "Capture saved to $dest ($latest)"

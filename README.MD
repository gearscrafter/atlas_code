# Atlas Code

A programming environment **native to Meta Quest 3**, written in **Rust** (OpenXR + Vulkan), with an orange-and-cyan
glass look over the passthrough. A Vim-style editor floats above your real desk, a keyboard lies flat on the table, your
files fan out as cards you pull from the air, and with **Atlas Bridge** your laptop screen sits right beside you. Like
Neovim, the environment itself is the product: you shape it with **Lua**.

> **Status:** early access (0.1). Tested on Quest 3. Hand gesture thresholds are still being tuned with real hands.
> Atlas Bridge runs on Windows; the Mac version has not been built yet.

## Why

Building for XR means going back and forth: write on a monitor, put the headset on, try it, take it off. And typing
*inside* a headset is the most awkward part: typing in the air is tiring, and aiming at floating keys is slow. Atlas
works on both: a loop where you never take the headset off, and a way of writing designed for hands and a real desk.

Principles:

- **Hands first.** A physical keyboard is optional.
- **Your room is the workspace.** Things rest on your desk and stay where you put them.
- **Customizable like Vim.** Tuned defaults, all replaceable from `init.lua`.
- **Standard formats.** Scenes export to glTF (see `atlas-scene`).

Next on the road: a prototyping loop where a Lua script declares what you want to see in your room, you see and grab
it at real size, reload instantly, and export it to glTF for Blender, Godot or Unity.

## What it does

- **Modal editor** (Vim-style): Normal, Insert, Visual and Command; undo/redo, registers, `dd`, `yy`, `w`/`b`,
  `gg`/`G`, `ZZ`/`ZQ`, and `Ctrl+S` to save.
- **Syntax highlighting** (tree-sitter) for Rust, Python, JavaScript, JSON and C.
- **Files**: a browser inside the panel, and a **drawer of file cards** that fan out on your desk.
- **Git**: `:clone`, `status`, `diff`, `log`, `commit`, `fetch`, `pull` (fast-forward only) and `push`, including
  private repositories with a token, with progress.
- **Keyboard on your desk**, with a Vim row, a symbols layer and a trackpad. Meta's system keyboard is optional.
- **Hands**: Meta's hand mesh as a glass silhouette; pinch, fist and whole-hand gestures.
- **Orbs** that change the environment: grab one with your whole hand, crush it on the desk or throw it at a wall.
- **Atlas Bridge**: files both ways with your laptop, its screen beside the editor, and (if you allow it) its mouse
  and keyboard from the headset.
- **Lua 5.4 scripting**: see [docs/API-LUA.md](docs/API-LUA.md) (v0, experimental).

## Gestures (defaults, tunable from Lua)

| Gesture | Effect |
|---|---|
| Closed fist, pull down / pull up | Open / close the editor (a holographic scroll unrolls) |
| Pinch in the air above keyboard height and pull toward you | Open the file drawer; the cards fan out |
| Pinch anywhere in that zone and push | Close the drawer |
| Swipe with an open palm over the cards | Page through the cards |
| Tap a card | Open the file (or enter the folder) |
| Pinch one of the keyboard's L-shaped corners | Move the keyboard (the middle of the keyboard never moves it) |
| Grab an orb with your whole hand; crush it or throw it | Change the environment |
| Open palm, flat and still, on a desk or against a wall | Map the desk / a wall |
| Two hands pinching for 1 s, or hold the Meta button | Recenter |

With Atlas Bridge connected, also:

| Gesture | Effect |
|---|---|
| Point at the laptop screen from afar and pinch, then move your hand | Pull it closer or push it away (amplified) |
| Pinch one of its corners and stretch | Resize it |
| Close your hand on it | Carry it up close |
| Drop a file card so that, from your eyes, it covers the screen | Send the file to the laptop |
| With `:remote`: point and pinch | Move and click the laptop's mouse (hold to drag) |

## Keys and commands

| Key / command | Effect |
|---|---|
| `i` `a` `o` `Esc` | insert / append / new line / back to Normal |
| `h j k l`, `w b`, `gg G` | move |
| `v` + motion, `y`, `d`, `c`, `p` | select, yank, delete, change, paste |
| `u`, `Ctrl+R` | undo, redo |
| `Ctrl+S`, `:w` | save (in any mode) |
| `:q` / `:q!` | close the file (saving or not) and show its folder |
| `:wq`, `ZZ` | save and close the file |
| `:q` from the list, `:qa` | quit the app |
| `Backspace`, `-` | from a file: open its folder list |
| `:e <path>`, `:ls`, `:back` | open/create, list, go back to the previous buffer |
| `:clone <url> [folder]` | clone a repository |
| `:git status\|diff\|log\|commit <msg>\|fetch\|pull\|push` | git operations |
| `:git token [host] [user] <token>`, `:git creds`, `:git logout` | credentials |
| `:run`, `:reload` | run the buffer as Lua / reload `init.lua` |
| `:font <n>`, `:panel <m>`, `:distance <m>` | try text size, panel width and distance live (`+0.1` / `-0.1` to nudge); put the values you like in `init.lua` |
| `:log`, `:log clear` | show/hide and clear the output panel |
| `:caption <big text> \| <small text>` | show a large caption in front of you for a few seconds (for demos and videos) |

Atlas Bridge:

| Command | Effect |
|---|---|
| `:bridge` | listen for Atlas Bridge on your laptop (shows the address and a pairing code) |
| `:send [file]` | send the open file (as it is in the editor, even unsaved) or the named one to the laptop's `Downloads/Atlas` |
| `:screen here` / `:screen <width>` | bring the laptop screen 70 cm in front of you, or set its width in metres |
| `:remote` / `:remote off` | use the laptop's mouse and keyboard from the headset (the laptop must allow it) |

Developer checks run real gestures with a **test hand** (drawn as a glowing sphere) and report OK or why they failed:

| Command | What the test hand does |
|---|---|
| `:selftest keyboard` | pinches the middle of the keyboard (it must not move), then a corner, and drags it 6 cm and back |
| `:selftest drop` | opens the drawer and drops the selected card in front of the laptop screen (needs a shared screen) |
| `:selftest screen` | pulls the laptop screen closer from afar, then stretches a corner (needs a shared screen) |
| `:selftest remote` | clicks the centre of the laptop screen, waits 12 s for typing, then pinches the editor (needs `:remote`) |

Files live in `/sdcard/Android/data/com.atlascode.xr/files/` (copy them with `adb push` / `adb pull`).

## Install on your Quest (sideload)

Atlas is open source and is not on the Meta store: you install the APK yourself.

1. Turn on **developer mode** for your Quest in the Meta Horizon phone app, and allow USB debugging.
2. Download `AtlasCode-<version>.apk` from the releases (or SideQuest), connect the headset to your PC and install it
   (SideQuest does this for you):

   ```bash
   adb install -r AtlasCode-<version>.apk
   adb shell am start -n com.atlascode.xr/android.app.NativeActivity
   ```

3. Afterwards it also shows up in the headset under **Library > Unknown Sources**.

Supported: Quest 3 and Quest 3S. Published APKs are signed with the project's release key, so each new version installs
over the previous one and keeps your files.

## Atlas Bridge: work next to your laptop

Atlas Bridge is a small program for your laptop that connects it with Atlas over your local network (both on the same
Wi-Fi).

**Get it.** Windows: download `AtlasBridge-<version>-windows.exe` from the release and run it. It is not code-signed
yet, so the first time Windows SmartScreen warns you: choose **More info → Run anyway**. Mac: not available yet. To
build it yourself: `cargo build --release -p atlas-bridge`.

**Connect.** On the headset, type `:bridge`: Atlas shows its address and a **6-digit code**. On the laptop, enter
both in Atlas Bridge and press **Connect**.

**Files.** Drop files on the Atlas Bridge window: they land in Atlas's `inbox/` folder and show up in the card drawer.
The other way, drop a card in front of the laptop screen or type `:send`: the file lands in `Downloads/Atlas` on the
laptop and Atlas Bridge shows it selected in the file explorer (untick the box to stop that). **Open folder** opens
the folder. Nothing is ever overwritten.

**Screen.** Turn on **Share screen**: your main screen appears to the left of the editor, and stays where you leave it,
also if you reconnect. Reach it from afar, resize it by its corners, or bring it back with `:screen here` (see the
gestures above). The headset asks the laptop for the resolution the screen needs as you see it: about 1280 px wide
when it is far, up to 2560 px when it is big and close, at 8–12 images a second (more pixels, fewer images). It is
made for reading, not for video. If the images stop for 4 seconds, the screen hides.

**Control.** On Atlas Bridge, tick **Let the headset control this laptop's mouse and keyboard** (it starts off). Then
type `:remote` on the headset: the ray's cursor moves the laptop's mouse and a pinch is a click. After a click on the
laptop screen, the keyboard (Atlas's own or a Bluetooth one) types there, and the screen gets an orange outline; pinch
the Atlas editor to type in Atlas again. `:remote off` stops it. While it is on, keys your fingers touch on the virtual
keyboard go to the laptop.

**Security.** Pairing uses SPAKE2, so the code never travels over the network, and everything after it is encrypted
(ChaCha20-Poly1305). Three wrong codes close the bridge; type `:bridge` again for a new code. File names are cleaned so
nothing can be written outside `inbox/` or `Downloads/Atlas`, and files over 64 MB are refused. The laptop obeys the
mouse and keyboard only while you allow it.

**Without a window:** `atlas-bridge send <address> <code> <file>...`, `atlas-bridge screen <address> <code>
[seconds]`, and `atlas-bridge session <address> <code> <seconds> [--screen] [--reveal] [--control] [file...]` (does
what the window does, on the same connection, and prints what happens).

On a Mac, once it exists, Atlas Bridge will need **Screen Recording** and **Accessibility** permissions (System
Settings > Privacy & Security), and Ctrl will become Cmd.

## Build

Requirements: stable Rust with the Android target, `cargo-apk`, Android SDK + NDK 28, a JDK (the one from Android
Studio works), `curl` and `unzip`, and a Quest 3 with developer mode and USB debugging.

```bash
rustup target add aarch64-linux-android
cargo install cargo-apk

# Once: download the OpenXR loader (verifies its SHA-256)
scripts/fetch_openxr_loader.sh

export ANDROID_HOME=...                       # Android SDK folder
export ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/28.2.13676358
export JAVA_HOME=...                          # a JDK

cargo apk build -p atlas-xr                   # debug build
adb install -r target/debug/apk/AtlasCode.apk
adb shell am start -n com.atlascode.xr/android.app.NativeActivity
```

Use the **release build** day to day (smaller and not debuggable). It needs a signature; for personal use the local
debug key works:

```bash
export CARGO_APK_RELEASE_KEYSTORE=$HOME/.android/debug.keystore
export CARGO_APK_RELEASE_KEYSTORE_PASSWORD=android
cargo apk build -p atlas-xr --release
```

To prepare a published version (signed with the project's key, with Atlas Bridge, licenses and checksums), see
[docs/RELEASE.md](docs/RELEASE.md).

## Test without the headset

Everything except `atlas-xr` is plain Rust and is tested on the PC:

```bash
cargo test --workspace --exclude atlas-xr
cargo run -p atlas-ui --example preview -- panel.png "jjiHello^:w~"   # draw the panel to a PNG
```

End-to-end git tests (`crates/atlas-git/tests`) start a local HTTP git server with authentication and need `git` and
`python` on the PATH; if they are missing the tests are skipped with a notice. Tests that use the real network are
marked `#[ignore]` (`cargo test -- --ignored`). On the headset, the `:selftest` commands check the gestures.

## Layout

| Crate | Contents |
|---|---|
| `atlas-editor` | Text (rope), modal editing, files, browser, output log. No XR, no Android |
| `atlas-syntax` | tree-sitter highlighting |
| `atlas-ui` | CPU drawing of the panels, file cards, keyboard and alerts. Previewable on the PC |
| `atlas-git` | Clone (`gix`), status/commit/push/pull (`libgit2`), own HTTP transport, credentials |
| `atlas-spatial` | 3D math without XR: anchor, hands and gestures, desk, walls, throw physics, drawer, keyboard |
| `atlas-script` | Sandboxed Lua interpreter and the scripting API |
| `atlas-scene` | Scene model (nodes, shapes, materials), JSON save format and glTF export. Not wired to the headset yet |
| `atlas-link` | The headset ↔ laptop protocol: pairing, encryption, files, screen images, mouse and keyboard orders |
| `atlas-bridge` | Atlas Bridge, the laptop program (window and command line) |
| `atlas-xr` | The app: OpenXR, Vulkan, passthrough, hands, keyboard, panels, Atlas Bridge side and Keystore |
| `vendor/` | Replacement for `rustls-platform-verifier` (see below) |

## Credential security

- Tokens are encrypted with **AES-256-GCM** using a non-exportable **Android Keystore** key, and stored in the app's
  `no_backup` folder (Android excludes it from backups).
- If the Keystore does not work, **no token is stored in plain text**.
- The token never appears in the remote URL, in `.git/config` or in the log, and is masked (`****`) while typed.
- Only the **release** build prevents reading app data with `adb shell run-as`.
- Known limit: `cargo-apk` cannot set `android:allowBackup="false"`; the app is still marked as backup-able, which is
  offset by `no_backup` and encryption.
- Prefer a **fine-grained** token (limited to the repositories you need, with an expiry).

## Decisions and limits

- **TLS**: `reqwest` verifies certificates with `rustls-platform-verifier`, which on Android needs a Java component
  that a native `cargo-apk` app cannot ship. `vendor/rustls-platform-verifier` replaces it with verification against
  bundled Mozilla root certificates. Consequence: the system certificate store is not used (a corporate CA installed
  on the device will not be trusted).
- Git only speaks **HTTP/HTTPS** (no SSH), `pull` is fast-forward only and `:git commit` stages everything.
- Atlas Bridge needs the laptop and the headset on the same local network.

## License

Apache 2.0. See `LICENSE`. Third-party components: [docs/THIRD-PARTY.md](docs/THIRD-PARTY.md); every release ships
`THIRD-PARTY-LICENSES.txt`.

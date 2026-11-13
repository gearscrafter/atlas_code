# Changelog

Based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.1.0] - 2026-11-15

First early-access version. A complete rewrite in Rust of an earlier C++/Skia prototype.

### Added

**In the headset**

- OpenXR session over Vulkan (multiview) with passthrough on Meta Quest 3; floating panels drawn on the CPU and shown
  as Quad layers (sharp text), each on its own thread.
- Modal editor (Normal, Insert, Visual, Command) with undo/redo, registers, `ZZ`/`ZQ` and `Ctrl+S`.
- Syntax highlighting with tree-sitter (Rust, Python, JavaScript, JSON, C) and diff colors.
- Git: `clone`, `status`, `diff`, `log`, `commit`, `fetch`, `pull`, `push`, with progress; private repositories with a
  token; credentials encrypted with the Android Keystore.
- Hands: Meta's hand mesh as a glass silhouette; pinch, fist and whole-hand gestures; the editor opens as a scroll with
  a closed fist.
- A drawer of file cards: pinch in the air above keyboard height and pull; the cards fan out on your desk.
- A virtual keyboard flat on your desk with a Vim row, a symbols layer and a trackpad. It moves only by pinching one
  of its L-shaped corners, which light up when your hand comes close. Optional system keyboard.
- Orbs that change the environment (crush or throw them); desk and wall mapping with the palm.
- Lua 5.4 scripting (API v0, experimental): orbs, gesture events, `config`, `on_frame`, `init.lua`, `:run`, `:reload`.
- `:caption` for large captions in demos and videos.

**Atlas Bridge** (Windows)

- Pairing with a 6-digit code (SPAKE2, then ChaCha20-Poly1305), files both ways (`inbox/` on the headset,
  `Downloads/Atlas` on the laptop, shown in the file explorer), `:send`, and dropping a card on the laptop screen.
- The laptop screen beside the editor, at a resolution that adapts to how big you see it (up to 2560 px). Reach it
  from afar with a ray, pull it closer, resize it by its corners, or bring it back with `:screen here`.
- `:remote`: with the laptop's permission, its mouse and keyboard from the headset.
- A command-line mode (`send`, `screen`, `session`).

**For developers**

- `:selftest keyboard | drop | screen | remote`: real gestures performed by a visible test hand, with OK or the reason
  for a failure.
- `atlas-scene`: a scene model (nodes, shapes, materials, anchors) saved as JSON and exported to glTF 2.0 (not wired
  to the headset yet).
- `scripts/release.sh`: a signed release with Atlas Bridge, third-party licenses and checksums.

### Removed

- The C++ prototype, the Android/Gradle project, Skia and the Neovim files.
- A first attempt at Meta's virtual keyboard extension (deprecated by Meta) and an unused row-style carousel layout.

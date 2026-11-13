# Publishing Atlas Code (sideload / SideQuest)

## The release key

The APK is signed with a release key that lives **outside the repository**, in `~/.atlas-release/`
(`release.jks` and `keystore-password.txt`). Every update must be signed with this same key; otherwise Android refuses
to install it over the previous version and people have to uninstall Atlas, losing their files on the headset.

**Back it up now** (a password manager or an encrypted drive, and a second place). If it is lost, there is no way to
publish an update that installs over the existing one. Never commit it.

Certificate: `CN=Atlas Code, O=Atlas Code`, RSA 4096,
SHA-256 `1A:34:98:F3:CA:EC:7D:E4:C9:A5:33:2B:BA:AD:E6:20:BD:39:D7:D6:FC:56:76:A9:0A:C4:9C:E4:45:CC:A4:F5`.

## Build a release

```bash
scripts/release.sh
```

It runs the tests, builds the APK signed with the release key (and refuses to continue if it is signed with the debug
key), builds Atlas Bridge, and writes `dist/atlas-code-<version>/` and a `.zip` with the APK, Atlas Bridge, `LICENSE`,
`README`, `THIRD-PARTY-LICENSES.txt` and `SHA256SUMS`. Raise `version` in the workspace `Cargo.toml` for each release
(the Android version code comes from it).

## Before publishing: 10 minutes with your hands

The automated checks (`:selftest drop`, `keyboard`, `screen`, `remote`) prove the logic on the headset, but the gesture
thresholds were never tuned with real hands. Do this once per release, without the PC driving anything:

1. Open Atlas. Close your fist and pull down: the editor unrolls. Type `i`, a few words, `Esc`, `:w`.
2. Rest your palm on the desk until it says "desk mapped". The keyboard lies on the desk; type on it.
3. Pinch a **corner** of the keyboard (the L grips light up) and move it. Pinch the middle: it must not move.
4. Pinch in the air above keyboard height and pull: the drawer opens and the cards fan out. Swipe with an open palm to
   page. Tap a card to open the file. Push the drawer to close it.
5. `:bridge`, connect Atlas Bridge on the laptop, drop a file: it shows up in `inbox/`. `:send`: it lands in
   `Downloads/Atlas`.
6. Share the screen. Point at it from afar, pinch and pull it closer; stretch a corner; `:screen here`.
7. Tick "Let the headset control…" on the laptop, `:remote`, click a text box on the laptop screen and type; pinch the
   editor to type in Atlas again; `:remote off`.

Note anything that did not respond on the first try: that is what a new user hits first.

## Distributing

1. **First, a GitHub release** with the `.zip` contents, shared with a few Quest 3 owners. Collect what confuses them.
2. **Then SideQuest** (marked **Early Access**), uploading the APK from `dist/`. The listing's icon, title image and
   banner come from `scripts/store_art_abstract.py` (`dist/store/abstract/`).

## Known limits to state in the release notes

- Tested only on Quest 3. Hand gesture thresholds are estimates.
- Atlas Bridge: Windows only for now (the Mac version has never been built). The `.exe` is not code-signed, so Windows
  SmartScreen warns the first time ("More info → Run anyway").
- Laptop and headset must be on the same Wi-Fi.
- Laptop control is off by default and must be allowed on the laptop; while it is on, keys your fingers touch on the
  virtual keyboard go to the laptop.

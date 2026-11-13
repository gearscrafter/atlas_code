"""Writes to standard output the licenses of every dependency of Atlas (the headset and Atlas Bridge): for each
crate, its name, version and declared license, and the text of its LICENSE / COPYING / NOTICE files. That way every
release includes the copyright notices that MIT, Apache-2.0 and BSD require.

Usage: python scripts/third_party_licenses.py > THIRD-PARTY-LICENSES.txt
"""

import json
import pathlib
import re
import subprocess
import sys

NOTICE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|NOTICE|UNLICENSE|COPYRIGHT)", re.IGNORECASE)

# What is not a crate but goes inside the APK.
EXTRA = """
== OpenXR loader (libopenxr_loader.so), Khronos Group -- Apache-2.0
Copyright (c) 2017-2024 The Khronos Group Inc. Licensed under the Apache License, Version 2.0 (see LICENSE.txt).
Downloaded from the official Khronos release by scripts/fetch_openxr_loader.sh.
"""


def license_files(root: pathlib.Path):
    files = [p for p in sorted(root.iterdir()) if p.is_file() and NOTICE_FILE.match(p.name)]
    # C libraries that some crates bundle (libgit2, zlib...): their license is one level below.
    for sub in sorted(root.iterdir()):
        if sub.is_dir() and not sub.name.startswith((".", "src", "tests", "benches", "examples")):
            files += [p for p in sorted(sub.iterdir()) if p.is_file() and NOTICE_FILE.match(p.name)]
    return files


def main():
    meta = json.loads(subprocess.check_output(["cargo", "metadata", "--format-version", "1", "--locked"]))
    workspace = set(meta["workspace_members"])
    out = sys.stdout
    out.reconfigure(encoding="utf-8")
    out.write("Third-party software in Atlas Code and Atlas Bridge\n")
    out.write("=" * 52 + "\n\n")
    out.write("Atlas Code itself is licensed under the Apache License 2.0 (LICENSE.txt).\n")
    out.write("It includes the following third-party components, under their own licenses:\n")
    out.write(EXTRA)
    for pkg in sorted(meta["packages"], key=lambda p: (p["name"], p["version"])):
        if pkg["id"] in workspace:
            continue
        root = pathlib.Path(pkg["manifest_path"]).parent
        out.write(f"\n== {pkg['name']} {pkg['version']} -- {pkg.get('license') or 'see files below'}\n")
        files = license_files(root)
        if not files:
            out.write("(no license file in the package; see the declared license above)\n")
        for f in files:
            out.write(f"--- {f.relative_to(root)}\n")
            out.write(f.read_text(encoding="utf-8", errors="replace").strip() + "\n")


if __name__ == "__main__":
    main()

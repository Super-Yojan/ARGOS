#!/usr/bin/env python3
"""Lay out a downloadable ARGOS build and zip it.

macOS gets ARGOS.app with the assets in Contents/Resources/assets; Windows
and Linux get a folder with the executable and assets/ beside it. Both are
the layouts `packaged_asset()` in crates/argos-gpui/src/install.rs looks for.
Every build carries .env.example; copy it to .env beside the app (or in the
user config folder) to set the fleet password and the Maps key.

  python3 scripts/package.py --binary target/release/argos-gpui \
      --platform macos-arm64 --version 0.1.0 --out dist

The macOS bundle is left unzipped so the workflow can sign and notarize it
before `ditto` zips it; pass --zip to zip it here instead.
"""
import argparse
import os
import shutil
import stat
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ASSETS = [ROOT / "crates" / "terra-map" / "src" / "assets" / "robot.glb"]

README = """ARGOS {version}
==============

The operator dashboard for Terra rovers and the Zorvane simulator: a 3D map
of the fleet, goals, autonomy levels, teleop and FPV video over Zenoh.

Run
---
{run}

Simulation or real fleet
------------------------
--sim connects to a local Zorvane simulator at tcp/127.0.0.1:7447 (prefix
terra/rover, no password). Start Zorvane first:
https://github.com/Super-Yojan/Zorvane/releases

--real (the default) connects to the fleet in your .env. Copy .env.example
to .env beside {beside} and set ARGOS_ZENOH_PASSWORD. ARGOS also reads .env
from {config}. GOOGLE_MAPS_API_KEY in the same file turns on
photorealistic 3D tiles; without it the map shows a notice and still works.

Docs: https://super-yojan.dev/ARGOS/
Source: https://github.com/Super-Yojan/ARGOS
"""

RUN = {
    "macos": """Open ARGOS.app. To choose the mode, run it from Terminal:

  ./ARGOS.app/Contents/MacOS/argos --sim""",
    "windows": """Double-click argos.exe. To choose the mode, run it from PowerShell:

  .\\argos.exe --sim""",
    "linux": """  ./argos --sim

It needs Vulkan and a Wayland or X11 desktop at runtime (Debian/Ubuntu:
libvulkan1 mesa-vulkan-drivers libxkbcommon-x11-0 libasound2).""",
}

BESIDE = {"macos": "ARGOS.app", "windows": "argos.exe", "linux": "argos"}
CONFIG = {
    "macos": "~/Library/Application Support/ARGOS",
    "windows": "%APPDATA%\\ARGOS",
    "linux": "~/.config/argos",
}

NOTICE = """ARGOS third-party notices
=========================

Photorealistic 3D Tiles are fetched at runtime from Google Maps Platform with
your own API key, under Google's terms; map data attribution is shown in the
window. No map data ships in this download.

The executable includes Rust crates under their own licenses (MIT, Apache-2.0,
BSD and similar), and Cisco's OpenH264 compiled from source (BSD-2-Clause).
"""

INFO_PLIST = """<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>argos</string>
  <key>CFBundleIdentifier</key><string>dev.super-yojan.argos</string>
  <key>CFBundleName</key><string>ARGOS</string>
  <key>CFBundleDisplayName</key><string>ARGOS</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{version}</string>
  <key>CFBundleVersion</key><string>{version}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSLocalNetworkUsageDescription</key>
  <string>ARGOS connects to rovers and the Zorvane simulator on your network.</string>
</dict>
</plist>
"""


def copy_assets(dest: Path) -> None:
    dest.mkdir(parents=True, exist_ok=True)
    for source in ASSETS:
        shutil.copy2(source, dest / source.name)


def zip_tree(root: Path, archive: Path) -> None:
    """Zip `root` as a top-level folder, keeping the executable bits."""
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED, compresslevel=9) as out:
        for path in sorted(root.rglob("*")):
            if path.is_dir():
                continue
            info = zipfile.ZipInfo.from_file(path, path.relative_to(root.parent).as_posix())
            info.compress_type = zipfile.ZIP_DEFLATED
            with path.open("rb") as data:
                out.writestr(info, data.read())


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--platform", required=True, help="macos-arm64, windows-x64 or linux-x64")
    parser.add_argument("--version", required=True)
    parser.add_argument("--out", type=Path, default=Path("dist"))
    parser.add_argument("--zip", action="store_true", help="Also zip the macOS bundle")
    args = parser.parse_args()

    os_name = args.platform.split("-")[0]
    if os_name not in RUN:
        raise SystemExit(f"unknown platform {args.platform}")
    name = f"argos-{args.version}-{args.platform}"
    stage = args.out / name
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)

    if os_name == "macos":
        contents = stage / "ARGOS.app" / "Contents"
        (contents / "MacOS").mkdir(parents=True)
        executable = contents / "MacOS" / "argos"
        shutil.copy2(args.binary, executable)
        copy_assets(contents / "Resources" / "assets")
        (contents / "Info.plist").write_text(INFO_PLIST.format(version=args.version))
    else:
        executable = stage / ("argos.exe" if os_name == "windows" else "argos")
        shutil.copy2(args.binary, executable)
        copy_assets(stage / "assets")
    executable.chmod(executable.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    readme = README.format(version=args.version, run=RUN[os_name], beside=BESIDE[os_name], config=CONFIG[os_name])
    (stage / "README.txt").write_text(readme)
    (stage / "NOTICE.txt").write_text(NOTICE)
    shutil.copy2(ROOT / ".env.example", stage / ".env.example")

    if os_name != "macos" or args.zip:
        archive = args.out / f"{name}.zip"
        zip_tree(stage, archive)
        print(archive)
    else:
        print(stage)
    if "GITHUB_OUTPUT" in os.environ:
        with open(os.environ["GITHUB_OUTPUT"], "a") as output:
            output.write(f"name={name}\nstage={stage}\n")


if __name__ == "__main__":
    main()

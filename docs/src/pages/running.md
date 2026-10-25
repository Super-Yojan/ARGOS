---
layout: ../layouts/DocPage.astro
title: Download and run
description: Get ARGOS for macOS, Windows or Linux and open the dashboard. No account is needed to drive the simulator.
---

Builds are on the [releases page](https://github.com/Super-Yojan/ARGOS/releases): one zip each for macOS (Apple Silicon), Windows (x64) and Linux (x64).

| Platform | Run |
| --- | --- |
| macOS | Open `ARGOS.app`. If macOS says it can't verify the developer, right-click the app, choose **Open**, then **Open** again |
| Windows | Double-click `argos.exe`. SmartScreen may ask once: **More info**, then **Run anyway** |
| Linux | `./argos`. It needs Vulkan and a Wayland or X11 desktop: on Debian or Ubuntu, `libvulkan1 mesa-vulkan-drivers libxkbcommon-x11-0 libasound2` |

Keep the `assets` folder beside the executable; on macOS it is inside the app.

Opened with no options, ARGOS connects to the real fleet, which needs a password. To try it against the simulator first, start [Zorvane](https://super-yojan.dev/Zorvane/) and press **Sim** in the console bar, or open ARGOS with `--sim`:

| Platform | Simulation mode |
| --- | --- |
| macOS | `./ARGOS.app/Contents/MacOS/argos --sim` |
| Windows | `.\argos.exe --sim` |
| Linux | `./argos --sim` |

[Simulation or real](/ARGOS/modes) explains both connections. `--help` lists the options.

## The .env file

ARGOS reads its settings from a `.env` file and from the environment. Every download carries `.env.example`; copy it to `.env` and fill in what you need. ARGOS looks for `.env` in these places, and a later one wins:

| Where | Path |
| --- | --- |
| Your config folder | macOS `~/Library/Application Support/ARGOS/.env`, Windows `%APPDATA%\ARGOS\.env`, Linux `~/.config/argos/.env` |
| Beside the app | The folder holding `ARGOS.app`, `argos.exe` or `argos` |
| The current directory | Where you started it from a terminal |

A variable already set in the environment beats every file. The two you are most likely to set are `ARGOS_ZENOH_PASSWORD` for the real fleet and `GOOGLE_MAPS_API_KEY` for the 3D map. [Settings](/ARGOS/settings) lists the rest.

> ⚠ Never commit `.env` or share it. It holds the fleet password and your Maps key.

## The 3D map

The map is Google Photorealistic 3D Tiles. ARGOS fetches them with your own key, from a Google Cloud project with billing and the Map Tiles API turned on:

```sh
GOOGLE_MAPS_API_KEY=your-key
```

Without a key the window still opens and the rovers still draw; a notice on the map says to set the key. Set `MAPS_DEMO=1` in the environment (not `.env`) to draw a small offline fixture instead; it never calls Google.

## Build from source

You need stable Rust (edition 2024), a C toolchain, libvpx, and a sibling Terra checkout at the revision in `.github/scripts/checkout-observability.sh`.

```sh
git clone https://github.com/Super-Yojan/ARGOS.git
cd ARGOS
bash .github/scripts/checkout-observability.sh
cp .env.example .env
cargo run -p argos-gpui -- --sim
```

On macOS, `brew install libvpx`. On Linux, GPUI and the codecs need:

```sh
sudo apt-get install -y pkg-config libssl-dev libvulkan-dev \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libxcb1-dev libx11-dev libxcursor-dev libxi-dev libxrandr-dev \
  libasound2-dev libfontconfig-dev libvpx-dev nasm
```

A source build reads `.env` from the repository root as well as the places above.

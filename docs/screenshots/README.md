# Screenshots

`c3-console/` is the current ARGOS window (`argos-gpui`), recaptured from main on 2026-10-27. The six names are unchanged so existing issue links can be pointed at the new files:

| File | What it shows |
| --- | --- |
| `01-overview-3d.png` | 3D map, console bar, three rover cards |
| `02-overview-2d.png` | Same fleet in 2D |
| `03-inspector-takeover.png` | Rover selected below L0. Primary button is **Take over** |
| `04-inspector-drive.png` | That rover confirmed L0. Primary button is **Drive** |
| `05-inspector-driving.png` | **Leave drive**, manual speed and spin controls, status `Driving rover 1 · W` |
| `06-inspector-stopped.png` | **Safety stop active** and **Reset stop** |

`pilot-ux/` is extra frames for the pilot UX notes (#73–#82). The offline fixture can show them without a live fleet:

| File | Useful for |
| --- | --- |
| `levels-l1-assisted.png` | L1 stays on **Take over**. The drive pad is not a separate tab; it appears on Overview only after **Drive** (#74, #76) |
| `zoomed-out-status.png` | Zoomed out, the three status cards stack on one anchor (#78) |
| `zoomed-in-status.png` | Zoomed in, each card sits on its own rover (#78) |
| `zoomed-in-labels-clipped.png` | Zoomed further, two cards slide off the field and only one status stays readable (#78) |

The map is the offline `MAPS_DEMO=1` quad, so every frame says `Offline GPU fixture · no Google map data`. Goal markers share one green; the selected rover marker turns green too. There is no per-rover goal color to photograph.

## How to reproduce

From the repo root, with a Vulkan driver (lavapipe is enough):

```sh
cargo build -p argos-gpui --bin argos-gpui --example screenshot-fixture
scripts/capture-c3-screenshots.sh up
```

That starts `screenshot-fixture` on `tcp/127.0.0.1:7447` (prefix `terra/rover`) and `argos-gpui --sim` with `MAPS_DEMO=1`. The fixture publishes rovers 1–3 with goals, and applies `autonomy`, `safety`, and `goal` commands so Take over, Drive, and Stop change the window.

`scripts/capture-c3-screenshots.sh shot <png>` grabs the window. Drive mode ends on the next frame unless the window is focused (`Drive stopped because the window lost focus`). These frames were taken on the XFCE display, where the window can be focused. A bare Xvfb session is enough for the overview and the Take over / Drive buttons, not for a held W key.

Scroll zooms the map. Select a rover card, then **Take over**, **Drive**, hold W, and **Stop**. `scripts/capture-c3-screenshots.sh down` stops the app, the fixture, and an Xvfb this script started.

# ARGOS

!!! tip "TL;DR"
    You sit at **ARGOS**.
    **Terra** is the vehicle. **TerraPhone** is the phone on that vehicle.
    **Zorvane** is the world.
    They meet on Zenoh. The prefix is `terra/rover`.

Click a box.

<object class="system-diagram" data="assets/system.svg" type="image/svg+xml" aria-label="Clickable diagram of ARGOS, Terra, TerraPhone, and Zorvane">
  <img src="assets/system.svg" alt="ARGOS links over Zenoh to Terra, TerraPhone, and Zorvane">
</object>

*Operator → ARGOS → Zenoh `terra/rover` → Terra (TerraPhone on the robot) and the Zorvane simulator.*

## What you open today

<div class="shot-row" markdown="1">

<figure class="wide" markdown="1">
![ARGOS on Mac. Rover 8 sits on a map. The goal is active.](assets/macos-dashboard.jpg)
<figcaption>Mac app. Simulator run, 2026-10-06. Live pose, active goal.</figcaption>
</figure>

<figure class="phone" markdown="1">
![ARGOS on an iPhone simulator. Fleet list, rover 0, goal cancelled.](assets/ios-dashboard.jpg)
<figcaption>iPhone simulator. Same prefix, after a waypoint run.</figcaption>
</figure>

</div>

The shipping screen is a rover list plus a map.

Send a waypoint. Take over. Stop.

The [scene without the list](design/operator-dashboard/README.md) is the design draft.

![Grayscale wireframe. One map, robots as objects, no roster.](design/operator-dashboard/wireframes/01-main-layout.png)

*Planned iPad scene. One place. Robots are objects.*

## Where to go

| Go | You will see |
| --- | --- |
| [Vision and command model](vision.md) | Intent, tasking, or a point |
| [Two-stage VLA](vla.md) | Staff model, then the vehicle model. Planned |
| [Zenoh](zenoh.md) | Keys this app actually uses |
| [Getting started](getting-started.md) | Run ARGOS against Zorvane |
| [Deployment](deployment.md) | LAN today. AWS tunnel is planned |
| [Roadmap](roadmap.md) | [Project board](https://github.com/users/Super-Yojan/projects/6) |

Source repos: [ARGOS](https://github.com/Super-Yojan/ARGOS), [Terra](https://github.com/Super-Yojan/Terra), [Zorvane](https://github.com/Super-Yojan/Zorvane).

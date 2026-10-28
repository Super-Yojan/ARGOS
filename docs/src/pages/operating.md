---
layout: ../layouts/DocPage.astro
title: Operate a rover
description: Take control, select a rover, drive it, send it to a goal and choose how much it decides for itself.
---

ARGOS asks; the rover decides. Every command below goes to Terra on the rover (or Zorvane in simulation), which checks it against its own state and limits before anything moves. ARGOS shows what the rover reports back, not what it hoped would happen.

## The window

The **console bar** across the top holds the map mode, the **Sim** | **Real** switch, **Map grid**, the latest status, this window's control claim, link health, and **Stop all**.

Each rover has a callout on the map. Rovers outside the view, and fleet members with no position yet, are tabs along the lower edge. Online rovers with a fresh pose draw in colour; stale and offline rovers are tinted.

| Gesture | Effect |
| --- | --- |
| Drag empty map | Orbit in 3D |
| Shift-drag | Pan |
| Scroll | Zoom |
| Click a rover | Select it and open its inspector |
| Esc | Cancel a goal drag; nothing is sent |

The camera recentres on the fleet when rovers first report a position, and again only when the set of rovers changes.

## Take control and select

**Take control** in the console bar gives this window a 30-second operator lease, renewed while the window is open, so only one window drives at a time. **Release control** gives it back and closes the rover panel.

Click a rover, on the map or in the tabs, to open its **inspector**: its state and trace, **Take over** and **Stop**, and the **Overview** and **Video** tabs. Overview carries hardware, autonomy, navigation, the current goal and, while driving, the drive controls.

ARGOS reads the rover's hardware state (armed, disarmed, motion paused) and shows it under **Hardware**, with the rover's own reason when it gives one. It does not arm the rover; that happens on the rover.

## Drive

Press **Take over**. ARGOS asks the rover for L0 manual control, and the button becomes **Drive** once the rover confirms. Press **Drive**, then hold the keys, or the on-screen W A S D pad:

| Key | Action |
| --- | --- |
| W / Up | Forward |
| S / Down | Reverse |
| A D / Left Right | Turn |
| Space | Stop |

ARGOS sends drive commands about 20 times a second, only while a key is held. Letting go, **Leave drive**, switching rovers or the window losing focus all send zero. The speed buttons (0.1 to 0.5 m/s) and the turn chips set this window's caps, 0.2 m/s and 0.5 rad/s by default ([Settings](/ARGOS/settings#drive-caps)). They are a second line of defence; the rover enforces its own.

## Stop

**Stop** on a rover, Space while driving, or **Stop all** in the console bar latch the rover's safety stop and send a zero command. A latched stop stays until you press **Reset stop** under Hardware. Reset clears the stop; it does not resume driving by itself.

## Send a goal

With the rover on L2 or L3, click the map, or its occupancy grid, to place a destination. A ghost marks the spot with its distance from the rover; the rover stays at its live position. **Send goal** in the popup sends it; **Cancel** or Esc sends nothing. **Cancel goal** in the inspector stops a goal in progress.

New goals are refused while the rover's pose is degraded or stale, and the inspector says why.

### Outdoor GPS

Outdoor navigation is a separate source, chosen in the inspector while the rover is disarmed and idle. Once the rover acknowledges **Outdoor GPS** and is on L3, place a destination the same way: the ghost shows the arrival radius (5 m unless the rover reports another) and the GPS distance, and ARGOS sends a latitude and longitude. The inspector names each reason a mission holds (`GPS stale`, `Heading uncertain`, `Route blocked`, `Paused` and others). Recovered sensors never resume motion by themselves; press **Resume outdoor mission**.

## Autonomy levels

The autonomy buttons ask the rover to change level. The inspector shows the level you asked for and the level the rover is actually in.

| Button | Level | What the rover does |
| --- | --- | --- |
| L0 Manual | `teleop` | Only what you drive |
| L1 Assisted | `assisted_teleop` | Your driving, through its obstacle and braking checks |
| L2 Direct | `waypoint_direct` | Drives straight to the goal you set |
| L3 Waypoint | `waypoint` | Plans a path to the goal you set |
| L4 Explore | `explore` | Picks unmapped frontiers and drives to them, with no per-goal approval |
| Supervised | `supervised` | Proposes frontiers to explore and waits for your approval before each one |
| Search | `target_search` | Searches a bounded area for a target, then holds and reports it |
| L5 Task | `task` | Its on-phone language model or MCP agent turns a task you type into waypoints, and asks you when unsure |

**L4 Explore** runs for the budget you pick under **Exploration · L4**: 3, 5 or 10 min. The inspector shows elapsed and remaining time, the current frontier, area covered, and why the run ended. Stop, Cancel goal, or choosing another level ends it. ARGOS does not send L4 while a stop is latched or while Outdoor GPS is the navigation source.

**L5 Task** is under **Task · L5**. Type what the rover should do, pick 3, 5 or 10 min, and press **Start task** (or the **L5 Task** level button). The phone's own language model reads the phone's map, picks the next waypoints, and Terra checks each one with its L3 planner before driving it. The rover holds while the model thinks. When the model asks a question it appears in amber; type the answer and press **Answer**. While a task runs you can also add guidance with **Guide**, or press **Cancel task**. The inspector shows the state, model calls used, time, the plan, the model's reason and the last few steps. Space types a space while a task box has focus; use **Stop all** to stop then. The rover offers L5 only if its phone can run a model (an iPhone with Apple Intelligence on iOS 26 or later) or has loaded its MCP agent (Qwen 3.5 2B, downloaded on the first phone start). ARGOS does not send a task while a stop is latched or while Outdoor GPS is the navigation source.

When the phone has both, **Built-in model** and **MCP agent** buttons pick who runs the next task; ARGOS prefers the MCP agent. The agent works through Terra's tools: it reads the map, looks through the camera, sends waypoints that Terra checks the same way, and asks you questions. It may also ask for **raw drive** for up to 60 s. That request shows in amber with its reason; nothing moves until you press **Approve drive**. During a grant the agent's speed is capped at 0.5 m/s and the obstacle planner still checks every command. **Deny drive** or **End drive** stops it at once, and the grant ends by itself when its time runs out.

## Agents

The **Agent** panel at the bottom left lets an AI agent put recommendation cards in front of you (Advise, the default) or, in Operator mode, command rovers through these same checks. Raw drive from an agent always waits for **Approve drive**. See [Agents and recommendations](/ARGOS/agents).

## The map grid

**Map grid** draws the selected rover's occupancy grid under it, semi-transparent. A cell at 65 or above is occupied, 0 to 64 is free, and a negative cell is unknown. Only the selected rover's grid is fetched, so selecting another rover fills in its grid within about two seconds.

## 3D capture

When the grid shows obstacles where the floor is clear, select the rover and press **Capture 3D** in the inspector. The rover records every pixel of its depth camera for 10 seconds at 1 frame per second and labels each point the way its mapper classifies it. ARGOS saves the result as a PLY file, coloured by label: free green, obstacle red, overhead blue, out of range grey, outside band yellow. The file also keeps the label number and frame index for each point. Open it in MeshLab, CloudCompare or Blender.

Files go to `Documents/ARGOS Captures` in your home folder, or to `ARGOS_CAPTURE_DIR` if you set it. Each frame is about 640 KB, so a capture sends about 6 MB over the link. The rover sends nothing until you ask, and on a busy link it drops frames instead of slowing the controls. The inspector shows how many frames arrived and the point count for each label.

To watch the classification as the rover moves, press **Live 3D** instead. The rover streams every third pixel across and down, twice a second (about 70 KB per frame), and ARGOS draws the newest frame on the map under the rover with the same colours. Points are raised by their height above the ground under the camera, so obstacles stand up in the 3D view and a tilted ground plane shows as a slope; in 2D you see them from above. Press **Live 3D** again to stop. ARGOS renews the stream every 2 seconds, so if ARGOS closes or loses the link the rover stops on its own within 6 seconds. Capture 3D still saves a full PLY and turns Live 3D off while it records.

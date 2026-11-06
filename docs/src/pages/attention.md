---
layout: ../layouts/DocPage.astro
title: Attention logging
description: Record per-rover wait, interaction and neglect time, plus where the operator looks and how often they blink, for user studies.
---

ARGOS writes what the operator attends to into the session log (`Documents/ARGOS Sessions`). From that log, `attention_times.py` (in the Masters-Thesis repository, next to the "Autonomy Is Not Free Attention" paper) computes wait, interaction and neglect time per rover and level.

| Metric | From | To |
| --- | --- | --- |
| Neglect time (NT) | rover resumes on its own | rover next needs help |
| Wait time (WT) | rover needs help | operator turns to it: select, video, or a gaze dwell |
| Interaction time (IT) | operator turns to it | rover resumes |

## Timeline panel

**Panels → Timeline** (or the View menu, Cmd+Opt+7) opens a bottom panel with one row per rover: the last 5 minutes and the next 10, with a fixed now line. Solid blue is interaction, amber is waiting on you, a thin line is neglect (running alone), and ◇ marks a predicted need: explore or task budget ending, or arrival at the goal at the rover's recent speed. Rows are in queue order: longest wait first, then soonest predicted need, then rovers you are working with. Click a row to select that rover.

A rover waits from the moment it needs help until you select it; from then until it runs again is interaction. A rover at L0/L1 that nobody is driving counts as waiting.

## What is logged

| Record | When | Key fields |
| --- | --- | --- |
| `help` | A rover at L2–L5 stops working for at least 1.5 s (`raised`), or works again or is taken over (`cleared`) | `help_id`, `class` (`done`, `blocked`, `stopped`), `reason`, `level`, `at`, `outcome` |
| `help_id`, `prompted` on `goal_sent`, `action_sent`, `goal_cancel_sent`, `intervention` | A command is sent while that rover waits | links the command to the episode |
| `rover_focus` | Selection changes (`via: select`) or FPV starts or stops (`via: video`) | `rover_id`, `previous` |
| `window_focus` | The ARGOS window gains or loses focus | `active` |
| `callout` | A rover card changes what it says | `tag` (`stopped`, `attention`, `goal`, ...), `state` |
| `pose_sample` | Once a second | every rover's `x`, `y`, `yaw`, `status`, `level` |
| `gaze_dwell` | The gaze leaves an area it held | `area` (`card`, `marker`, `inspector`, `map`, `console`, `window`, `off_window`), `rover_id`, `start_utc`, `duration_s` |
| `gaze_sample` | Up to 10 times a second | screen and window `x`, `y`, `area`, `rover_id` |
| `blink` | The eyes reopen | `duration_ms` |
| `gaze_status` | Tracker connects, calibrates, loses or finds the face | `error_px`, `window` bounds |

A look at a rover card, its map marker or the inspector counts for that rover. A gaze must hold an area for 0.1 s before the dwell switches to it, so saccades across a card do not count.

## Eye tracking and blinks

ARGOS listens for an eye tracker on UDP `127.0.0.1:47870` (`ARGOS_GAZE_PORT`, `0` turns it off). The included tracker uses the laptop webcam:

```sh
uv run scripts/argos_eye_tracker.py              # first run calibrates (13 dots), then tracks
uv run scripts/argos_eye_tracker.py --recalibrate
uv run scripts/argos_eye_tracker.py --simulate   # fake gaze and blinks, no camera
```

Run ARGOS full screen or maximised on the main display. Calibration is saved in `~/.argos/eye-calibration.json` and reused while the screen size is the same. Recalibrate when the laptop, chair or lighting changes. The tracker reports its median error after calibration. Webcam gaze is good to a few degrees, enough to tell which card or part of the map someone looks at.

On macOS, allow camera access for your terminal the first time.

Any other tracker (a Tobii bar, for example) can feed ARGOS by sending the same JSON datagrams: `{"type": "gaze", "utc": ..., "x": ..., "y": ...}` in screen points, `{"type": "blink", "utc": ..., "duration_ms": ...}`, and `{"type": "no_face", "utc": ...}`.

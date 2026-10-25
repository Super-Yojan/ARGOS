---
layout: ../layouts/DocPage.astro
title: FPV video
description: Watch a rover's front camera in the inspector. Video is WebRTC, receive only; driving stays on the Zenoh controls.
---

Select a rover, open the **Video** tab in its inspector, and press **Start FPV**. ARGOS asks the rover to call it, answers, and draws the picture in the tab with its frame rate, bit rate and round-trip time. **Stop FPV** hangs up. One rover streams at a time; if video is still running for a rover you have moved away from, a stop button for it stays in the console bar.

Video is only video. Drive, stop and every other command stay on the Zenoh controls, so a frozen picture never holds a command.

## How the call works

Signaling travels over the same Zenoh connection as everything else, so there is nothing extra to open in simulation or on the LAN. The rover offers H.264 first and VP8 as a fallback. A call that stalls is ended: 15 seconds for the offer, 20 for the answer, and 30 to connect.

In simulation, Zorvane answers with its rover's camera at 640 × 480 and 15 fps.

## Across networks

On the real fleet the media goes peer to peer. ARGOS uses a public STUN server by default, which is enough when either side can be reached. If both sit behind strict NATs, add a TURN server:

```sh
ARGOS_FPV_TURN_URL=turn:turn.example.com:3478
ARGOS_FPV_TURN_USERNAME=operator
ARGOS_FPV_TURN_CREDENTIAL=the-turn-password
```

All three are needed together. Do not put the username or password in the URL. On a local router with no internet, `ARGOS_FPV_STUN=none` keeps to local addresses.

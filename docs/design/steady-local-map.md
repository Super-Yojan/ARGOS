# Steady local map, GPS and satellite corrections

## The problem

Outdoors, a rover's local map (occupancy grid, live cloud, route, local goals,
found buckets) jumped across the 3D map every time the phone's GPS fix changed.

TerraPhone publishes its localization in `gps` or `local` mode and never a
`geographic` alignment, so `fleet_marks` had no map frame for a phone rover.
It drew the rover at the raw fix and built the overlay frame with
`LocalFrame::pinned(raw fix, pose, fused heading)` on every 100 ms poll. Phone
GPS wanders 3 to 5 m from fix to fix, so the whole local map, which is rigid
and accurate in the phone's VIO frame, was dragged to each new fix. Heading
noise also swung it about the rover: 5 degrees moves a cell 10 m ahead by
almost 1 m.

Terra's own outdoor controller already avoids this for guidance (`GeoAnchor`
in `terra-autonomy`), but it publishes only the anchor's sigma, not a frame.

## The fix (stage 1, this branch)

The local frame is never moved. Only one transform is estimated, local metres
to the globe (rotation plus offset, like ROS `map -> odom`):

- `argos_core::FrameAligner` pairs each GPS fix with the local pose at the fix
  time and fits the transform over the last 120 s of pairs, weighted by
  reported accuracy.
- Rotation comes from Terra's fused heading while the rover has barely moved,
  then more and more from the driven GPS track (about 15 m of track outweighs
  the compass), which also cancels a constant compass bias.
- A fix far from the fit is rejected; three in a row re-anchor on the newest
  fix. A new frame ID or run ID, or a pose that teleports, starts over.
- The published frame slides toward each new fit at up to 0.5 m/s and
  3 degrees/s, so corrections are smooth.
- The rover marker is drawn through the same frame, so it moves with VIO and
  the map moves with it.

`ARGOS_MAP_ALIGN=pinned` restores the old behaviour for side-by-side tests.

## Stage 2: known landmarks from the satellite map

`FrameAligner::observe_landmark` takes a local point whose true position is
known, with its own sigma (about 0.5 m), into the same fit. Wiring left to do:

1. The experimenter drops pins on the 3D map before a run (bucket spots,
   cones, a building corner). Pins are hidden from the operator in study runs.
2. When Terra reports a found bucket at local x/y, match it to the nearest pin
   within a gate (for example 6 m, with no second pin within 3 m) and call
   `observe_landmark`. A wrong match is rejected by the same outlier gate.

## Stage 3: matching the map to satellite imagery

With no pins, align the occupancy grid's occupied cells with building
outlines and curbs taken from map data or traced from the satellite image:
a small correlation search over offset (±6 m) and rotation (±10 degrees)
around the current fit. A clear best match is fed back as a landmark-like
correction with its own sigma.

## Later: one estimator on the rover

Move the aligner into Terra and publish a `geographic` localization
(`originLatitude`, `originLongitude`, `anchorX`, `anchorY`, `rotation`).
ARGOS already reads that through `LocalFrame::from_localization` and would
then skip its own aligner, so the dashboard and the rover share one estimate.

## Testing on the rovers

1. Parked test: rover still in the open for 5 minutes, dashboard on `steady`,
   then on `pinned`. Record how far a grid cell 10 m ahead moves per fix.
   Target: under 0.5 m per fix on `steady`.
2. Straight drive: drive 30 m straight and back. The map should not smear;
   a wall or bucket seen going out should line up coming back.
3. Loop and return: drive a loop, park on a taped start mark. The rover
   marker should land within about 2 m of the mark on the satellite map.
4. Compass check: turn the rover in place at several headings. The map should
   rotate with the rover, not wobble on its own.
5. Tracking restart: cover the camera until tracking resets. The map should
   restart cleanly, not stretch.
6. Log every run with session logs on, so the two modes can be compared
   offline.

# Reusable native map crate

Approved direction: a cloneable `Map` controller created from `MapConfig`, plus
`MapView::new(map.clone(), cx)` for GPUI. Robot selection callbacks call
`map.show_lat_lon(lat, lon)`; the view applies smooth, one-shot camera navigation
without changing zoom or 2D/3D orientation. Application types such as Robot and
Fleet remain outside the library.

Ship a local library named `terra_map` in package `terra-map`, preserving the
current `learn-gpui` demo binary and adding a runnable fleet example. Do not
publish the crate or include credentials in package contents.

Public types: Map, MapConfig, MapView, MapMode, GeoPosition, ObjectId, Color,
ObjectStatus, MapEvent and MapError. Commands validate inputs, return typed
errors, and update bounded shared state; no GPUI context is needed by callbacks.
Object IDs belong to one map and cannot alias another map's objects.

Support local static GLB/glTF models initially. Read/decode files off the UI and
render threads. Share decoded and GPU assets across instances of the same model.
Position objects in WGS84 latitude/longitude/ellipsoid altitude, orient in local
ENU coordinates, and support position, heading, positive scale, colored emissive
highlight, clearing highlight, and removal. Highlight changes must not recolor
map tiles. No automatic terrain clamping, skeletal animation or Wavefront OBJ
loader in this first API.

Model load failures are observable through ObjectStatus and a bounded event
queue. Removing an object while it loads must not resurrect it. Bound object
count and decoded model memory. Preserve existing tile budgets, attribution,
mode controls, frame ordering, and diagnostics. Controls can be hidden when the
host app supplies its own UI; map attribution stays visible.

Validation: callback commands from other threads, map-scoped IDs, parameter
validation, interpolation across the date line, local asset loading and sharing,
load failures/removal, GPU highlight isolation and transforms, a compiling fleet
example, packaging contents excluding .env, full regression and live tile checks.

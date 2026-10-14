# GMU Fairfax building footprints

Anchor `38.8297, -77.3075` is the ARGOS and Zorvane default. Footprints are projected at runtime with `CampusAnchor`, the same equirectangular conversion as `argos_core::Anchor`: +x north, +y west, scale `40075016.686 / 360`.

## Data

`gmu-fairfax.json` is an offline extract of OpenStreetMap `building` ways around that anchor, about 320 m on every side, which covers the maps camera's ~180 m ground frame plus a margin. One Overpass query on 2026-10-12 (`timestamp_osm_base` `2026-10-12T02:44:06Z`):

```
[out:json][timeout:90];
(
  way["building"](38.826825,-77.311190,38.832575,-77.303810);
  relation["building"](38.826825,-77.311190,38.832575,-77.303810);
);
out geom;
```

The Exploratory Hall relation repeats a way that is already in the extract, so the bundle keeps the way once. 44 footprints remain.

## Heights

No footprint in this extract has an OSM `height` tag (`withHeight` is 0). Five have `building:levels`, extruded at 3.5 m per level:

| Name | Levels | Metres |
| --- | --- | --- |
| Johnson Center | 4 | 14 |
| Planetary Hall | 4 | 14 |
| Engineering Building | 5 | 17.5 |
| Exploratory Hall | 5 | 17.5 |
| Horizon Hall | 6 | 21 |

The other 39 use the default 10.5 m, which is three of those levels. Parking decks and arenas in the margin are in that default group.

## License

Building footprints are © OpenStreetMap contributors and are made available under the Open Database License (ODbL) 1.0.

https://www.openstreetmap.org/copyright

https://opendatacommons.org/licenses/odbl/1-0/

This file is a derivative database. Share-alike applies to the database, not to the rendered picture.

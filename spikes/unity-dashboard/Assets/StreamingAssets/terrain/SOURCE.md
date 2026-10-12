# GMU Fairfax terrain bundle

Anchor `38.8297, -77.3075` is the ARGOS and Zorvane default (George Mason University, Johnson Center, Fairfax). Local metres use the same equirectangular conversion as `argos_core::Anchor` and the Swift `MapProjection`: +x north, +y west, scale `40075016.686 / 360`.

## Elevation

`gmu-fairfax-terrarium.png` is Terrarium-encoded PNG tile `15/9347/12543` from the AWS Open Data bucket `elevation-tiles-prod` (Mapzen / Tilezen packaging).

```
https://s3.amazonaws.com/elevation-tiles-prod/terrarium/15/9347/12543.png
```

Height in metres is `(R * 256 + G + B / 256) - 32768`.

Fairfax is covered by USGS 3DEP (formerly NED). Tilezen's attribution note says 3DEP products from The National Map are in the public domain. USGS asks for credit when the data is republished:

`United States 3DEP terrain data courtesy of the U.S. Geological Survey.`

The global tile set also mixes other providers. This spike ships only the one tile that contains the GMU anchor. Attribution for the tile service: [Tilezen joerd attribution](https://github.com/tilezen/joerd/blob/master/docs/attribution.md).

## Imagery

`gmu-fairfax-naip.png` is a 4096×4096 USGS NAIP image of that tile's bounding box, from The National Map ImageServer (`USGSNAIPImagery`). The server rejects one export above its size limit, so this file is four 2048×2048 quadrants of the same bbox stitched together. NAIP is a U.S. Government work and is in the public domain.

`NAIP imagery courtesy of the U.S. Geological Survey.`

The Unity material dims the photo and pulls a little saturation out. It does not fog it. Rover status colors stay the brightest marks in the frame.

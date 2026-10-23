# Test fixtures

`textured-quad.glb` is a generated 2 × 2 textured quad, created for this project.
It contains four colored quadrants and the copyright string “Synthetic test fixture”.

`box-draco.gltf` and `Box.bin` are the Khronos glTF Sample Assets Box model,
from the `glTF-Draco` directory:
https://github.com/KhronosGroup/glTF-Sample-Assets/tree/main/Models/Box

Copyright © 2017 Cesium. The model is distributed under the Creative Commons
Attribution 4.0 International license:
https://creativecommons.org/licenses/by/4.0/legalcode
No changes were made to the model data; Box.gltf was renamed box-draco.gltf.
The upstream model README records the license and authorship:
https://github.com/KhronosGroup/glTF-Sample-Assets/blob/main/Models/Box/README.md

The POSITION attribute uses Draco unique ID 1, while NORMAL uses ID 0. This
catches implementations that incorrectly assume POSITION always has ID 0.

`rgb-quad.gltf` and `rgb-quad.drc` are generated synthetic fixtures. The RGB
attribute has two unique color values mapped non-sequentially onto four points,
which verifies normalization and point-to-attribute mapping in the native decoder.

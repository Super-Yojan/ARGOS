#!/usr/bin/env python3
"""Make a rover marker model for the ARGOS map from a CAD glTF.

The map's object shader is unlit, so a plain CAD mesh draws as a flat
silhouette. This bakes simple lighting into vertex colours (COLOR_0), scales
the mesh to metres, and puts its base on y = 0 so it sits on the ground.

Shrink a large CAD export first with gltfpack (no quantization, which the map
loader does not read). terra.glb was made from Zorvane's
simulator/assets/models/crab_trap_bot_sim.glb (millimetres, front -Z) with:

    npx gltfpack@0.24.0 -i crab_trap_bot_sim.glb -o simplified.glb -si 0.2 -sa -noq
    scripts/prepare_rover_model.py simplified.glb crates/terra-map/src/assets/terra.glb \\
        --unit 0.001 --name "Terra (CrabTrapBot)"

Input: one mesh with POSITION and NORMAL as float accessors, glTF Y-up with
the rover's front along -Z (what Zorvane uses). Needs numpy.
"""
import argparse
import json
import struct

import numpy as np

GLB_MAGIC = 0x46546C67
JSON_CHUNK = 0x4E4F534A
BIN_CHUNK = 0x004E4942
# Light from above, a little to the right and from behind the chase camera.
LIGHT = np.array([0.35, 0.85, 0.40])


def read_glb(path):
    data = open(path, "rb").read()
    magic, _, _ = struct.unpack_from("<III", data, 0)
    assert magic == GLB_MAGIC, "not a .glb file"
    offset, doc, binary = 12, None, b""
    while offset < len(data):
        length, kind = struct.unpack_from("<II", data, offset)
        chunk = data[offset + 8 : offset + 8 + length]
        if kind == JSON_CHUNK:
            doc = json.loads(chunk)
        elif kind == BIN_CHUNK:
            binary = chunk
        offset += 8 + length
    return doc, bytearray(binary)


def write_glb(path, doc, binary):
    text = json.dumps(doc, separators=(",", ":")).encode()
    text += b" " * (-len(text) % 4)
    binary = bytes(binary) + b"\0" * (-len(binary) % 4)
    total = 12 + 8 + len(text) + 8 + len(binary)
    with open(path, "wb") as out:
        out.write(struct.pack("<III", GLB_MAGIC, 2, total))
        out.write(struct.pack("<II", len(text), JSON_CHUNK) + text)
        out.write(struct.pack("<II", len(binary), BIN_CHUNK) + binary)


def read_vec3(doc, binary, index):
    accessor = doc["accessors"][index]
    assert accessor["componentType"] == 5126 and accessor["type"] == "VEC3", "need float VEC3"
    view = doc["bufferViews"][accessor["bufferView"]]
    stride = view.get("byteStride", 12)
    start = view.get("byteOffset", 0) + accessor.get("byteOffset", 0)
    raw = np.frombuffer(binary, dtype=np.uint8, count=stride * (accessor["count"] - 1) + 12, offset=start)
    rows = np.lib.stride_tricks.as_strided(raw, shape=(accessor["count"], 12), strides=(stride, 1))
    return rows.copy().view(np.float32).reshape(-1, 3).astype(np.float64)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("input")
    parser.add_argument("output")
    parser.add_argument("--unit", type=float, default=1.0, help="metres per model unit (0.001 for millimetres)")
    parser.add_argument("--name", default="Rover")
    args = parser.parse_args()

    doc, binary = read_glb(args.input)
    assert len(doc["meshes"]) == 1 and len(doc["nodes"]) == 1, "expected one mesh and one node"
    material = doc["materials"][0] if doc.get("materials") else {}
    base = np.array(material.get("pbrMetallicRoughness", {}).get("baseColorFactor", [0.6, 0.6, 0.6, 1])[:3])
    light = LIGHT / np.linalg.norm(LIGHT)
    lowest = None
    for primitive in doc["meshes"][0]["primitives"]:
        attributes = primitive["attributes"]
        positions = read_vec3(doc, binary, attributes["POSITION"])
        normals = read_vec3(doc, binary, attributes["NORMAL"])
        normals /= np.maximum(np.linalg.norm(normals, axis=1, keepdims=True), 1e-9)
        # Lambert key light, plus a sky fill so faces turned away are not black.
        key = np.clip(normals @ light, 0.0, 1.0)
        sky = 0.5 + 0.5 * normals[:, 1]
        shade = 0.28 + 0.55 * key + 0.25 * sky
        colors = np.clip(base[None, :] * shade[:, None], 0.0, 1.0)
        rgba = np.hstack([colors, np.ones((len(colors), 1))]).astype(np.float32)
        binary += b"\0" * (-len(binary) % 4)
        doc["bufferViews"].append({"buffer": 0, "byteOffset": len(binary), "byteLength": rgba.nbytes})
        binary += rgba.tobytes()
        doc["accessors"].append({
            "bufferView": len(doc["bufferViews"]) - 1,
            "componentType": 5126,
            "count": len(rgba),
            "type": "VEC4",
        })
        attributes["COLOR_0"] = len(doc["accessors"]) - 1
        low = positions[:, 1].min()
        lowest = low if lowest is None else min(lowest, low)
    # Vertex colours carry the shading; keep the material white so they show.
    for material in doc.get("materials", []):
        material.setdefault("pbrMetallicRoughness", {})["baseColorFactor"] = [1, 1, 1, 1]
    doc["buffers"][0]["byteLength"] = len(binary)
    node = doc["nodes"][0]
    node["name"] = f"{args.name}, front -Z, metres"
    node["scale"] = [args.unit] * 3
    node["translation"] = [0.0, -lowest * args.unit, 0.0]
    doc["asset"]["extras"] = {"made_with": "scripts/prepare_rover_model.py", "unit": args.unit}
    write_glb(args.output, doc, binary)
    print(f"{args.output}: {sum(len(read_vec3(doc, binary, p['attributes']['POSITION'])) for p in doc['meshes'][0]['primitives'])} vertices")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Fail when a capture contains a large area of pure magenta (#FF00FF).

Unity draws Hidden/InternalErrorShader as that color. A missing occupancy
shader then fills the rover cluster and the Mac script would still pass.
"""

import struct
import sys
import zlib

DEFAULT_LIMIT = 5000


def magenta_pixels(path):
    data = open(path, "rb").read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError(path + " is not a PNG")
    pos = 8
    width = height = color = interlace = None
    idat = bytearray()
    while pos + 8 <= len(data):
        length = struct.unpack(">I", data[pos : pos + 4])[0]
        kind = data[pos + 4 : pos + 8]
        chunk = data[pos + 8 : pos + 8 + length]
        pos += 12 + length
        if kind == b"IHDR":
            width, height, bit, color, _comp, _filt, interlace = struct.unpack(">IIBBBBB", chunk)
            if bit != 8 or color not in (2, 6) or interlace != 0:
                raise ValueError(path + " is not an 8-bit RGB PNG")
        elif kind == b"IDAT":
            idat.extend(chunk)
        elif kind == b"IEND":
            break
    if width is None:
        raise ValueError(path + " has no IHDR")
    channels = 3 if color == 2 else 4
    raw = zlib.decompress(bytes(idat))
    stride = width * channels
    rows = []
    cursor = 0
    for _y in range(height):
        filt = raw[cursor]
        cursor += 1
        row = bytearray(raw[cursor : cursor + stride])
        cursor += stride
        prior = rows[-1] if rows else bytearray(stride)
        if filt == 0:
            pass
        elif filt == 1:
            for i in range(stride):
                left = row[i - channels] if i >= channels else 0
                row[i] = (row[i] + left) & 255
        elif filt == 2:
            for i in range(stride):
                row[i] = (row[i] + prior[i]) & 255
        elif filt == 3:
            for i in range(stride):
                left = row[i - channels] if i >= channels else 0
                row[i] = (row[i] + ((left + prior[i]) // 2)) & 255
        elif filt == 4:
            for i in range(stride):
                left = row[i - channels] if i >= channels else 0
                up = prior[i]
                up_left = prior[i - channels] if i >= channels else 0
                row[i] = (row[i] + _paeth(left, up, up_left)) & 255
        else:
            raise ValueError(path + " has an unknown PNG filter")
        rows.append(row)
    count = 0
    for row in rows:
        for x in range(0, stride, channels):
            if row[x] == 255 and row[x + 1] == 0 and row[x + 2] == 255:
                count += 1
    return count, width * height


def _paeth(left, up, up_left):
    estimate = left + up - up_left
    dist_left = abs(estimate - left)
    dist_up = abs(estimate - up)
    dist_up_left = abs(estimate - up_left)
    if dist_left <= dist_up and dist_left <= dist_up_left:
        return left
    if dist_up <= dist_up_left:
        return up
    return up_left


def main(argv):
    limit = DEFAULT_LIMIT
    paths = []
    for arg in argv:
        if arg.startswith("--limit="):
            limit = int(arg.split("=", 1)[1])
        else:
            paths.append(arg)
    if not paths:
        print("usage: check-magenta.py [--limit=N] capture.png ...", file=sys.stderr)
        return 2
    failed = False
    for path in paths:
        count, total = magenta_pixels(path)
        print(f"{path}: pure #FF00FF pixels {count} of {total}", flush=True)
        if count > limit:
            print(
                f"FAIL: {path} has {count} pure magenta pixels (limit {limit}). "
                "A player shader resolved to Hidden/InternalErrorShader.",
                file=sys.stderr,
            )
            failed = True
    return 1 if failed else 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except (OSError, ValueError, zlib.error) as error:
        print("FAIL: " + str(error), file=sys.stderr)
        sys.exit(2)

# /// script
# requires-python = ">=3.12"
# dependencies = []
# ///
"""Generate a transparent Xcursor theme so the kiosk shows no mouse cursor."""

import os
import struct

ROOT = "/tmp/opencode/blank-cursor"
CURSORS = os.path.join(ROOT, "blank", "cursors")
os.makedirs(CURSORS, exist_ok=True)

MAGIC = 0x72756358  # "Xcur"
IMAGE_TYPE = 0xFFFD0002
HEADER_SIZE = 16
IMAGE_HEADER_SIZE = 36


def xcursor(width: int, height: int) -> bytes:
    """A single-image, fully transparent Xcursor file."""
    pixels = b"\x00\x00\x00\x00" * (width * height)
    # TOC: one entry; image chunk starts right after header + toc.
    toc_offset = HEADER_SIZE + 12
    header = struct.pack("<IIII", MAGIC, HEADER_SIZE, 0x00010000, 1)
    toc = struct.pack("<III", IMAGE_TYPE, width, toc_offset)
    image = struct.pack(
        "<IIIIIIIII",
        IMAGE_HEADER_SIZE,
        IMAGE_TYPE,
        width,
        1,  # version
        width,
        height,
        0,  # xhot
        0,  # yhot
        0,  # delay
    )
    return header + toc + image + pixels


for name in ("left_ptr", "default", "pointer", "text", "crosshair", "hand2", "top_left_arrow"):
    with open(os.path.join(CURSORS, name), "wb") as f:
        f.write(xcursor(1, 1))

with open(os.path.join(ROOT, "blank", "index.theme"), "w") as f:
    f.write("[Icon Theme]\nName=blank\nComment=Transparent cursor\n")

print("wrote", CURSORS)

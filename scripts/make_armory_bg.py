#!/usr/bin/env python3
"""The Armory's purple background (private/export/ui/armory/textures/armory_bg.png).

The live game draws it behind ShopNGUI at run time; it is not in the install's NGUI data.
OBSERVED from the median of 36 Armory screenshots (1280x720): brightest violet
[100,60,235] around (1000,200) px, darker violet [40,30,135] further out, and a dark
navy band [3,18,60] along the bottom edge.
"""
import pathlib

import numpy as np
from PIL import Image

OUT = pathlib.Path(__file__).resolve().parent.parent / "private/export/ui/armory/textures/armory_bg.png"
W, H = 320, 180
y, x = np.mgrid[0:H, 0:W].astype(np.float32)
px, py = x * 4, y * 4
r = np.hypot(px - 1000, (py - 200) * 1.3) / 750
inner, outer, navy = np.array([100, 60, 235]), np.array([40, 30, 135]), np.array([3, 18, 60])
t = np.clip(r, 0, 1)[..., None]
c = inner * (1 - t) + outer * t
b = np.clip((py - 560) / 150, 0, 1)[..., None]
c = c * (1 - b) + navy * b
OUT.parent.mkdir(parents=True, exist_ok=True)
Image.fromarray(c.astype(np.uint8)).save(OUT)
print(f"armory background -> {OUT}")

#!/usr/bin/env python3
"""Export the player skins (64x32, the blocky body's texture layout) for bots to wear.

resources.assets and the sharedassets files hold the stock skins: "Skins Shablons/shablon_00..15" (the skin store's
templates), newskin_N and the *Sett / *_skin / league_skin_* skins of the old skin shop
(SkinsController keeps bought ones by these names), plus "Multiplayer Skins/multi_skin_1".
Only textures whose drawn area matches the body texture ("player") are kept, so icons and
enemy maps of the same size fall out.

Writes private/export/characters/skins/<name>.png and skins.json (the list).
tools/venv/bin/python scripts/export_skins.py
"""
import glob
import json
import pathlib
import re

import numpy as np
import UnityPy
import gamepaths

ROOT = pathlib.Path(__file__).resolve().parent.parent
GAME = str(gamepaths.DATA)
OUT = ROOT / "private/export/characters/skins"
NAMES = re.compile(r"^(shablon_\d+|newskin_\d+|\w+Sett|\w+_skin|league_skin_\w+|multi_skin_\d+|skin810_\d+|easter_skin\d)$")


def scan(env, texs):
    for o in env.objects:
        if o.type.name != "Texture2D":
            continue
        t = o.read()
        if (t.m_Width, t.m_Height) == (64, 32) and (t.m_Name == "player" or NAMES.match(t.m_Name)):
            texs.setdefault(t.m_Name, t)


def main():
    texs = {}
    for f in [f"{GAME}/resources.assets", *sorted(glob.glob(f"{GAME}/sharedassets*.assets"))]:
        scan(UnityPy.load(f), texs)
    ref = np.asarray(texs.pop("player").image.convert("RGBA"))[..., 3] > 8
    OUT.mkdir(parents=True, exist_ok=True)
    kept = []
    for name, t in sorted(texs.items()):
        img = t.image.convert("RGBA")
        a = np.asarray(img)[..., 3] > 8
        iou = (a & ref).sum() / max((a | ref).sum(), 1)
        if a.all():
            iou = 1.0  # fully opaque skins (no transparent padding) are still body textures
        if iou < 0.8:
            print(f"skip {name} (overlap {iou:.2f})")
            continue
        img.save(OUT / f"{name}.png")
        kept.append(name)
    (OUT / "skins.json").write_text(json.dumps(kept, indent=1))
    print(f"{len(kept)} skins -> {OUT}")


if __name__ == "__main__":
    main()

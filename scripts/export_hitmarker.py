#!/usr/bin/env python3
"""The hit marker around the crosshair (AimCrosshairController in resources.assets):
aimHitSprites' textures (hitmarkerTEST1 = hit / kill, hitmarkerTEST_hs = headshot) to
private/export/ui/hitmarker/. Colours, offsets and fade are read off the component and
written into the game (feel.rs); notes in private/notes/aim_crosshair.json.

    tools/venv/bin/python scripts/export_hitmarker.py
"""
import os
import pathlib

import UnityPy
from UnityPy.helpers.TypeTreeGenerator import TypeTreeGenerator

ROOT = pathlib.Path(__file__).resolve().parent.parent
GAME = os.path.expanduser("~/.local/share/Steam/steamapps/common/Pixel Gun 3D PC Edition/Pixel Gun 3D_Data")


def main():
    gen = TypeTreeGenerator("2022.3.76f1")
    gen.load_local_dll_folder(str(ROOT / "private/cpp2il/dummydll"))
    env = UnityPy.load(f"{GAME}/resources.assets")
    env.typetree_generator = gen
    objs = next(iter(env.files.values())).objects
    out = ROOT / "private/export/ui/hitmarker"
    out.mkdir(parents=True, exist_ok=True)
    for o in env.objects:
        if o.type.name != "MonoBehaviour":
            continue
        try:
            d = o.read_typetree()
        except Exception:
            continue
        if "aimHitSprites" not in d or "headshotHitmarkerSprites" not in d:
            continue
        seen = set()
        for key in ("aimHitSprites", "headshotHitmarkerSprites"):
            for p in d[key]:
                t = objs[p["m_PathID"]].read_typetree().get("mTexture")
                if not t or not t["m_PathID"] or t["m_PathID"] in seen:
                    continue
                seen.add(t["m_PathID"])
                img = objs[t["m_PathID"]].read()
                img.image.save(out / f"{img.m_Name}.png")
                print(img.m_Name)
        return


if __name__ == "__main__":
    main()

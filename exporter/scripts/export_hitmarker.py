#!/usr/bin/env python3
"""The hit marker around the crosshair (AimCrosshairController in resources.assets):
aimHitSprites' textures (hitmarkerTEST1 = hit / kill, hitmarkerTEST_hs = headshot) to
private/export/ui/hitmarker/. Colours, offsets and fade are read off the component and
written into the game (feel.rs); notes in private/notes/aim_crosshair.json.

    tools/venv/bin/python scripts/export_hitmarker.py
"""
import pathlib

import UnityPy
import gamepaths

ROOT = pathlib.Path(__file__).resolve().parent.parent
GAME = str(gamepaths.DATA)


def main():
    gen = gamepaths.typetree_generator()
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

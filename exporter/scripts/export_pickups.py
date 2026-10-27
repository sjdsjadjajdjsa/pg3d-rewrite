#!/usr/bin/env python3
"""Export the in-match pickup ("bonus") models and sounds.

    tools/venv/bin/python scripts/export_pickups.py

The game spawns the prefab BonusGeneric (resources.assets) at a map's BonusCreationZone
points and switches on one of its child models (ammo, health, armor, grenade, ...).
Writes private/export/pickups/pickups.gltf (all children, each switched on) and
sounds/<n>.wav (BonusItem.itemSounds, same order as the models).
"""
import json
import pathlib
import sys

import numpy as np

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import export_map as em  # noqa: E402
import export_weapons as ew  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "private/export/pickups"


def main():
    env, _scene, _n = em.load_level_env("level17")
    res = next(f for k, f in env.files.items() if k.endswith("/resources.assets"))
    go = next(o.read() for o in res.objects.values()
              if o.type.name == "GameObject" and o.read().m_Name == "BonusGeneric")
    t = next(em.deref(getattr(c, "component", c)) for c in go.m_Component
             if getattr(c, "component", c).deref().type.name == "Transform")
    ex = em.Exporter(env, OUT)
    ex.vis.name = "pickups"
    ex.force_active = {"ammo", "health", "armor", "grenade", "mech", "Coin"}
    n = ex.walk(t, np.eye(4))
    ex.vis.doc["nodes"][n].update(translation=[0, 0, 0], rotation=[0, 0, 0, 1], scale=[1, 1, 1])
    ex.vis.doc["scenes"][0]["nodes"].append(n)
    ex.col.node(name="empty")
    ex.col.doc["scenes"][0]["nodes"] = [0]
    # pickup sounds from the BonusItem component
    sounds = {}
    ex_info = {}
    for c in go.m_Component:
        r = getattr(c, "component", c).deref()
        if r.type.name != "MonoBehaviour":
            continue
        env.typetree_generator = ew.typetree_generator()
        try:
            d = r.read_typetree()
        except Exception:
            continue
        finally:
            env.typetree_generator = None
        (OUT / "sounds").mkdir(parents=True, exist_ok=True)
        for i, p in enumerate(d.get("itemSounds", [])):
            f = r.assets_file if p["m_FileID"] == 0 else None
            if f is None or p["m_PathID"] not in f.objects:
                continue
            clip = f.objects[p["m_PathID"]].read()
            try:
                samples = clip.samples
            except Exception as e:
                print(f"  sound {clip.m_Name}: {e!r}", file=sys.stderr)
                continue
            for _fname, data in samples.items():
                (OUT / "sounds" / f"{i}.wav").write_bytes(data)
                sounds[i] = clip.m_Name
                break
        names = []
        for p in d.get("itemMdls", []):
            o = r.assets_file.objects.get(p["m_PathID"]) if p["m_FileID"] == 0 else None
            names.append(o.read().m_Name if o is not None else None)
        ex_info = {"models": names, "sounds": sounds}
    ex.save({"pickups": "BonusGeneric", **ex_info})
    print(f"pickups: {json.dumps(ex_info)} {json.dumps(dict(ex.stats))}")


if __name__ == "__main__":
    main()

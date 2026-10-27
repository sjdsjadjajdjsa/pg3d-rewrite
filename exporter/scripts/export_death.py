#!/usr/bin/env python3
"""Export the death effect: the "angel" (Player_Dead_Angel in resources.assets), a white
ghost of the blocky body with wings (material Player_Dead) that plays its one clip,
"Take 001", rising out of where a player died.

    tools/venv/bin/python scripts/export_death.py

Writes private/export/characters/dead_angel/angel.gltf (+ info.json with the clip length).
"""
import json
import pathlib
import sys

import numpy as np

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import export_map as em  # noqa: E402
import export_weapons as ew  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "private/export/characters/dead_angel"
NAME = "Player_Dead_Angel"


def main():
    env, _scene, _n = em.load_level_env("level17")
    res = next(f for k, f in env.files.items() if k.endswith("/resources.assets"))
    prefab = None
    for o in res.objects.values():
        if o.type.name == "GameObject" and o.read().m_Name == NAME:
            t = next(em.deref(getattr(c, "component", c)) for c in o.read().m_Component
                     if getattr(c, "component", c).deref().type.name == "Transform")
            prefab = t  # (a child of the dead-effects holder prefab)
            break
    if prefab is None:
        sys.exit(f"{NAME} not found")
    go = em.deref(prefab.m_GameObject)
    anim = next((em.deref(getattr(c, "component", c)) for c in go.m_Component
                 if getattr(c, "component", c).deref().type.name == "Animation"), None)
    ex = em.Exporter(env, OUT)
    ex.vis.name = "angel"
    ex.force_active = {NAME}
    n = ex.walk(prefab, np.eye(4))
    # the prefab sits 1 m down (its origin is the player's, 1 m above the feet): keep that
    ex.vis.doc["nodes"][n].update(translation=[0, 0, 0], rotation=[0, 0, 0, 1], scale=[1, 1, 1])
    ex.vis.doc["scenes"][0]["nodes"].append(n)
    ex.col.node(name="empty")
    ex.col.doc["scenes"][0]["nodes"] = [0]
    clips = ew.export_animations(ex, env, "", animation=anim, root=n) if anim else []
    lengths = {}
    if anim:
        for p in anim.m_Animations:
            c = em.deref(p)
            if c is not None:
                lengths[c.m_Name] = float(c.m_MuscleClipInfo.m_StopTime) if hasattr(c, "m_MuscleClipInfo") else None
    ex.save({"prefab": NAME, "clips": clips, "lengths": lengths})
    print(f"angel: clips={clips} {lengths} {json.dumps(dict(ex.stats))} -> {OUT}")


if __name__ == "__main__":
    main()

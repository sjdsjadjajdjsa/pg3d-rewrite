#!/usr/bin/env python3
"""Export the default player character (the blocky Pixel Gun body) for the menu and game.

    tools/venv/bin/python scripts/export_character.py

The lobby (level17) shows the player as "MainMenu_Pers_MainMenu", filled at run time with
the prefab Character_model_V2 from resources.assets. We export that prefab with its own
animation clips (Idle first).
Writes private/export/characters/default/character.gltf (+ info.json, textures/).
"""
import json
import pathlib
import sys

import numpy as np
import UnityPy

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import export_map as em  # noqa: E402
import export_weapons as ew  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "private/export/characters/default"


def main():
    env, _scene, _n = em.load_level_env("level17")
    res = next(f for k, f in env.files.items() if k.endswith("/resources.assets"))
    prefab = None
    for o in res.objects.values():
        if o.type.name == "GameObject" and o.read().m_Name == "Character_model_V2":
            t = next(em.deref(getattr(c, "component", c)) for c in o.read().m_Component
                     if getattr(c, "component", c).deref().type.name == "Transform")
            if not t.m_Father.m_PathID:
                prefab = t
                break
    if prefab is None:
        sys.exit("Character_model_V2 not found")
    # the prefab's own Animation (Idle, gliders, shop poses). The lobby object's Idle
    # ("MainMenu_Pers_MainMenu") was made for an older rig: it keys Foot_right at scale
    # -1.588, which on this prefab (Foot_right at scale 1) blows the right leg up and
    # mirrors it.
    go = em.deref(prefab.m_GameObject)
    anim = next((em.deref(getattr(c, "component", c)) for c in go.m_Component
                 if getattr(c, "component", c).deref().type.name == "Animation"), None)
    ex = em.Exporter(env, OUT)
    ex.vis.name = "character"
    ex.skip_names = {"graffiti_killstreak_wall"}  # killstreak effect, shown only in matches
    n = ex.walk(prefab, np.eye(4))
    ex.vis.doc["nodes"][n].update(translation=[0, 0, 0], rotation=[0, 0, 0, 1], scale=[1, 1, 1])
    ex.vis.doc["scenes"][0]["nodes"].append(n)
    ex.col.node(name="empty")
    ex.col.doc["scenes"][0]["nodes"] = [0]
    clips = ew.export_animations(ex, env, "", animation=anim, root=n) if anim else []
    ex.save({"character": "Character_model_V2", "clips": clips,
             "animations": {c: 0 for c in clips}})
    print(f"character: clips={clips} {json.dumps(dict(ex.stats))} -> {OUT}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Print project-wide Unity settings that matter for the rewrite (colour space, physics, time)."""
import UnityPy

import gamepaths

GGM = str(gamepaths.DATA / "globalgamemanagers")

assets = UnityPy.load(GGM)
for o in assets.objects:
    if o.type.name == "PlayerSettings":
        d = o.read_typetree()
        print("PlayerSettings", {k: d[k] for k in d if "olor" in k or "Gamma" in k})
    elif o.type.name == "PhysicsManager":
        d = o.read_typetree()
        print("PhysicsManager", {k: d[k] for k in ("m_Gravity", "m_DefaultContactOffset") if k in d})
    elif o.type.name == "TimeManager":
        print("TimeManager", o.read_typetree())

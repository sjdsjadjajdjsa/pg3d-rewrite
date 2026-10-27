#!/usr/bin/env python3
"""The PC version's post-processing (Unity Post Processing Stack v2) per map.

resources.assets has one "<Scene>_PC" prefab per map with a global PostProcessVolume (child
"PostProcessing") whose profile lists the effects: AmbientOcclusion, Bloom,
ChromaticAberration, ColorGrading, AutoExposure, MotionBlur, LensDistortion.
"DefaultGraphics_PC" serves the maps without their own. The Settings > Graphics check boxes
(SettingsGraphics: Bloom, Motion Blur, Chromatic Aberration, Anti-Aliasing) switch them.

Writes private/export/maps/postprocess.json: {"Default": {"Bloom": {"intensity": ...}, ...},
"Mine": {...}, ...} with each effect's overridden values (the rest are PPv2 defaults).

    tools/venv/bin/python scripts/export_postprocess.py
"""
import json
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

    def tree(pid):
        return objs[pid].read_typetree()

    def root_of(go_pid):
        g = tree(go_pid)
        tr = next((tree(c["component"]["m_PathID"]) for c in g["m_Component"]
                   if objs[c["component"]["m_PathID"]].type.name == "Transform"), None)
        name = g["m_Name"]
        while tr and tr["m_Father"]["m_PathID"]:
            tr = tree(tr["m_Father"]["m_PathID"])
            name = tree(tr["m_GameObject"]["m_PathID"])["m_Name"]
        return name

    out = {}
    for o in env.objects:
        if o.type.name != "MonoBehaviour":
            continue
        try:
            v = o.read_typetree()
        except Exception:
            continue
        if "sharedProfile" not in v or "isGlobal" not in v:
            continue
        prof = v["sharedProfile"]
        if not prof["m_PathID"]:
            continue
        root = root_of(v["m_GameObject"]["m_PathID"])
        key = root[:-3] if root.endswith("_PC") else root
        effects = {}
        for s in tree(prof["m_PathID"]).get("settings", []):
            st = tree(s["m_PathID"])
            name = st.get("m_Name", "?").replace("(Clone)", "").strip()
            vals = {k: x["value"] for k, x in st.items()
                    if isinstance(x, dict) and "overrideState" in x and x["overrideState"]}
            vals["enabled"] = bool(st.get("enabled", {}).get("value", 1))
            vals["active"] = bool(st.get("active", 1))
            effects[name] = vals
        out[key] = effects
    dst = ROOT / "private/export/maps/postprocess.json"
    dst.write_text(json.dumps(out, indent=1, default=float))
    print(f"{len(out)} profiles -> {dst}")


if __name__ == "__main__":
    main()

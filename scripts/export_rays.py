#!/usr/bin/env python3
"""Export the beams of ray weapons (railguns, laser and freeze rays).

A weapon with `railgun` or `freezer` set draws the prefab named by its `railName` from the
"rays" bundle: a LineRenderer ("Ray") stretched from the muzzle to what was hit, faded out
over `timeDeactivate` seconds (RayAndExplosionsStackItem); freezer rays (FreezerRay) stay
on while the trigger is held. Writes private/export/rays/rays.json and textures/.

    tools/venv/bin/python scripts/export_rays.py [Weapon77 Weapon105 ...]
"""
import glob
import json
import os
import pathlib
import sys

import UnityPy

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "private/export/rays"
WEAPONS = ROOT / "private/export/weapons"
INDEX = ROOT / "private/index/bundles.json"


def load_with_deps(bundle):
    idx = json.loads(INDEX.read_text())
    first = glob.glob(f"{idx['root']}/{bundle}/*/*")[0]
    seen, queue = {first}, [first]
    while queue:
        e = UnityPy.load(queue.pop())
        for f in e.files.values():
            for ext in getattr(f, "externals", []) or []:
                rel = idx["cab"].get(ext.path.split("/")[-1])
                if rel and os.path.join(idx["root"], rel) not in seen:
                    seen.add(os.path.join(idx["root"], rel))
                    queue.append(os.path.join(idx["root"], rel))
            for sub in getattr(f, "files", {}).values():
                for ext in getattr(sub, "externals", []) or []:
                    rel = idx["cab"].get(ext.path.split("/")[-1])
                    if rel and os.path.join(idx["root"], rel) not in seen:
                        seen.add(os.path.join(idx["root"], rel))
                        queue.append(os.path.join(idx["root"], rel))
    return UnityPy.load(*sorted(seen))


def color(c):
    return [round(c.r, 4), round(c.g, 4), round(c.b, 4), round(c.a, 4)]


def main():
    weapons = json.loads((WEAPONS / "weapons.json").read_text())
    names = sys.argv[1:] or sorted(n for n, w in weapons.items()
                                   if (w.get("railgun") or w.get("freezer")) and (WEAPONS / n / "weapon.gltf").exists())
    wanted = {weapons[n].get("railName") or n: n for n in names}
    env = load_with_deps("rays")
    prefabs = {}
    for o in env.objects:
        if o.type.name == "AssetBundle":
            for k, v in o.read().m_Container:
                prefabs[k.split("/")[-1].replace(".prefab", "").lower()] = v.asset
    (OUT / "textures").mkdir(parents=True, exist_ok=True)
    out = {}
    for rail, weapon in wanted.items():
        ptr = prefabs.get(rail.lower())
        if ptr is None:
            print(f"no ray prefab {rail} for {weapon}", file=sys.stderr)
            continue
        go = ptr.read()
        ray = {"weapon": weapon, "life": 0.3}
        stack = [go]
        while stack:
            g = stack.pop()
            for c in g.m_Component:
                r = getattr(c, "component", c).deref()
                if r.type.name == "Transform":
                    for ch in r.read().m_Children:
                        stack.append(ch.read().m_GameObject.read())
                elif r.type.name == "MonoBehaviour":
                    try:
                        d = r.read_typetree()
                    except Exception:
                        continue
                    if "timeDeactivate" in d:
                        ray["life"] = d["timeDeactivate"]
                    if "lifetime" in d:
                        ray["freezer_life"] = d["lifetime"]
                elif r.type.name == "LineRenderer":
                    lr = r.read_typetree()
                    p = lr["m_Parameters"]
                    ray["width"] = p.get("widthMultiplier", 1.0)
                    ray["width_curve"] = [[k["time"], k["value"]] for k in p["widthCurve"]["m_Curve"]]
                    g_ = p["colorGradient"]
                    ray["colors"] = [[g_[f"ctime{i}"] / 65535, *[round(g_[f"key{i}"][ch], 4) for ch in "rgb"]]
                                     for i in range(g_["m_NumColorKeys"])]
                    ray["alphas"] = [[g_[f"atime{i}"] / 65535, round(g_[f"key{i}"]["a"], 4)]
                                     for i in range(g_["m_NumAlphaKeys"])]
                    ray["texture_mode"] = p.get("textureMode", 0)
                    mats = [m for m in r.read().m_Materials if m.m_PathID]
                    if mats:
                        try:
                            m = mats[0].read()
                        except Exception as e:
                            print(f"  {weapon} material: {e!r}", file=sys.stderr)
                            m = None
                        if m is not None:
                            ray["material"] = m.m_Name
                            try:
                                sh = m.m_Shader.read()
                                ray["shader"] = sh.m_ParsedForm.m_Name if hasattr(sh, "m_ParsedForm") else sh.m_Name
                            except Exception:
                                ray["shader"] = ""
                            ray["mat_colors"] = {(k if isinstance(k, str) else k.name): color(v) for k, v in m.m_SavedProperties.m_Colors}
                            for k, te in m.m_SavedProperties.m_TexEnvs:
                                kn = k if isinstance(k, str) else k.name
                                if kn == "_MainTex" and te.m_Texture.m_PathID:
                                    t = te.m_Texture.read()
                                    png = f"textures/{t.m_Name}.png"
                                    t.image.save(OUT / png)
                                    ray["texture"] = png
                                    ray["tiling"] = [te.m_Scale.x, te.m_Scale.y]
        out[weapon] = ray
        print(f"ray {weapon} ({rail}): {json.dumps(ray)[:300]}")
    (OUT / "rays.json").write_text(json.dumps(out, indent=1))
    print(f"rays: {len(out)} -> {OUT}")


if __name__ == "__main__":
    main()

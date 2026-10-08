#!/usr/bin/env python3
"""Write each weapon's first-person clips (Idle, Shoot, Shoot1.., Reload) as
weapons/<name>/clips.json, in the format of profile.json (export_profiles.py): per clip
{"length", "fps", "tracks": [{"path", "rotation", "translation", "scale"}]}, paths starting
below the prefab's "*_Inner" object.

The Armory plays them on the weapon's arms when you hold Shoot (ShopShootingController:
cross-fade 0.05 s from the profile pose to Idle, Animation.Play("Shoot") for every shot,
the Reload clip when the pretend clip runs dry, then Idle and a 0.3 s cross-fade back to
the profile pose). Read from the already exported weapon.gltf (sampled at 30 fps).

    python3 scripts/export_weapon_clips.py [WeaponName ...]
"""
import json
import pathlib
import sys

import numpy as np

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "private/export/weapons"
CLIPS = ("Idle", "Shoot", "Shoot1", "Shoot2", "Reload")
NCOMP = {"SCALAR": 1, "VEC3": 3, "VEC4": 4}


def export(d: pathlib.Path) -> bool:
    g = json.loads((d / "weapon.gltf").read_text())
    anims = [a for a in g.get("animations", []) if a.get("name") in CLIPS]
    if not anims:
        return False
    buf = (d / g["buffers"][0]["uri"]).read_bytes()
    parent = {c: i for i, n in enumerate(g["nodes"]) for c in n.get("children", [])}

    def acc(i):
        a = g["accessors"][i]
        bv = g["bufferViews"][a["bufferView"]]
        n = NCOMP[a["type"]]
        return np.frombuffer(buf, np.float32, a["count"] * n, bv.get("byteOffset", 0) + a.get("byteOffset", 0)).reshape(-1, n)

    def path(i):
        parts = []
        while i is not None and not g["nodes"][i].get("name", "").lower().endswith("_inner"):
            parts.append(g["nodes"][i].get("name", ""))
            i = parent.get(i)
        return "/".join(reversed(parts)) if i is not None else None

    out = {}
    for a in anims:
        tracks, length, fps = {}, 0.0, 30
        for ch in a["channels"]:
            p = path(ch["target"]["node"])
            if p is None:
                continue
            s = a["samplers"][ch["sampler"]]
            t, v = acc(s["input"]), acc(s["output"])
            length = max(length, float(t[-1, 0]))
            if len(t) > 1:
                fps = round(1.0 / float(t[1, 0] - t[0, 0]))
            tracks.setdefault(p, {"path": p})[ch["target"]["path"]] = np.round(v, 5).tolist()
        out[a["name"]] = {"length": round(length, 4), "fps": fps, "tracks": list(tracks.values())}
    (d / "clips.json").write_text(json.dumps(out))
    return True


def main():
    names = sys.argv[1:] or sorted(p.name for p in OUT.iterdir() if (p / "weapon.gltf").exists())
    done = sum(export(OUT / n) for n in names)
    print(f"weapon clips: {done} of {len(names)} weapons -> {OUT}/<name>/clips.json")


if __name__ == "__main__":
    main()

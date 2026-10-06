#!/usr/bin/env python3
"""Export weapon stats (all weapons) and weapon models (selected) for the Bevy rewrite.

    tools/venv/bin/python scripts/export_weapons.py                 # stats + default models
    tools/venv/bin/python scripts/export_weapons.py Weapon2 Weapon9 # stats + these models

Needs private/index/weaponsounds.tsv:
    tools/venv/bin/python scripts/find_behaviours.py WeaponSounds > private/index/weaponsounds.tsv

Writes private/export/weapons/:
    weapons.json            every WeaponSounds prefab: its scalar settings + a few arrays
    <Weapon>/weapon.gltf    first-person model (arms + gun, bind pose, no animation yet)
"""
import collections
import json
import os
import pathlib
import sys

import numpy as np
import UnityPy
from UnityPy.helpers.TypeTreeGenerator import TypeTreeGenerator

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import export_map as em  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
GAME = em.GAME_DATA
INDEX = ROOT / "private/index/weaponsounds.tsv"
OUT = ROOT / "private/export/weapons"
DEFAULT_MODELS = ["Weapon1", "Weapon2", "Weapon3", "Weapon5", "Weapon9", "Weapon11"]
KEEP_LISTS = ("damageByTier", "dpses")
KEEP_DICTS = ("damageRange", "gunPosition", "positionShop", "rotationShop")


def typetree_generator():
    gen = TypeTreeGenerator("2022.3.76f1")
    gen.load_local_dll_folder(str(ROOT / "private/cpp2il/dummydll"))
    return gen


def export_stats(rows, gen):
    by_file = collections.defaultdict(list)
    for path, _cls, go, pid in rows:
        by_file[path].append((go, int(pid)))
    weapons = {}
    for n, (path, items) in enumerate(by_file.items()):
        assets = UnityPy.load(os.path.join(GAME, path))
        assets.typetree_generator = gen
        want = {pid: go for go, pid in items}
        for o in assets.objects:
            if o.path_id not in want:
                continue
            try:
                d = o.read_typetree()
            except Exception as e:
                print(f"  {want[o.path_id]}: {e!r}"[:200], file=sys.stderr)
                continue
            w = {k: v for k, v in d.items() if isinstance(v, (int, float, str)) and k not in ("m_Enabled", "m_Name")}
            w.update({k: d[k] for k in KEEP_LISTS + KEEP_DICTS if k in d})
            w["bundle"] = path.split("/")[-3] if "/bundles/" in path else os.path.basename(path)
            weapons[want[o.path_id]] = w
        print(f"  stats: {n + 1}/{len(by_file)} files, {len(weapons)} weapons", file=sys.stderr)
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "weapons.json").write_text(json.dumps(weapons, indent=0, sort_keys=True))
    print(f"weapons.json: {len(weapons)} weapons")


def export_model(row, idx):
    path, _cls, name, pid = row
    full = os.path.join(GAME, path)
    # load the weapon bundle plus everything it depends on (shared textures, shaders)
    seen, queue = {full}, [full]
    while queue:
        assets = UnityPy.load(queue.pop())
        for sf in em.iter_serialized(assets):
            for ext in sf.externals:
                rel = idx["cab"].get(ext.path.split("/")[-1])
                if rel:
                    p = os.path.join(idx["root"], rel)
                    if p not in seen:
                        seen.add(p)
                        queue.append(p)
    assets = UnityPy.load(*sorted(seen))
    mb = next((o for o in assets.objects if o.path_id == int(pid) and o.type.name == "MonoBehaviour"
               and os.path.abspath(getattr(getattr(o.assets_file, "parent", None), "path", "") or "") == os.path.abspath(full)), None)
    if mb is None:
        mb = next(o for o in assets.objects if o.path_id == int(pid))
    go = mb.read(check_read=False).m_GameObject.read()
    out = OUT / name
    ex = em.Exporter(assets, out)
    ex.vis.name = "weapon"
    root_t = next(em.deref(getattr(c, "component", c)) for c in go.m_Component
                  if getattr(c, "component", c).deref().type.name == "Transform")
    n = ex.walk(root_t, np.eye(4))
    # the prefab root's own placement is decided by the game, not the prefab
    node = ex.vis.doc["nodes"][n]
    node.update(translation=[0, 0, 0], rotation=[0, 0, 0, 1], scale=[1, 1, 1])
    ex.vis.doc["scenes"][0]["nodes"].append(n)
    if not ex.col.doc["nodes"]:
        ex.col.node(name="empty")
    ex.col.doc["scenes"][0]["nodes"] = list(range(len(ex.col.doc["nodes"])))
    ex.save({"weapon": name, "animations": animation_lengths(assets, f"{name}_Inner")})
    print(f"model {name}: {json.dumps(dict(ex.stats))}")


def clip_length(clip):
    """Length in seconds of a legacy AnimationClip = last keyframe time over all curves."""
    length = 0.0
    for k in ("m_RotationCurves", "m_PositionCurves", "m_ScaleCurves", "m_EulerCurves", "m_FloatCurves"):
        for cv in getattr(clip, k, None) or []:
            keys = cv.curve.m_Curve
            if keys:
                length = max(length, keys[-1].time)
    return length


def animation_lengths(assets, inner_name):
    """Clip name -> seconds for the weapon's first-person Animation component. In the
    original, the Shoot clip length sets the fire interval and Reload the reload time."""
    for o in assets.objects:
        if o.type.name != "Animation":
            continue
        anim = o.read()
        go = em.deref(anim.m_GameObject)
        if go is None or go.m_Name != inner_name:
            continue
        out = {}
        for p in anim.m_Animations:
            clip = em.deref(p)
            if clip is not None:
                out[clip.m_Name] = round(clip_length(clip), 4)
        return out
    return {}


def main():
    names = sys.argv[1:] or DEFAULT_MODELS
    rows = [line.rstrip("\n").split("\t") for line in open(INDEX, encoding="utf-8")]
    rows = [r for r in rows if r[1] == "WeaponSounds"]
    export_stats(rows, typetree_generator())
    idx = json.loads(em.INDEX.read_text())
    by_name = {}
    for r in rows:
        by_name.setdefault(r[2], r)
    for n in names:
        if n in by_name:
            export_model(by_name[n], idx)
        else:
            print(f"no weapon named {n}", file=sys.stderr)


if __name__ == "__main__":
    main()

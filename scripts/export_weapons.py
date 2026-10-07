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


def export_prefab(row, idx, out, gltf_name, anim_holder, skip=()):
    """Export the GameObject owning MonoBehaviour `row` (index.tsv row) as <out>/<gltf_name>.gltf,
    with skins and the legacy animations of the child called `anim_holder`.
    Returns (exporter, animation clip names)."""
    path, _cls, _name, pid = row
    full = os.path.join(GAME, path)
    # load the bundle plus everything it depends on (shared meshes, textures, clips)
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
    ex = em.Exporter(assets, out)
    ex.vis.name = gltf_name
    ex.skip_names = set(skip)
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
    clips = export_animations(ex, assets, anim_holder)
    ex.sounds = export_sounds(assets, go, out)
    return ex, assets, clips


def export_model(row, idx):
    name = row[2]
    ex, assets, clips = export_prefab(row, idx, OUT / name, "weapon", f"{name}_Inner")
    ex.save({"weapon": name, "animations": animation_lengths(assets, f"{name}_Inner"), "clips": clips,
             "sounds": ex.sounds})
    print(f"model {name}: {json.dumps(dict(ex.stats))}")


def export_sounds(assets, go, out):
    """Find AudioClips referenced by the prefab's scripts / AudioSources and write them as
    <out>/sounds/<field>.wav. Returns {field name: relative file}. The field names are the
    original ones (e.g. shoot, reload, attackSound, deathSound)."""
    gen = typetree_generator()
    assets.typetree_generator = gen
    files = {f.name: f for f in em.iter_serialized(assets)}
    found = {}

    def resolve(sf, ptr):
        fid, pid = ptr["m_FileID"], ptr["m_PathID"]
        target = sf if fid == 0 else files.get(sf.externals[fid - 1].path.split("/")[-1]) if fid <= len(sf.externals) else None
        if target is None or pid not in target.objects:
            return None
        return target.objects[pid]

    def scan(reader):
        try:
            tree = reader.read_typetree()
        except Exception:
            return
        sf = reader.assets_file

        def walk(x, field):
            if isinstance(x, dict):
                if set(x) == {"m_FileID", "m_PathID"} and x["m_PathID"]:
                    ob = resolve(sf, x)
                    if ob is not None and ob.type.name == "AudioClip" and field not in found:
                        found[field] = ob
                    return
                for k, v in x.items():
                    walk(v, k)
            elif isinstance(x, list):
                for i, v in enumerate(x):
                    walk(v, f"{field}{i}" if len(x) > 1 else field)
        walk(tree, "")

    def visit(g):
        for c in g.m_Component:
            ptr = getattr(c, "component", c)
            try:
                r = ptr.deref()
            except Exception:
                continue
            if r.type.name in ("MonoBehaviour", "AudioSource"):
                scan(r)
            if r.type.name == "Transform":
                for ch in r.read().m_Children:
                    t = em.deref(ch)
                    if t is not None and em.deref(t.m_GameObject) is not None:
                        visit(t.m_GameObject.read())
    visit(go)
    result = {}
    (out / "sounds").mkdir(parents=True, exist_ok=True)
    for field, ob in found.items():
        try:
            clip = ob.read()
            for _name, data in clip.samples.items():
                rel = f"sounds/{em.safe(field)}.wav"
                (out / rel).write_bytes(data)
                result[field] = rel
                break
        except Exception as e:
            print(f"  sound {field}: {e!r}"[:200], file=sys.stderr)
    return result


FPS = 30.0


def _vals(v, n):
    return [v.x, v.y, v.z, v.w][:n] if n == 4 else [v.x, v.y, v.z]


def sample_curve(keys, times, n):
    """Evaluate a Unity AnimationCurve (cubic Hermite keys) at `times`. Returns (len, n)."""
    t = np.array([k.time for k in keys])
    p = np.array([_vals(k.value, n) for k in keys], dtype=np.float64)
    vin = np.array([_vals(k.inSlope, n) for k in keys], dtype=np.float64)
    vout = np.array([_vals(k.outSlope, n) for k in keys], dtype=np.float64)
    out = np.empty((len(times), n))
    for i, x in enumerate(times):
        if x <= t[0] or len(t) == 1:
            out[i] = p[0]
            continue
        if x >= t[-1]:
            out[i] = p[-1]
            continue
        j = int(np.searchsorted(t, x, side="right")) - 1
        dt = t[j + 1] - t[j]
        u = (x - t[j]) / dt
        m0, m1 = vout[j] * dt, vin[j + 1] * dt
        h00, h10, h01, h11 = 2*u**3 - 3*u**2 + 1, u**3 - 2*u**2 + u, -2*u**3 + 3*u**2, u**3 - u**2
        v = h00 * p[j] + h10 * m0 + h01 * p[j + 1] + h11 * m1
        # infinite slopes mean "stepped" keys in Unity
        step = ~np.isfinite(m0) | ~np.isfinite(m1)
        out[i] = np.where(step, p[j], v)
    return out


def euler_to_quat(deg):
    """Unity euler angles (degrees, applied Z then X then Y) -> quaternions (x, y, z, w)."""
    r = np.radians(deg) / 2
    cx, cy, cz = np.cos(r[:, 0]), np.cos(r[:, 1]), np.cos(r[:, 2])
    sx, sy, sz = np.sin(r[:, 0]), np.sin(r[:, 1]), np.sin(r[:, 2])
    # q = qy * qx * qz
    w = cy * cx * cz + sy * sx * sz
    x = cy * sx * cz + sy * cx * sz
    y = sy * cx * cz - cy * sx * sz
    z = cy * cx * sz - sy * sx * cz
    return np.stack([x, y, z, w], axis=1)


def export_animations(ex, assets, inner_name):
    """Convert the legacy Animation clips on `inner_name` into glTF animations."""
    # several prefabs can contain an object with this name: take the one we exported
    anim, root = None, None
    for o in assets.objects:
        if o.type.name != "Animation":
            continue
        a = o.read()
        go = em.deref(a.m_GameObject)
        if go is None or go.m_Name != inner_name:
            continue
        t = next(em.deref(getattr(c, "component", c)) for c in go.m_Component
                 if getattr(c, "component", c).deref().type.name == "Transform")
        root = ex.node_by_transform.get(em.obj_key(t))
        if root is not None:
            anim = (a, go)
            break
    if anim is None:
        return []
    a, go = anim
    doc = ex.vis.doc

    def resolve(path):
        node = root
        for part in [x for x in path.split("/") if x]:
            kids = doc["nodes"][node].get("children", [])
            hit = next((k for k in kids if doc["nodes"][k]["name"] == part), None)
            if hit is None:
                hit = next((k for k in kids if doc["nodes"][k]["name"].lower() == part.lower()), None)
            if hit is None:
                return None
            node = hit
        return node

    names = []
    for ptr in a.m_Animations:
        clip = em.deref(ptr)
        if clip is None:
            continue
        length = clip_length(clip)
        times = np.arange(0.0, length + 1e-6, 1.0 / FPS)
        if len(times) == 0 or times[-1] < length - 1e-4:
            times = np.append(times, length)
        tacc = ex.vis.accessor(times.astype(np.float32), "SCALAR", minmax=True)
        samplers, channels = [], []

        def add(node, path, values, kind):
            samplers.append({"input": tacc, "output": ex.vis.accessor(values.astype(np.float32), kind),
                             "interpolation": "LINEAR"})
            channels.append({"sampler": len(samplers) - 1, "target": {"node": node, "path": path}})

        for cv in clip.m_RotationCurves or []:
            node = resolve(cv.path)
            if node is not None and cv.curve.m_Curve:
                q = sample_curve(cv.curve.m_Curve, times, 4)
                q /= np.maximum(np.linalg.norm(q, axis=1, keepdims=True), 1e-9)
                add(node, "rotation", q * np.array([1, -1, -1, 1]), "VEC4")
        for cv in getattr(clip, "m_EulerCurves", None) or []:
            node = resolve(cv.path)
            if node is not None and cv.curve.m_Curve:
                q = euler_to_quat(sample_curve(cv.curve.m_Curve, times, 3))
                add(node, "rotation", q * np.array([1, -1, -1, 1]), "VEC4")
        for cv in clip.m_PositionCurves or []:
            node = resolve(cv.path)
            if node is not None and cv.curve.m_Curve:
                add(node, "translation", sample_curve(cv.curve.m_Curve, times, 3) * np.array([-1, 1, 1]), "VEC3")
        for cv in clip.m_ScaleCurves or []:
            node = resolve(cv.path)
            if node is not None and cv.curve.m_Curve:
                add(node, "scale", sample_curve(cv.curve.m_Curve, times, 3), "VEC3")
        if channels:
            doc.setdefault("animations", []).append({"name": clip.m_Name, "samplers": samplers, "channels": channels})
            names.append(clip.m_Name)
    return names


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

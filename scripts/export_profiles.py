#!/usr/bin/env python3
"""Export each weapon's Armory pose ("profile" animation) as weapons/<name>/profile.json.

The Armory character holds the weapon prefab at its gun point and plays the weapon's
profile clip on the prefab's arms (FPS_PLAYER_Arm_Left/Right and the gun bones under
them): WeaponSounds.profileAnimationPath ("WeaponProfiles/Weapon2_profileanim") names a
WeaponProfileAnimationObject asset (assets/editor/resources/weaponprofiles/...asset, in
the bundles listed in private/index/bundle_containers.tsv, made by bundle_index.py) whose WeaponProfileAnimation field is a legacy AnimationClip.

profile.json: {"length": s, "fps": n, "tracks": [{"path": clip path, "rotation": [[x,y,z,w]..],
"translation": [[x,y,z]..], "scale": [..]}]} in glTF axes (same conversion as the weapon
animations in weapon.gltf). Paths start at the arms object, named like the prefab's with
" 1" appended (an instantiated copy)."""
import json, os, pathlib, sys

import numpy as np
import UnityPy

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import export_map as em  # noqa: E402
import export_weapons as ew  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "private/export/weapons"
FPS = 30


def clip_tracks(clip):
    length = ew.clip_length(clip)
    times = np.arange(0.0, length + 1e-6, 1.0 / FPS)
    if len(times) == 0 or times[-1] < length - 1e-4:
        times = np.append(times, length)
    tracks = {}
    t = lambda p: tracks.setdefault(p, {"path": p})
    for cv in clip.m_RotationCurves or []:
        if cv.curve.m_Curve:
            q = ew.sample_curve(cv.curve.m_Curve, times, 4)
            q /= np.maximum(np.linalg.norm(q, axis=1, keepdims=True), 1e-9)
            t(cv.path)["rotation"] = np.round(q * np.array([1, -1, -1, 1]), 5).tolist()
    for cv in getattr(clip, "m_CompressedRotationCurves", None) or []:
        keys = ew.compressed_rotation_keys(cv)
        if keys:
            q = ew.sample_curve(keys, times, 4)
            q /= np.maximum(np.linalg.norm(q, axis=1, keepdims=True), 1e-9)
            t(cv.m_Path)["rotation"] = np.round(q * np.array([1, -1, -1, 1]), 5).tolist()
    for cv in getattr(clip, "m_EulerCurves", None) or []:
        if cv.curve.m_Curve:
            q = ew.euler_to_quat(ew.sample_curve(cv.curve.m_Curve, times, 3))
            t(cv.path)["rotation"] = np.round(q * np.array([1, -1, -1, 1]), 5).tolist()
    for cv in clip.m_PositionCurves or []:
        if cv.curve.m_Curve:
            t(cv.path)["translation"] = np.round(ew.sample_curve(cv.curve.m_Curve, times, 3) * np.array([-1, 1, 1]), 5).tolist()
    for cv in clip.m_ScaleCurves or []:
        if cv.curve.m_Curve:
            t(cv.path)["scale"] = np.round(ew.sample_curve(cv.curve.m_Curve, times, 3), 5).tolist()
    return {"length": round(float(length), 4), "fps": FPS, "tracks": list(tracks.values())}


def main():
    weapons = json.load(open(OUT / "weapons.json"))
    idx = json.loads(em.INDEX.read_text())
    where = {}
    for line in open(ROOT / "private/index/bundle_containers.tsv", encoding="utf-8"):
        b, key = line.rstrip("\n").split("\t")
        where[key] = b
    names = sys.argv[1:] or sorted(n for n in os.listdir(OUT) if (OUT / n / "weapon.gltf").exists())
    by_bundle = {}
    for n in names:
        p = weapons.get(n, {}).get("profileAnimationPath")
        key = f"assets/editor/resources/{p.lower()}.asset" if p else None
        if key in where:
            by_bundle.setdefault(where[key], []).append((n, key))
        else:
            print(f"  {n}: no profile animation ({p!r})", file=sys.stderr)
    gen = ew.typetree_generator()
    done = 0
    for b, items in by_bundle.items():
        full = os.path.join(idx["root"], b)
        seen, queue = {full}, [full]
        while queue:
            a = UnityPy.load(queue.pop())
            for sf in em.iter_serialized(a):
                for ext in sf.externals:
                    rel = idx["cab"].get(ext.path.split("/")[-1])
                    if rel and os.path.join(idx["root"], rel) not in seen:
                        seen.add(os.path.join(idx["root"], rel))
                        queue.append(os.path.join(idx["root"], rel))
        env = UnityPy.load(*sorted(seen))
        env.typetree_generator = gen
        container = {}
        for o in env.objects:
            if o.type.name == "AssetBundle":
                for k, v in o.read().m_Container:
                    container[k] = v.asset
        for n, key in items:
            ptr = container.get(key)
            obj = ptr.deref() if ptr is not None else None
            if obj is None:
                print(f"  {n}: {key} not in {b}", file=sys.stderr)
                continue
            d = obj.read_typetree()
            cp = d.get("WeaponProfileAnimation") or {}
            clip = None
            if cp.get("m_PathID"):
                sf = obj.assets_file
                clip = em.resolve(sf, cp) if hasattr(em, "resolve") else None
                if clip is None:
                    from UnityPy.classes import PPtr  # noqa: F401
                    fid, pid = cp["m_FileID"], cp["m_PathID"]
                    target = sf if fid == 0 else sf.environment.get_cab(sf.externals[fid - 1].path.split("/")[-1])
                    clip = target.objects[pid].read() if target else None
            if clip is None:
                print(f"  {n}: no clip", file=sys.stderr)
                continue
            if not hasattr(clip, "m_RotationCurves"):
                clip = clip.read()
            (OUT / n / "profile.json").write_text(json.dumps(clip_tracks(clip)))
            done += 1
    print(f"profiles: {done} weapons -> {OUT}/<name>/profile.json")


if __name__ == "__main__":
    main()

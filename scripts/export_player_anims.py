#!/usr/bin/env python3
"""Export the in-match player body (Royale_Player_body: the skinned model other players are
drawn with) as characters/player_body/body.gltf, and its third-person animations
(RoyalePlayerAnimController's Mecanim clips: run_forward, run_left, Idle_v02, Jump, ...) as
characters/player_body/anims.json in the format of the weapons' profile.json:
{clip: {"length", "fps", "tracks": [{"path", "rotation", "translation", "scale"}]}}, paths
relative to the character's root node, glTF axes.

Mecanim clips keep their curves packed (streamed cubic keys, dense samples, constants)
and name the animated transforms by CRC32 of their path; the paths come from the
character.gltf hierarchy.

    tools/venv/bin/python scripts/export_player_anims.py
"""
import json
import os
import pathlib
import struct
import sys
import zlib

import numpy as np
import UnityPy

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import export_weapons as ew  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
GAME = os.path.expanduser("~/.local/share/Steam/steamapps/common/Pixel Gun 3D PC Edition/Pixel Gun 3D_Data")
CHAR = ROOT / "private/export/characters/player_body"
FPS = 30
DIMS = {1: 3, 2: 4, 3: 3, 4: 3}  # position, rotation (quaternion), scale, euler


def mecanim_euler_to_quat(deg):
    """Mecanim's euler curves (localEulerAnglesRaw) compose as qz * qy * qx (X applied
    first), unlike legacy clips' Z-X-Y; checked against the bones' rest rotations."""
    r = np.radians(deg) / 2
    cx, cy, cz = np.cos(r[:, 0]), np.cos(r[:, 1]), np.cos(r[:, 2])
    sx, sy, sz = np.sin(r[:, 0]), np.sin(r[:, 1]), np.sin(r[:, 2])
    w = cz * cy * cx + sz * sy * sx
    x = cz * cy * sx - sz * sy * cx
    y = cz * sy * cx + sz * cy * sx
    z = sz * cy * cx - cz * sy * sx
    return np.stack([x, y, z, w], axis=1)


def streamed_curves(clip, count):
    """curve index -> list of (time, [a, b, c, d]) cubic keys."""
    data = struct.pack(f"<{len(clip.data)}I", *clip.data)
    out = {i: [] for i in range(count)}
    pos = 0
    while pos + 8 <= len(data):
        t, n = struct.unpack_from("<fi", data, pos)
        pos += 8
        for _ in range(n):
            idx, a, b, c, d = struct.unpack_from("<i4f", data, pos)
            pos += 20
            if idx in out:
                out[idx].append((t, (a, b, c, d)))
    return out


def eval_streamed(keys, t):
    if not keys:
        return 0.0
    # the key at or before t (keys are sorted by time)
    k = keys[0]
    for kk in keys:
        if kk[0] <= t + 1e-6:
            k = kk
        else:
            break
    dt = max(0.0, t - k[0])
    a, b, c, d = k[1]
    return ((a * dt + b) * dt + c) * dt + d


def export_clip(cl, hash_to_path):
    mc = cl.m_MuscleClip.m_Clip
    mc = getattr(mc, "data", mc)
    st, dn, cs = mc.m_StreamedClip, mc.m_DenseClip, mc.m_ConstantClip
    n_st, n_dn = st.curveCount, dn.m_CurveCount
    stream = streamed_curves(st, n_st)
    length = float(cl.m_MuscleClip.m_StopTime - cl.m_MuscleClip.m_StartTime)
    times = np.arange(0.0, length + 1e-6, 1.0 / FPS)
    if len(times) == 0:
        times = np.array([0.0])
    dense = np.array(dn.m_SampleArray, dtype=np.float32).reshape(-1, n_dn) if n_dn else None
    consts = list(cs.data)

    def curve(i, t):
        if i < n_st:
            return eval_streamed(stream[i], t)
        i -= n_st
        if i < n_dn:
            f = (t - dn.m_BeginTime) * dn.m_SampleRate
            f0 = int(np.clip(np.floor(f), 0, len(dense) - 1))
            f1 = min(f0 + 1, len(dense) - 1)
            w = float(np.clip(f - f0, 0, 1))
            return float(dense[f0, i] * (1 - w) + dense[f1, i] * w)
        i -= n_dn
        return float(consts[i]) if i < len(consts) else 0.0

    tracks = {}
    ci = 0
    for b in cl.m_ClipBindingConstant.genericBindings:
        dims = DIMS.get(b.attribute, 1) if b.typeID == 4 else 1
        idx = list(range(ci, ci + dims))
        ci += dims
        if b.typeID != 4 or b.attribute not in DIMS:
            continue
        path = hash_to_path.get(b.path)
        if path is None:
            continue
        v = np.array([[curve(i, t) for i in idx] for t in times])
        tr = tracks.setdefault(path, {"path": path})
        if b.attribute == 1:
            tr["translation"] = np.round(v * np.array([-1, 1, 1]), 5).tolist()
        elif b.attribute == 2:
            q = v / np.maximum(np.linalg.norm(v, axis=1, keepdims=True), 1e-9)
            tr["rotation"] = np.round(q * np.array([1, -1, -1, 1]), 5).tolist()
        elif b.attribute == 3:
            tr["scale"] = np.round(v, 5).tolist()
        elif b.attribute == 4:
            q = mecanim_euler_to_quat(v)
            tr["rotation"] = np.round(q * np.array([1, -1, -1, 1]), 5).tolist()
    return {"length": round(length, 4), "fps": FPS, "tracks": list(tracks.values())}


def export_body():
    """The Royale_Player_body hierarchy (Player prefab) as body.gltf."""
    import export_map as em
    env, _scene, _n = em.load_level_env("level17")
    res = next(f for k, f in env.files.items() if k.endswith("/resources.assets"))
    best = None
    for o in res.objects.values():
        if o.type.name != "Animator":
            continue
        a = o.read()
        go = em.deref(a.m_GameObject)
        if go is None or go.m_Name != "Royale_Player_body":
            continue
        t = next(em.deref(getattr(c, "component", c)) for c in go.m_Component
                 if getattr(c, "component", c).deref().type.name == "Transform")
        top = t
        while top.m_Father.m_PathID:
            top = em.deref(top.m_Father)
        # the ordinary Player prefab, not PlayerBattleRoyale
        if best is None or em.deref(top.m_GameObject).m_Name == "Player":
            best = t
    ex = em.Exporter(env, CHAR)
    ex.vis.name = "body"
    n = ex.walk(best, np.eye(4))
    ex.vis.doc["nodes"][n].update(translation=[0, 0, 0], rotation=[0, 0, 0, 1], scale=[1, 1, 1])
    ex.vis.doc["scenes"][0]["nodes"].append(n)
    ex.col.node(name="empty")
    ex.col.doc["scenes"][0]["nodes"] = [0]
    ex.save({"body": "Royale_Player_body"})
    print(f"player body: {json.dumps(dict(ex.stats))}", file=sys.stderr)


def main():
    export_body()
    g = json.loads((CHAR / "body.gltf").read_text())
    parent = {c: i for i, n in enumerate(g["nodes"]) for c in n.get("children", [])}

    def names_up(i, root):
        p = []
        while i is not None and i != root:
            p.append(g["nodes"][i]["name"])
            i = parent.get(i)
        return "/".join(reversed(p)) if i == root else None

    env = UnityPy.load(f"{GAME}/resources.assets")
    ctrl = next(o.read() for o in env.objects if o.type.name == "AnimatorController" and o.peek_name() == "RoyalePlayerAnimController")
    clips, seen = [], set()
    for p in ctrl.m_AnimationClips:
        cl = p.deref().read()
        if cl.m_Name not in seen:
            seen.add(cl.m_Name)
            clips.append(cl)
    wanted = {b.path for cl in clips for b in cl.m_ClipBindingConstant.genericBindings}
    # the Animator sits on some node of the model: pick the root whose paths match most hashes
    best = (0, None, {})
    for root in range(len(g["nodes"])):
        h = {}
        for i in range(len(g["nodes"])):
            path = names_up(i, root)
            if path:
                h[zlib.crc32(path.encode()) & 0xFFFFFFFF] = path
        hits = len(wanted & set(h))
        if hits > best[0]:
            best = (hits, root, h)
    hits, root, h = best
    print(f"  root node {g['nodes'][root]['name']!r}: {hits} of {len(wanted)} animated paths found", file=sys.stderr)
    out = {cl.m_Name: export_clip(cl, h) for cl in clips}
    out["_root"] = g["nodes"][root]["name"]
    (CHAR / "anims.json").write_text(json.dumps(out))
    print(f"player anims: {len(clips)} clips ({', '.join(sorted(c.m_Name for c in clips))[:300]}) -> {CHAR}/anims.json")


if __name__ == "__main__":
    main()

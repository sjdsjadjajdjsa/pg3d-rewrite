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
import pathlib
import struct
import sys
import zlib

import numpy as np
import UnityPy

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import export_weapons as ew  # noqa: E402
import gamepaths  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
GAME = str(gamepaths.DATA)
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


CONDITION = {1: "if", 2: "if_not", 3: "greater", 4: "less", 6: "equals", 7: "not_equal"}
PARAM_TYPE = {1: "float", 3: "int", 4: "bool", 9: "trigger"}


def export_controller(base, clip_names):
    """The AnimatorController's compiled state machines (ControllerConstant) as plain JSON:
    parameters, layers, states with their blend trees and transitions, by name."""
    tos = {}
    for e in base["m_TOS"]:
        k, v = (e[0], e[1]) if isinstance(e, (list, tuple)) else (e["first"], e["second"])
        tos[int(k)] = v
    name = lambda h: tos.get(int(h)) if int(h) else None  # noqa: E731
    c = base["m_Controller"]
    values = c["m_Values"]["data"]["m_ValueArray"]
    params = [{"name": name(v["m_ID"]), "type": PARAM_TYPE.get(v["m_Type"], v["m_Type"])} for v in values]
    layers = [
        {
            "name": name(L["data"]["m_Binding"]),
            "state_machine": L["data"]["m_StateMachineIndex"],
            "additive": L["data"]["(int&)m_LayerBlendingMode"] == 1,
            "weight": L["data"]["m_DefaultWeight"],
        }
        for L in c["m_LayerArray"]
    ]

    def transition(t, states):
        t = t["data"]
        dest = t["m_DestinationState"]
        return {
            # 30000+: exit to the machine's default state
            "to": states[dest] if dest < len(states) else None,
            "duration": t["m_TransitionDuration"],
            "offset": t["m_TransitionOffset"],
            "exit_time": t["m_ExitTime"],
            "has_exit_time": bool(t["m_HasExitTime"]),
            "can_self": bool(t.get("m_CanTransitionToSelf", True)),
            "conditions": [
                {"param": name(x["data"]["m_EventID"]), "mode": CONDITION.get(x["data"]["m_ConditionMode"]), "threshold": x["data"]["m_EventThreshold"]}
                for x in t["m_ConditionConstantArray"]
            ],
        }

    def node(n):
        n = n["data"]
        if not n["m_ChildIndices"]:
            return {"clip": clip_names[n["m_ClipID"]] if n["m_ClipID"] < len(clip_names) else None, "duration": n["m_Duration"]}
        kind = {0: "1d", 1: "simple_directional", 2: "freeform_directional", 3: "freeform_cartesian", 4: "direct"}[n["m_BlendType"]]
        out = {"blend": kind, "x": name(n["m_BlendEventID"]), "y": name(n["m_BlendEventYID"]), "children": n["m_ChildIndices"]}
        if n["m_BlendType"] == 0:
            out["thresholds"] = n["m_Blend1dData"]["data"]["m_ChildThresholdArray"]
        else:
            out["positions"] = [[p["x"], p["y"]] for p in n["m_Blend2dData"]["data"]["m_ChildPositionArray"]]
        return out

    machines = []
    for sm in c["m_StateMachineArray"]:
        sm = sm["data"]
        st = sm["m_StateConstantArray"]
        names = [name(s["data"]["m_NameID"]) for s in st]
        states = []
        for s in st:
            s = s["data"]
            trees = s["m_BlendTreeConstantArray"]
            states.append(
                {
                    "name": name(s["m_NameID"]),
                    "speed": s["m_Speed"],
                    "speed_param": name(s["m_SpeedParamID"]),
                    "loop": bool(s["m_Loop"]),
                    "tree": [node(n) for n in trees[0]["data"]["m_NodeArray"]] if trees else [],
                    "transitions": [transition(t, names) for t in s["m_TransitionConstantArray"]],
                }
            )
        machines.append(
            {
                "default": names[sm["m_DefaultState"]],
                "any_state": [transition(t, names) for t in sm["m_AnyStateTransitionConstantArray"]],
                "states": states,
            }
        )
    return {"parameters": params, "layers": layers, "state_machines": machines}


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
    # the Player prefab's body Animator plays FPSPlayerAnimController: an override of
    # RoyalePlayerAnimController (Run -> run_forward_faster, Jump_landing_2 -> Landing_v02, ...)
    over = next(o for o in env.objects if o.type.name == "AnimatorOverrideController" and o.peek_name() == "FPSPlayerAnimController")
    ov = over.read_typetree()
    objs = {o.path_id: o for o in env.objects}
    base_obj = objs[ov["m_Controller"]["m_PathID"]]
    swap = {c["m_OriginalClip"]["m_PathID"]: c["m_OverrideClip"]["m_PathID"] for c in ov["m_Clips"] if c["m_OverrideClip"]["m_PathID"]}
    base = base_obj.read_typetree()
    clip_ids = [swap.get(c["m_PathID"], c["m_PathID"]) for c in base["m_AnimationClips"]]
    clip_names = [objs[i].peek_name() for i in clip_ids]
    clips, seen = [], set()
    for pid in clip_ids:
        cl = objs[pid].read()
        if cl.m_Name not in seen:
            seen.add(cl.m_Name)
            clips.append(cl)
    controller = export_controller(base, clip_names)
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
    out["_controller"] = controller
    (CHAR / "anims.json").write_text(json.dumps(out))
    print(f"player anims: {len(clips)} clips ({', '.join(sorted(c.m_Name for c in clips))[:300]}) -> {CHAR}/anims.json")


if __name__ == "__main__":
    main()

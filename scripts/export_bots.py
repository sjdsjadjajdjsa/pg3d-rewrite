#!/usr/bin/env python3
"""Export campaign enemies ("bots") for the Bevy rewrite.

    tools/venv/bin/python scripts/export_bots.py [Enemy1_go Enemy2_go ...]

Needs private/index/bots.tsv:
    tools/venv/bin/python scripts/find_behaviours.py MeleeBot ShootingBot FiringShotBot \
        KamikadzeBot MeleeShootBot ShootingBossBot StalkerBot > private/index/bots.tsv

Writes private/export/bots/:
    bots.json             settings of every bot prefab (health, damagePerHit, speeds, ...)
    <Bot>/bot.gltf        model with skeleton + animations (from the child "<Bot minus _go>")
    <Bot>/info.json       clip names and lengths
"""
import collections
import json
import os
import pathlib
import sys

import UnityPy

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import export_map as em  # noqa: E402
import export_weapons as ew  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
INDEX = ROOT / "private/index/bots.tsv"
OUT = ROOT / "private/export/bots"
DEFAULT = [f"Enemy{i}_go" for i in range(1, 13)] + ["Enemy56_go", "Enemy57_go", "Enemy65_go"]


def export_stats(rows, gen):
    by_file = collections.defaultdict(list)
    for path, cls, go, pid in rows:
        by_file[path].append((go, cls, int(pid)))
    bots = {}
    for path, items in by_file.items():
        assets = UnityPy.load(os.path.join(em.GAME_DATA, path))
        assets.typetree_generator = gen
        want = {pid: (go, cls) for go, cls, pid in items}
        for o in assets.objects:
            if o.path_id not in want:
                continue
            go, cls = want[o.path_id]
            try:
                d = o.read_typetree()
            except Exception as e:
                print(f"  {go}: {e!r}"[:200], file=sys.stderr)
                continue
            b = {k: v for k, v in d.items() if isinstance(v, (int, float, str)) and k not in ("m_Enabled", "m_Name")}
            b["class"] = cls
            b.update(nav_agent(o))
            if "bulletPrefab" in d:
                b.update(bullet_info(o, d["bulletPrefab"]))
            bots[go] = b
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / "bots.json").write_text(json.dumps(bots, indent=0, sort_keys=True))
    print(f"bots.json: {len(bots)} bots")


def bullet_info(mb_obj, ptr):
    """Shooting bots fire a particle 'bullet' prefab: export its colour and size."""
    try:
        if ptr.get("m_FileID", 0) != 0 or not ptr.get("m_PathID"):
            return {}
        go = mb_obj.assets_file.objects[ptr["m_PathID"]].read()
        for c in go.m_Component:
            p = getattr(c, "component", c)
            if p.deref().type.name == "ParticleSystem":
                ps = p.deref().read_typetree()
                init = ps["InitialModule"]
                col = init["startColor"]["maxColor"]
                return {"bulletName": go.m_Name, "bulletColor": [col["r"], col["g"], col["b"], col["a"]],
                        "bulletSize": init["startSize"]["scalar"]}
        return {"bulletName": go.m_Name}
    except Exception as e:
        print(f"  bullet: {e!r}"[:200], file=sys.stderr)
        return {}


def nav_agent(mb_obj):
    """Settings of the prefab's NavMeshAgent (speed, size, baseOffset = hover height)."""
    try:
        go = mb_obj.read(check_read=False).m_GameObject.read()
        for c in go.m_Component:
            p = getattr(c, "component", c)
            if p.deref().type.name == "NavMeshAgent":
                d = p.deref().read_typetree()
                return {"agentSpeed": d["m_Speed"], "agentRadius": d["m_Radius"],
                        "agentHeight": d["m_Height"], "agentBaseOffset": d["m_BaseOffset"]}
    except Exception as e:
        print(f"  nav agent: {e!r}"[:200], file=sys.stderr)
    return {}


def main():
    names = sys.argv[1:] or DEFAULT
    rows = [line.rstrip("\n").split("\t") for line in open(INDEX, encoding="utf-8")]
    export_stats(rows, ew.typetree_generator())
    idx = json.loads(em.INDEX.read_text())
    by_name = {}
    for r in rows:
        by_name.setdefault(r[2], r)
    for n in names:
        if n not in by_name:
            print(f"no bot named {n}", file=sys.stderr)
            continue
        holder = n[:-3] if n.endswith("_go") else n
        # Healt_Point is the floating health bar; the game drives it, we draw our own
        ex, assets, clips = ew.export_prefab(by_name[n], idx, OUT / n, "bot", holder, skip=("Healt_Point",))
        ex.save({"bot": n, "animations": ew.animation_lengths(assets, holder), "clips": clips, "sounds": ex.sounds})
        print(f"model {n}: clips={clips} sounds={sorted(ex.sounds)} {json.dumps(dict(ex.stats))}")


if __name__ == "__main__":
    main()

#!/usr/bin/env python3
"""Export rocket/projectile settings (all) and models (those used by exported weapons).

    tools/venv/bin/python scripts/export_rockets.py

Needs private/index/rockets.tsv:
    tools/venv/bin/python scripts/find_behaviours.py RocketSettings > private/index/rockets.tsv

A weapon with `bazooka = 1` fires prefab "Rocket_<rocketNum>". Writes private/export/rockets/:
    rockets.json          every RocketSettings: lifeTime, startForce (speed), useGravity, ...
    Rocket_<n>/rocket.gltf model of the projectile
"""
import json
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import export_bots as eb  # noqa: E402
import export_map as em  # noqa: E402
import export_weapons as ew  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
INDEX = ROOT / "private/index/rockets.tsv"
OUT = ROOT / "private/export/rockets"


def main():
    rows = [line.rstrip("\n").split("\t") for line in open(INDEX, encoding="utf-8")]
    # reuse the bot stats exporter: same "scalar fields of a MonoBehaviour" job
    eb.OUT = OUT
    eb.export_stats(rows, ew.typetree_generator())
    (OUT / "bots.json").rename(OUT / "rockets.json")
    weapons = json.loads((ew.OUT / "weapons.json").read_text())
    wanted = sorted({f"Rocket_{w['rocketNum']}" for n, w in weapons.items()
                     if w.get("bazooka") and (ew.OUT / n / "weapon.gltf").exists()})
    idx = json.loads(em.INDEX.read_text())
    by_name = {}
    for r in rows:
        by_name.setdefault(r[2], r)
    for n in wanted:
        if n not in by_name:
            print(f"no rocket {n}", file=sys.stderr)
            continue
        ex, _assets, _clips = ew.export_prefab(by_name[n], idx, OUT / n, "rocket", "")
        ex.save({"rocket": n})
        print(f"model {n}: {json.dumps(dict(ex.stats))}")


if __name__ == "__main__":
    main()

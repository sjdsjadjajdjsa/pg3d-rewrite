#!/usr/bin/env python3
"""Export the shop/HUD pictures of the exported weapons.

The game keeps one picture per weapon, "<Tag>_icon1_big" (Tag from WeaponSounds, e.g.
FirstShotgun, UziWeapon, SVD), in the bundles woi_0 .. woi_N ("weapon icons"). The in-match
weapon bar shows them in its slots.

    tools/venv/bin/python scripts/export_weapon_icons.py [Weapon2 Weapon3 ...]

Writes private/export/weapons/<Weapon>/icon.png for every exported weapon folder (or the
ones named).
"""
import glob
import json
import pathlib
import sys

import UnityPy

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / "private/export/weapons"
INDEX = ROOT / "private/index/bundles.json"


def main():
    rows = json.loads((OUT / "weapons.json").read_text())
    names = sys.argv[1:] or sorted(p.name for p in OUT.iterdir() if p.is_dir())
    want = {}
    for n in names:
        tag = (rows.get(n) or {}).get("Tag") or ""
        if tag:
            want[f"{tag}_icon1_big".lower()] = n
    root = json.loads(INDEX.read_text())["root"]
    found = 0
    for path in sorted(glob.glob(f"{root}/woi_*/*/*")):
        if path.endswith((".resS", ".resource")):
            continue
        env = UnityPy.load(path)
        for o in env.objects:
            if o.type.name != "Texture2D":
                continue
            try:
                name = o.peek_name() or ""
            except Exception:
                continue
            weapon = want.get(name.lower())
            if weapon is None:
                continue
            o.read().image.save(OUT / weapon / "icon.png")
            print(f"icon {weapon}: {name}")
            found += 1
            del want[name.lower()]
    for k, n in want.items():
        print(f"icon {n}: {k} not found", file=sys.stderr)
    print(f"icons: {found} exported")


if __name__ == "__main__":
    main()

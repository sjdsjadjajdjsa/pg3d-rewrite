#!/usr/bin/env python3
"""Export the game's mode icons (resources.assets z_mode_*, gamemode_*) for the mode picker.

    tools/venv/bin/python scripts/export_mode_icons.py

Writes private/export/ui/modes/modeart/<name>.png (z_mode_teamf = Team Fight,
z_mode_wws, z_mode_frp, z_mode_random = the Random Map die, ...).
"""
import pathlib

import UnityPy
import gamepaths

ROOT = pathlib.Path(__file__).resolve().parent.parent
GAME = str(gamepaths.DATA)
OUT = ROOT / "private/export/ui/modes/modeart"


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    n = 0
    for o in UnityPy.load(f"{GAME}/resources.assets").objects:
        if o.type.name != "Texture2D":
            continue
        name = o.peek_name()
        if name and (name.startswith("z_mode_") or name.startswith("gamemode_") or name in ("icon_freeplay_mode", "mode_team_znachek")):
            o.read().image.save(OUT / f"{name}.png")
            n += 1
    print(f"mode icons: {n} -> {OUT}")


if __name__ == "__main__":
    main()

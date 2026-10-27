#!/usr/bin/env python3
"""Export the keyboard/mouse key pictures (kb_*) the game shows as key hints (GamepadTipView)
to private/export/ui/keys/<name>.png, e.g. kb_R.png, kb_MouseLeft.png, kb_LeftControl.png.

    tools/venv/bin/python scripts/export_keys.py
"""
import pathlib

import UnityPy
import gamepaths

ROOT = pathlib.Path(__file__).resolve().parent.parent
GAME = gamepaths.DATA
OUT = ROOT / "private/export/ui/keys"


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    env = UnityPy.load(str(GAME / "resources.assets"), *[str(p) for p in GAME.glob("resources.assets.resS")])
    n = 0
    for o in env.objects:
        if o.type.name != "Texture2D":
            continue
        d = o.read()
        if d.m_Name.startswith("kb_"):
            d.image.save(OUT / f"{d.m_Name}.png")
            n += 1
    print(f"keys: {n} -> {OUT}")


if __name__ == "__main__":
    main()

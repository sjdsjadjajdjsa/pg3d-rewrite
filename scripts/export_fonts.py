#!/usr/bin/env python3
"""Export the TrueType fonts embedded in resources.assets (e.g. RussoOne-Regular, the font of
the players' NickLabel TextMesh) to private/export/fonts/<name>.ttf.

    tools/venv/bin/python scripts/export_fonts.py
"""
import os
import pathlib

import UnityPy

ROOT = pathlib.Path(__file__).resolve().parent.parent
GAME = os.path.expanduser("~/.local/share/Steam/steamapps/common/Pixel Gun 3D PC Edition/Pixel Gun 3D_Data")


def main():
    out = ROOT / "private/export/fonts"
    out.mkdir(parents=True, exist_ok=True)
    env = UnityPy.load(f"{GAME}/resources.assets")
    for o in env.objects:
        if o.type.name != "Font":
            continue
        d = o.read_typetree()
        data = bytes(d.get("m_FontData") or b"")
        if data[:4] in (b"\x00\x01\x00\x00", b"OTTO", b"true"):
            (out / f"{d['m_Name']}.ttf").write_bytes(data)
            print(d["m_Name"], len(data))


if __name__ == "__main__":
    main()

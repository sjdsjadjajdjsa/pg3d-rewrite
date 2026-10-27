#!/usr/bin/env python3
"""Find TextAssets / MonoBehaviours containing a string in the game's top-level asset files.

    tools/venv/bin/python scripts/find_text.py Key_0595
"""
import glob
import os
import sys

import UnityPy
import gamepaths

GAME = str(gamepaths.DATA)
needle = sys.argv[1].encode()

files = [f"{GAME}/resources.assets", f"{GAME}/globalgamemanagers.assets"] + sorted(glob.glob(f"{GAME}/sharedassets*.assets")) + sorted(glob.glob(f"{GAME}/level[0-9]*"))
for path in files:
    if path.endswith(".resS"):
        continue
    raw = open(path, "rb").read()
    if needle not in raw:
        continue
    assets = UnityPy.load(path)
    for o in assets.objects:
        if o.type.name not in ("TextAsset", "MonoBehaviour"):
            continue
        data = o.get_raw_data()
        if needle in data:
            name = ""
            try:
                name = o.read(check_read=False).m_Name
            except Exception:
                pass
            print(os.path.basename(path), o.type.name, o.path_id, name, len(data))

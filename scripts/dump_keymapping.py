#!/usr/bin/env python3
"""Print the key binding window's tabs and actions (KeyMappingViewData in resources.assets):
id | English name | locked on keyboard | action number.

    tools/venv/bin/python scripts/dump_keymapping.py
"""
import json
import os
import sys

import UnityPy

sys.path.insert(0, os.path.dirname(__file__))
import export_weapons as ew  # noqa: E402

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
G = os.path.expanduser("~/.local/share/Steam/steamapps/common/Pixel Gun 3D PC Edition/Pixel Gun 3D_Data")
env = UnityPy.load(G + "/resources.assets")
env.typetree_generator = ew.typetree_generator()
S = json.load(open(f"{ROOT}/private/export/ui/strings_en-gb.json"))
f = list(env.files.values())[0]
for o in f.objects.values():
    if o.type.name != "MonoBehaviour" or b"keyMappingTabData" not in o.get_raw_data() and b"PrimaryCategory" not in o.get_raw_data():
        continue
    d = o.read_typetree()
    if "keyMappingTabData" not in d:
        continue
    for t in d["keyMappingTabData"]:
        print("==", t["id"], S.get(t["locKey"], t["locKey"]))
        for a in t["keyActions"]:
            print("  ", a["id"], "|", S.get(a["locKey"], a["locKey"]), "| kb-locked" if a["cantChangeOnKeyboard"] else "|", a["action"])
    break

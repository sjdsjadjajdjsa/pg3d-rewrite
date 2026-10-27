#!/usr/bin/env python3
"""weapons/skins.json: exported weapon -> its skin names (not "default"), from the
weaponskinsv2 assets listed in private/index/bundle_containers.tsv (every bundle's
container paths). The Armory draws a paint pattern on cells of weapons that have skins."""
import collections, json, os, pathlib, re

ROOT = pathlib.Path(__file__).resolve().parent.parent
W = ROOT / "private/export/weapons"
sk = collections.defaultdict(list)
for line in open(ROOT / "private/index/bundle_containers.tsv"):
    _b, k = line.rstrip("\n").split("\t")
    m = re.match(r"assets/editor/resources/weaponskinsv2/(weapon\d+)_(.+)\.asset$", k)
    if m:
        sk[m.group(1)].append(m.group(2))
out = {}
for n in sorted(os.listdir(W)):
    if (W / n / "weapon.gltf").exists():
        s = sorted(x for x in sk.get(n.lower(), []) if x != "default")
        if s:
            out[n] = s
(W / "skins.json").write_text(json.dumps(out, indent=0))
print(f"skins: {len(out)} weapons with skins -> {W / 'skins.json'}")

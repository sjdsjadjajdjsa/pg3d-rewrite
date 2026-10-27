#!/usr/bin/env python3
"""Export the weapon effect table the Armory uses for the effect row and its Properties
popup: InShopEffects value -> title key, description key, stat icon.

The table is built in the static constructor of Weapons.T_2001B54 (token 600D8DC): one
entry per effect, in enum order, each loading its description key, title key and icon
sprite name; the `mov r32, imm` before an entry is the previous entry's value, so the
entry's value is that + 1 (the first two entries come before any). A second table (shop
stats) follows; it starts where the constants go down again."""
import json, pathlib, sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import il2cpp_lits

ROOT = pathlib.Path(__file__).resolve().parent.parent
OUT = ROOT / 'private/export/weapons/effects.json'
CCTOR = (0x9CD940, 0x638D)
STRINGS = json.load(open(ROOT / 'private/export/ui/strings_en-gb.json'))

refs = il2cpp_lits.refs_in(*CCTOR)
# skip the method's one-time literal registration block: the body starts at the first
# literal loaded a second time
seen, start = set(), 0
for k, (_, kind, v) in enumerate(refs):
    if kind == 'lit':
        if v in seen:
            start = k
            break
        seen.add(v)
while start > 0 and refs[start - 1][1] == 'lit':
    start -= 1
groups, cur, val, prev = [], [], 0, -1
for _, kind, v in refs[start:]:
    if kind == 'imm':
        if v == 200:
            continue
        groups.append((val, cur))
        cur = []
        if v < prev:
            break
        prev, val = v, v + 1
        continue
    cur.append(v)
first = groups[0][1]
groups = [(0, first[:3]), (1, first[3:])] + groups[1:]
out = {}
for val, lits in groups:
    keys = sorted((x for x in lits if x.startswith('Key_')), key=lambda k: len(STRINGS.get(k, '')))
    icons = [x for x in lits if x.startswith('shop_stats')]
    if not keys or not icons:
        continue
    out[str(val)] = {'title': keys[0], 'desc': keys[-1] if len(keys) > 1 else None, 'icon': icons[-1]}
OUT.parent.mkdir(parents=True, exist_ok=True)
json.dump(out, open(OUT, 'w'), indent=1)
print(f'effects: {len(out)} -> {OUT}')

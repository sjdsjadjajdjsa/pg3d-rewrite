#!/usr/bin/env python3
"""Which methods use a string literal (regex, full match): finds RIP-relative loads of
the literal's metadata-usage slot (il2cpp_lits.py) and maps them to methods via
private/index/methods.tsv. usage: literal_refs.py <regex> [...]"""
import bisect, pathlib, re, struct, sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
import il2cpp_lits as L

ROOT = pathlib.Path(__file__).resolve().parent.parent
pats = [re.compile(p) for p in sys.argv[1:]]
slots = {a: s for a, s in L.slots().items() if any(p.fullmatch(s) for p in pats)}
meth = []
for line in open(ROOT / 'private/index/methods.tsv'):
    f = line.rstrip('\n').split('\t')
    try:
        meth.append((int(f[0], 16), int(f[1], 16), f'{f[3]}.{f[4]}'))
    except (ValueError, IndexError):
        pass
meth.sort()
starts = [m[0] for m in meth]
out = {}
for sec in ('.text', 'il2cpp'):
    vs, va, rs, ra = L.sections()[sec]
    code = L._dll[ra:ra + rs]
    for k in range(len(code) - 4):
        tgt = va + k + 4 + struct.unpack_from('<i', code, k)[0]
        if tgt in slots:
            j = bisect.bisect_right(starts, va + k) - 1
            name = f'{meth[j][2]}@{meth[j][0]:#x}' if j >= 0 and va + k < meth[j][0] + meth[j][1] else '?'
            out.setdefault(slots[tgt], set()).add(name)
for s in sorted(set(slots.values())):
    print(s, sorted(out.get(s, [])))

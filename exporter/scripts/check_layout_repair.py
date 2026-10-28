#!/usr/bin/env python3
"""How well layout_repair.py copes with game updates, measured on real game data.

Needs a game build whose saved layouts are right (the October 2026 build): its objects
read correctly, so they are the truth. Each script's saved layout is then changed the way an
update changes scripts, repaired against the real objects, and the repaired reading is
compared value by value with the truth:

    added      the game added a field          (one saved field removed)
    removed    the game removed a field        (a made-up field inserted)
    two added  two fields added                (two saved fields removed)
    changed    a field added and one removed

    python scripts/check_layout_repair.py [--min N] [-v] [scene or asset file ...]   default: level17 level6

--min N only checks scripts with at least N objects in the file (the menus' building blocks
have hundreds; the few scripts with one object are the hard, ambiguous case).

Per change kind: scripts read right, read wrong (the dangerous outcome) and given up.
"""
import collections
import random
import sys
import time

import UnityPy
from UnityPy.helpers.TypeTreeNode import TypeTreeNode

import gamepaths
import layout_repair as lr

FAKE = [
    [(1, "int", "fake", 0)],
    [(1, "float", "fake", 0)],
    [(1, "bool", "fake", lr.ALIGN)],
    [(1, "string", "fake", lr.ALIGN), (2, "Array", "Array", lr.ALIGN), (3, "int", "size", 0), (3, "char", "data", 0)],
    [(1, "PPtr<Object>", "fake", 0), (2, "int", "m_FileID", 0), (2, "SInt64", "m_PathID", 0)],
    [(1, "vector", "fake", 0), (2, "Array", "Array", lr.ALIGN), (3, "int", "size", 0), (3, "int", "data", 0)],
]


def flat(node):
    return [(n.m_Level, n.m_Type, n.m_Name, n.m_MetaFlag or 0) for n in node.traverse()]


def build(rows):
    return TypeTreeNode.from_list([TypeTreeNode(lv, t, nm, 0, 0, m_MetaFlag=fl) for lv, t, nm, fl in rows])


def mutate(root, kind, rnd, first):
    """The saved layout as it would be if the game had changed the script this way."""
    fields = [flat(c) for c in root.m_Children]
    head = [(0, root.m_Type, root.m_Name, root.m_MetaFlag or 0)]
    if len(fields) - first < 3:
        return None
    ops = {"added": ["del"], "removed": ["ins"], "two added": ["del", "del"], "changed": ["del", "ins"]}[kind]
    for op in ops:
        if op == "del":
            fields.pop(rnd.randrange(first, len(fields)))
        else:
            fields.insert(rnd.randrange(first, len(fields) + 1), rnd.choice(FAKE))
    return build(head + [r for f in fields for r in f])


def main():
    args = [a for a in sys.argv[1:] if a != "-v"]
    least = int(args.pop(args.index("--min") + 1)) if "--min" in args else 1
    args = [a for a in args if a != "--min"]
    files = args or ["level17", "level6"]
    data = gamepaths.DATA
    gen = gamepaths.typetree_generator()
    if not isinstance(gen, gamepaths.SavedLayouts):
        gen = gamepaths.SavedLayouts()
    index = lr.field_index(gen.layouts)
    rnd = random.Random(7)
    results = collections.defaultdict(collections.Counter)
    t0 = time.time()
    for name in files:
        env = UnityPy.load(str(data / name), str(data / "globalgamemanagers.assets"))
        env.typetree_generator = gen
        sf = next(f for f in env.files.values() if f.name == name)
        by_script = collections.defaultdict(list)
        for o in sf.objects.values():
            if o.type.name != "MonoBehaviour":
                continue
            try:
                node = o._get_typetree_node()
                truth = o.read_typetree(nodes=node)
            except Exception:
                continue
            if len(node.m_Children) > len(lr.HEADER):
                by_script[node.m_Type].append((o, node, truth))
        for script, objs in sorted(by_script.items()):
            if len(objs) < least:
                continue
            objs = objs[:2 * lr.SAMPLE]
            root = objs[0][1]
            files = lr.Files(sf, index)
            # the recorded values come from half the objects, the other half is repaired: an
            # update keeps many values but not all
            known = collections.defaultdict(set)
            for o, node, _ in objs[1::2]:
                try:
                    for k, f in lr.values(node, lr.Data(o.get_raw_data(), files)).items():
                        known[k].add(f)
                except (lr.Bad, lr.Unsupported):
                    pass
            objs = objs[0::2]
            first = lr.Repair(root).first
            datas = [lr.Data(o.get_raw_data(), files) for o, _, _ in objs]
            for kind in ("added", "removed", "two added", "changed"):
                changed = mutate(root, kind, rnd, first)
                if changed is None:
                    continue
                rep = lr.Repair(changed, known)
                plan, unsure = rep.plan(datas)
                outcome = "right"
                for (o, _, truth), d in zip(objs, datas):
                    def reads(edits, o=o):
                        try:
                            o.read_typetree(nodes=rep.node(edits, TypeTreeNode))
                            return True
                        except Exception:
                            return False
                    edits = rep.edits_for(d, plan, reads)
                    if edits is None:
                        outcome = "given up"
                        break
                    got = o.read_typetree(nodes=rep.node(edits, TypeTreeNode))
                    bad = [k for k, v in truth.items() if k in got and got[k] != v]
                    if bad:
                        outcome = "wrong"
                        if "-v" in sys.argv:
                            print(f"  {script} {kind}: {len(objs)} objects, {len(known)} known fields, wrong {bad[:4]}",
                                  {k: (truth[k], got[k]) for k in bad[:2]})
                        break
                results[kind][outcome] += 1
                results[f"{kind}, {'6+' if len(objs) >= 6 else '1-5'} objects"][outcome] += 1
                results[kind]["unsure"] += unsure
                results[kind]["unsure wrong"] += unsure and outcome == "wrong"
    print(f"{sum(sum(c.values()) for c in results.values())} script changes in {time.time() - t0:.0f} s")
    for kind, c in results.items():
        total = c["right"] + c["wrong"] + c["given up"]
        print(f"{kind:10} {c['right']:4} right  {c['wrong']:4} wrong  {c['given up']:4} given up  "
              f"({100 * c['right'] / total:.0f} % right; flagged unsure {c['unsure']}, of them wrong {c['unsure wrong']})")


if __name__ == "__main__":
    main()

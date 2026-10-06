#!/usr/bin/env python3
"""Index Cpp2IL "diffable-cs" output into flat TSV tables + a readable summary.

Input : private/cpp2il/cs/DiffableCs/<Assembly>/<namespace dirs>/<Type>.cs
Output: private/index/{types,methods,fields}.tsv and private/index/summary.md

Obfuscated identifiers (made of CJK characters like 丅丁丑) get a stable ASCII
alias built from their metadata token, e.g. method 0x6002910 -> m_6002910,
type 0x2000462 -> T_2000462, field 0x40025C1 -> f_40025C1.
"""
import collections
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "private/cpp2il/cs/DiffableCs"
OUT = ROOT / "private/index"

CJK = re.compile(r"[\u3400-\u9fff\uf900-\ufaff]")
TYPE_RE = re.compile(
    r"^(\t*)((?:(?:public|private|protected|internal|static|sealed|abstract|readonly|ref|unsafe|partial|new)\s+)*)"
    r"(class|struct|interface|enum)\s+([^\s:<]+(?:<[^>]*>)?)\s*(?::\s*(.*))?$")
ADDR_RE = re.compile(r'\[Address\(RVA = "0x([0-9A-Fa-f]+)", Offset = "0x[0-9A-Fa-f]+", Length = "0x([0-9A-Fa-f]+)"\)\]')
TOKEN_RE = re.compile(r'\[Token\(Token = "0x([0-9A-Fa-f]+)"\)\]')
FIELD_RE = re.compile(r"^\t*((?:\w+\s+)*?)([^\s;]+(?:<[^;]*?>)?(?:\[\])*)\s+([^\s;=]+)(?:\s*=\s*[^;]*)?;\s*//Field offset: 0x([0-9A-Fa-f]+)")
METHOD_NAME_RE = re.compile(r"([^\s(<]+)(?:<[^(]*>)?\s*\(")

def obf(name: str) -> bool:
    return bool(CJK.search(name))


def alias(name: str, prefix: str, token: str | None) -> str:
    if not obf(name):
        return name
    return f"{prefix}_{token}" if token else f"{prefix}_obf"


def parse_file(path: pathlib.Path, asm: str, types, methods, fields):
    lines = path.read_text(encoding="utf-8", errors="replace").splitlines()
    ns = ""
    if lines and lines[0].startswith("namespace "):
        ns = lines[0][len("namespace "):].rstrip(";").strip()
    stack = []  # (indent, type record)
    pending_addr = None
    pending_token = None
    for line in lines:
        m = TOKEN_RE.search(line)
        if m:
            pending_token = m.group(1)
            continue
        m = ADDR_RE.search(line)
        if m:
            pending_addr = (int(m.group(1), 16), int(m.group(2), 16))
            continue
        if line.lstrip().startswith(("[", "{")) or not line.strip():
            continue
        indent = len(line) - len(line.lstrip("\t"))
        m = TYPE_RE.match(line)
        if m:
            while stack and stack[-1][0] >= indent:
                stack.pop()
            raw = m.group(4)
            parent = stack[-1][1] if stack else None
            nice = alias(raw, "T", pending_token)
            full = (parent["full"] + "." if parent else (ns + "." if ns else "")) + nice
            rec = dict(asm=asm, ns=ns, raw=raw, name=nice, full=full, kind=m.group(3),
                       mods=m.group(2).strip(), base=(m.group(5) or "").strip(),
                       token=pending_token or "", nested=parent is not None,
                       obf=obf(raw), methods=0, fields=0, code_bytes=0, file=str(path.relative_to(SRC)))
            types.append(rec)
            stack.append((indent, rec))
            pending_addr = pending_token = None
            continue
        while stack and stack[-1][0] >= indent:
            stack.pop()
        owner = stack[-1][1] if stack else None
        if owner is None:
            pending_addr = pending_token = None
            continue
        s = line.strip()
        if pending_addr and "(" in s and not s.startswith(("get ", "set ", "add ", "remove ")):
            mm = METHOD_NAME_RE.search(s.split("//")[0])
            raw = mm.group(1) if mm else "?"
            if raw == owner["raw"].split("<")[0]:
                raw = ".cctor" if re.search(r"\bstatic\b", s.split("(")[0]) else ".ctor"
            nice = alias(raw, "m", pending_token)
            rva, length = pending_addr
            methods.append(dict(rva=rva, length=length, type=owner["full"], name=nice, raw=raw,
                                token=pending_token or "", sig=s.replace("{ }", "").strip(), asm=asm))
            owner["methods"] += 1
            owner["code_bytes"] += length
        elif "//Field offset:" in s:
            fm = FIELD_RE.match(line)
            if fm:
                mods = fm.group(1).split()
                raw = fm.group(3)
                fields.append(dict(type=owner["full"], name=alias(raw, "f", pending_token), raw=raw,
                                   ftype=fm.group(2), offset=int(fm.group(4), 16),
                                   static="static" in mods or "const" in mods, token=pending_token or ""))
                owner["fields"] += 1
        pending_addr = pending_token = None


def main():
    types, methods, fields = [], [], []
    for asm_dir in sorted(p for p in SRC.iterdir() if p.is_dir()):
        for f in sorted(asm_dir.rglob("*.cs")):
            parse_file(f, asm_dir.name, types, methods, fields)
    OUT.mkdir(parents=True, exist_ok=True)

    def tsv(name, rows, cols):
        with open(OUT / name, "w", encoding="utf-8") as fh:
            fh.write("\t".join(cols) + "\n")
            for r in rows:
                fh.write("\t".join(
                    (f"0x{r[c]:X}" if c in ("rva", "length", "offset") else str(r[c])).replace("\t", " ")
                    for c in cols) + "\n")

    tsv("types.tsv", types, ["asm", "full", "raw", "kind", "mods", "base", "token", "obf", "methods", "fields", "code_bytes", "file"])
    tsv("methods.tsv", methods, ["rva", "length", "asm", "type", "name", "raw", "token", "sig"])
    tsv("fields.tsv", fields, ["type", "name", "raw", "ftype", "offset", "static", "token"])
    write_summary(types, methods)
    print(f"types={len(types)} methods={len(methods)} fields={len(fields)} -> {OUT}")


def write_summary(types, methods):
    by_asm = collections.defaultdict(lambda: [0, 0, 0])
    for t in types:
        by_asm[t["asm"]][0] += 1
        by_asm[t["asm"]][1] += t["methods"]
        by_asm[t["asm"]][2] += t["code_bytes"]
    game = [t for t in types if t["asm"] in ("Assembly-CSharp", "Assembly-CSharp-firstpass")]
    ns_count = collections.Counter(t["ns"] or "(global)" for t in game if not t["nested"])
    top = [t for t in game if not t["nested"]]
    o = []
    o.append("# Cpp2IL summary\n")
    o.append(f"Total: {len(types)} types, {len(methods)} methods with code, "
             f"{sum(1 for t in types if t['obf'])} types with obfuscated names, "
             f"{sum(1 for m in methods if obf(m['raw']))} methods with obfuscated names.\n")
    o.append("## Assemblies (by compiled code size)\n")
    o.append("| assembly | types | methods | code KB |\n|---|---:|---:|---:|")
    for a, (nt, nm, nb) in sorted(by_asm.items(), key=lambda kv: -kv[1][2]):
        o.append(f"| {a} | {nt} | {nm} | {nb // 1024} |")
    o.append("\n## Game namespaces (Assembly-CSharp + firstpass, top-level types)\n")
    o.append("| namespace | types |\n|---|---:|")
    for n, c in ns_count.most_common(60):
        o.append(f"| {n} | {c} |")
    o.append("\n## Biggest game classes (by compiled code size, incl. nested types)\n")
    size = collections.Counter()
    meth = collections.Counter()
    for t in game:
        size[t["full"]] += t["code_bytes"]
        meth[t["full"]] += t["methods"]
    rolled = collections.Counter()
    rolled_m = collections.Counter()
    top_names = {t["full"] for t in top}
    for full, b in size.items():  # roll nested types up into their top-level owner
        k = full
        while k not in top_names and "." in k:
            k = k.rsplit(".", 1)[0]
        rolled[k] += b
        rolled_m[k] += meth[full]
    o.append("| class | methods | code KB | base |\n|---|---:|---:|---|")
    base = {t["full"]: t["base"] for t in top}
    for k, b in rolled.most_common(80):
        o.append(f"| {k} | {rolled_m[k]} | {b // 1024} | {base.get(k, '')[:60]} |")
    (OUT / "summary.md").write_text("\n".join(o) + "\n", encoding="utf-8")


if __name__ == "__main__":
    sys.exit(main())

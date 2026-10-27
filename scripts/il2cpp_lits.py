"""String literals of GameAssembly.dll code (il2cpp metadata v31).

A method that uses a literal loads it from a .data slot holding the metadata-usage token
0xA0000000 | index << 1 | 1 until the runtime resolves it. `slots()` maps those slot
addresses to their strings, `refs_in(rva, length)` lists, in code order, the literals a
method loads and the small `mov r32, imm32` constants between them."""
import glob, struct
import gamepaths

G = str(gamepaths.INSTALL)
_dll = open(f'{G}/GameAssembly.dll', 'rb').read()
_meta = open(glob.glob(f'{G}/*_Data/il2cpp_data/Metadata/global-metadata.dat')[0], 'rb').read()


def literals():
    so, ss, do = struct.unpack_from('<3I', _meta, 8)
    out = []
    for i in range(ss // 8):
        ln, di = struct.unpack_from('<II', _meta, so + 8 * i)
        out.append(_meta[do + di:do + di + ln].decode('utf-8', 'replace'))
    return out


def sections():
    pe = struct.unpack_from('<I', _dll, 0x3c)[0]
    n = struct.unpack_from('<H', _dll, pe + 6)[0]
    opt = struct.unpack_from('<H', _dll, pe + 20)[0]
    out = {}
    for i in range(n):
        o = pe + 24 + opt + 40 * i
        out[_dll[o:o + 8].rstrip(b'\0').decode()] = struct.unpack_from('<IIII', _dll, o + 8)
    return out


def slots():
    lits = literals()
    vs, va, rs, ra = sections()['.data']
    out = {}
    for off in range(0, rs - 8, 8):
        v = struct.unpack_from('<Q', _dll, ra + off)[0]
        if v >> 29 == 5 and v & 1 and v < 0xC0000000 and (v & 0x1FFFFFFF) >> 1 < len(lits):
            out[va + off] = lits[(v & 0x1FFFFFFF) >> 1]
    return out


def refs_in(rva, length, slot_map=None):
    """[(rva, 'lit', str) | (rva, 'imm', int)] in code order."""
    slot_map = slot_map or slots()
    for vs, va, rs, ra in sections().values():
        if va <= rva < va + rs:
            code = _dll[ra + rva - va:ra + rva - va + length]
            break
    else:
        return []
    out = []
    for k in range(len(code) - 4):
        tgt = rva + k + 4 + struct.unpack_from('<i', code, k)[0]
        if tgt in slot_map:
            out.append((rva + k, 'lit', slot_map[tgt]))
        if 0xB8 <= code[k] < 0xC0 and (k == 0 or code[k - 1] not in (0x48, 0x49, 0x4C)):
            imm = struct.unpack_from('<i', code, k + 1)[0]
            if 0 <= imm < 400:
                out.append((rva + k, 'imm', imm))
    return out

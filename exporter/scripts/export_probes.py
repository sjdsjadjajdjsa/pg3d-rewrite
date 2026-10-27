"""Export a map's baked light probes as an irradiance volume for the Bevy game.

Unity lights moving things (zombies, weapons, the player's character) with *light
probes*: points in the level where the baked indirect light was recorded as L2
spherical harmonics (27 numbers: 9 per colour channel). Bevy has no SH probes; it uses
an *irradiance volume*: a 3D grid of "ambient cubes" (one colour per +-X/+-Y/+-Z side).
We resample the probes onto such a grid.

Writes to private/export/maps/<map>/:
    probes.bin   float16 RGBA, Bevy's irradiance volume layout (Rx, 2Ry, 3Rz)
    probes.json  {"resolution": [Rx,Ry,Rz], "min": [...], "max": [...], "probes": N, ...}
Coordinates are already converted to glTF/Bevy (X mirrored), like export_map.py.

Usage: tools/venv/bin/python scripts/export_probes.py <map> [<map> ...]
"""

import json
import sys

import numpy as np

import export_map as em

# Unity's SphericalHarmonicsL2 follows Sloan's "Stupid SH tricks" convention.
# value(n) = sum_i sh[c, i] * BASIS[i] * CONV[i] * poly_i(n), where CONV is the cosine-lobe
# convolution divided by pi (that is what ShadeSH9 returns and multiplies with albedo).
BASIS = np.array([0.282095, -0.488603, 0.488603, -0.488603, 1.092548, -1.092548, 0.315392, -1.092548, 0.546274])
CONV = np.array([1.0, 2 / 3, 2 / 3, 2 / 3, 0.25, 0.25, 0.25, 0.25, 0.25])

# voxel size in metres, and the largest grid per axis
SPACING = 1.5
MAX_RES = 48
NEAREST = 4


def poly(d):
    """SH polynomials (Unity order) for unit directions d (..., 3) in Unity space."""
    x, y, z = d[..., 0], d[..., 1], d[..., 2]
    return np.stack([np.ones_like(x), y, z, x, x * y, y * z, 3 * z * z - 1, x * z, x * x - y * y], -1)


def find_probes(env, scene_path):
    ex = em.Exporter.__new__(em.Exporter)
    ex.env = env
    for obj in em.Exporter.scene_objects(ex, scene_path):
        if obj.type.name == "LightmapSettings":
            return em.deref(obj.read().m_LightProbes)
    return None


def export(map_name):
    env, scene_path, _ = em.load_env(map_name)
    lp = find_probes(env, scene_path)
    out = em.OUT_ROOT / map_name
    if lp is None or not lp.m_Data.m_Positions:
        print(f"probes {map_name}: none")
        (out / "probes.json").unlink(missing_ok=True)
        return
    # Unity space positions and coefficients [probe, channel, 9]
    pos = np.array([[p.x, p.y, p.z] for p in lp.m_Data.m_Positions], dtype=np.float64)
    sh = np.array([[getattr(c, f"sh_{i:_>2}_") for i in range(27)] for c in lp.m_BakedCoefficients])
    sh = sh.reshape(-1, 3, 9)
    n = min(len(pos), len(sh))
    pos, sh = pos[:n], sh[:n]

    # grid over the probes' bounding box (in Unity space; X is mirrored at the end),
    # stretched over everywhere characters can walk (the navmesh, 2 m above it): outside
    # the grid Bevy has no indirect light at all and characters turn near black, where
    # Unity would still light them from the nearest probes (as the blend below does)
    lo, hi = pos.min(0) - 1.0, pos.max(0) + 1.0
    nav = out / "navmesh.json"
    if nav.exists():
        nv = json.loads(nav.read_text())
        v = np.array(nv.get("vertices") or nv.get("verts") or [], dtype=np.float64).reshape(-1, 3)
        if len(v):
            v[:, 0] *= -1  # back to Unity space
            # (a few stray navmesh bits far off the map are left out)
            lo = np.minimum(lo, np.percentile(v, 0.5, 0) - [2.0, 1.0, 2.0])
            hi = np.maximum(hi, np.percentile(v, 99.5, 0) + [2.0, 3.0, 2.0])
            plo, phi = pos.min(0) - 1.0, pos.max(0) + 1.0
            lo, hi = np.maximum(lo, plo - 15.0), np.minimum(hi, phi + 15.0)
    res = np.clip(np.ceil((hi - lo) / SPACING).astype(int) + 1, 2, MAX_RES)
    axes = [np.linspace(lo[a], hi[a], res[a]) for a in range(3)]
    gx, gy, gz = np.meshgrid(*axes, indexing="ij")
    cells = np.stack([gx, gy, gz], -1).reshape(-1, 3)

    # inverse-distance blend of the nearest probes (Unity blends inside tetrahedra)
    blended = np.zeros((len(cells), 3, 9))
    for s in range(0, len(cells), 4096):
        c = cells[s:s + 4096]
        d2 = ((c[:, None, :] - pos[None, :, :]) ** 2).sum(-1)
        idx = np.argpartition(d2, min(NEAREST, n) - 1, axis=1)[:, :NEAREST]
        w = 1.0 / (np.take_along_axis(d2, idx, 1) + 1e-3)
        w /= w.sum(1, keepdims=True)
        blended[s:s + 4096] = (sh[idx] * w[..., None, None]).sum(1)

    # the six cube sides in Bevy space -> Unity space (mirror X)
    sides_bevy = np.array([[-1, 0, 0], [1, 0, 0], [0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1]], dtype=np.float64)
    sides_unity = sides_bevy * np.array([-1, 1, 1])
    k = poly(sides_unity) * BASIS * CONV  # (6, 9)
    vals = np.clip(np.einsum("vci,si->vsc", blended, k), 0, None)  # (voxel, side, rgb)

    rx, ry, rz = (int(r) for r in res)
    vals = vals.reshape(rx, ry, rz, 6, 3)
    vals = vals[::-1]  # Bevy x runs the other way
    # texture (width Rx, height 2Ry, depth 3Rz); numpy index [p, t, s]
    tex = np.zeros((3 * rz, 2 * ry, rx, 4), dtype=np.float16)
    tex[..., 3] = 1.0
    for side in range(6):
        axis, positive = side // 2, side % 2
        t0, p0 = positive * ry, axis * rz
        tex[p0:p0 + rz, t0:t0 + ry, :, :3] = vals[:, :, :, side, :].transpose(2, 1, 0, 3)
    (out / "probes.bin").write_bytes(tex.tobytes())
    mn = [-hi[0], lo[1], lo[2]]
    mx = [-lo[0], hi[1], hi[2]]
    up = vals[..., 3, :]
    info = {"resolution": [rx, ry, rz], "min": mn, "max": mx, "probes": int(n),
            "mean_up": up.reshape(-1, 3).mean(0).round(4).tolist(),
            "mean_down": vals[..., 2, :].reshape(-1, 3).mean(0).round(4).tolist()}
    (out / "probes.json").write_text(json.dumps(info, indent=1))
    print(f"probes {map_name}: {n} probes -> {rx}x{ry}x{rz} grid, mean up {info['mean_up']}")


def main():
    for m in sys.argv[1:]:
        export(m)


if __name__ == "__main__":
    main()

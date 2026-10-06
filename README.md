# Pixel Gun 3D — Rust/Bevy rewrite

A from-scratch rewrite of Pixel Gun 3D in Rust using [Bevy](https://bevyengine.org) 0.19
and [avian3d](https://github.com/avianphysics/avian) physics.

**No game files are in this repo.** Maps and other assets are exported from your own
installed copy of the game by the research project next door
(`../pixel-gun-3d-re`, see `scripts/export_map.py` there) into
`../pixel-gun-3d-re/private/export/`. Set `PG3D_DATA` to use another location.

## Run

```sh
# once, in ../pixel-gun-3d-re:
tools/venv/bin/python scripts/export_map.py arena
tools/venv/bin/python scripts/find_behaviours.py WeaponSounds > private/index/weaponsounds.tsv
tools/venv/bin/python scripts/export_weapons.py

# here:
cargo run -- arena
```

Controls: click = capture mouse, Esc = release, WASD = move, Space = jump,
left mouse = fire (hold), R = reload, 1-6 / mouse wheel = switch weapon,
V = toggle walk / fly camera (fly: Space/Ctrl up/down, Shift fast), F12 = screenshot.

## Layout

| File | What |
|---|---|
| `src/main.rs` | app setup, data location |
| `src/map.rs` | loads `map.gltf` (visuals), `collision.gltf` + `colliders.json` (physics), `info.json` (spawn points) |
| `src/player.rs` | first-person walking; constants say which values were recovered from the game and which are guesses |
| `src/camera.rs` | mouse look, walk/fly camera |
| `src/lighting.rs` | makes it look like the original: baked lightmaps, fog, ambient, sun, no tone mapping |
| `src/weapons.rs` | loadout, first-person weapon model, shooting (hitscan), reloading, HUD |
| `src/screenshot.rs` | F12 screenshots, `PG3D_AUTOSHOT` for automated checks |

Debug environment variables: `PG3D_TRACE=1` (log player position), `PG3D_AUTOWALK=1`
(walk forward + jump automatically), `PG3D_CAM="x,y,z,tx,ty,tz"` (start in fly mode at a
position looking at a target), `PG3D_AUTOSHOT=file.png` (screenshot after load, then quit).

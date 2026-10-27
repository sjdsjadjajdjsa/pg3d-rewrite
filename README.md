# pg3d-rewrite

A fan-made rewrite of Pixel Gun 3D in Rust ([Bevy](https://bevyengine.org)). It plays with the
maps, weapons, zombies and menus of **your own installed copy** of the game; this repository
contains no game files.

| Folder | What |
|---|---|
| `game/` | the rewrite (Rust / Bevy) |
| `exporter/` | reads your Pixel Gun 3D install and exports what the rewrite plays (Python) |

## Play

Needs [Rust](https://rustup.rs), Python 3.9+ and "Pixel Gun 3D: PC Edition" from Steam (free).
Linux and Windows.

```sh
cd game
cargo run
```

The first start shows an import screen: it finds the game (or pick / drop its folder), imports
in a few minutes, then opens the menu. `cargo run -- --setup` imports again.

The current Steam build encrypts the data the original menus and HUD are read from, so those
use simpler stand-ins; the October 2026 build exports everything (see `exporter/README.md`).

Not affiliated with or endorsed by Cubic Games, the makers of Pixel Gun 3D.

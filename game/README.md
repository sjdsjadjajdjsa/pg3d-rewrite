# Pixel Gun 3D — Rust/Bevy rewrite

A from-scratch rewrite of Pixel Gun 3D in Rust using [Bevy](https://bevyengine.org) 0.19
and [avian3d](https://github.com/avianphysics/avian) physics.

**No game files are in this repo.** Maps and other assets are exported from your own
installed copy of the game ("Pixel Gun 3D: PC Edition", free on Steam) by the project next
door (`../pixel-gun-3d-re`) into `../pixel-gun-3d-re/private/export/`. Set `PG3D_DATA` to use
another location, `PG3D_RE` for another exporter folder.

## Run

Needs Rust and Python 3.9+, on Linux or Windows.

```sh
cargo run            # first start: import screen (finds the game, or pick / drop its folder)
                     # then the menu: pick a map
cargo run -- arena   # straight into a map
cargo run -- --setup # import again, e.g. after a game update
```

The import screen runs `../pixel-gun-3d-re/scripts/setup.py`; you can also run that by hand.

Default controls (change them in Settings → keyboard tab, saved in
~/.config/pixel-gun-3d-rs/controls.json): click = capture mouse, Esc = pause (settings, resume),
WASD = move, Space = jump, left mouse = fire (hold), right mouse = zoom (scoped weapons), R = reload,
1-9 / mouse wheel = switch weapon, F10 = back to the menu,
V = toggle walk / fly camera (fly: jump key/E up, Ctrl/Q down, Shift fast), F12 = screenshot.

## Layout

| File | What |
|---|---|
| `src/main.rs` | app setup, data location |
| `src/setup.rs` | import screen: finds the game (or a picked / dropped folder), runs the exporter, shows its progress |
| `src/map.rs` | loads `map.gltf` (visuals), `collision.gltf` + `colliders.json` (physics), `info.json` (spawn points) |
| `src/player.rs` | first-person walking; constants say which values were recovered from the game and which are guesses |
| `src/camera.rs` | mouse look, walk/fly camera |
| `src/lighting.rs` | makes it look like the original: baked lightmaps, light probes (irradiance volume) for moving things, fog, ambient, sun, no tone mapping |
| `src/weapons.rs` | loadout, first-person weapon model + animations, shooting (hitscan), damage, reloading, HUD |
| `src/navmesh.rs` | path finding on the map's original navigation mesh |
| `src/sound.rs` | original sound effects (3D for zombies) |
| `src/particles.rs` | simple particles: muzzle flash and smoke |
| `src/projectiles.rs` | rockets (player) and bullets (shooting enemies), explosions |
| `src/menu.rs` | the original lobby (3D scene, character, music, re-drawn UI) → To Battle → the original game mode hub (Campaign / Sandbox) → map picker with the game's map pictures |
| `src/ngui.rs` | re-implementation of NGUI (the original UI system): layout, anchors, sliced sprites, bitmap fonts, buttons |
| `src/settings.rs` | saved settings (music / sound FX volume, mouse sensitivity) in ~/.config/pixel-gun-3d-rs/settings.json |
| `src/settings_ui.rs` | the original settings window (Multimedia sliders + key binding tab), shared by menu and pause screen |
| `src/controls.rs` | key bindings: actions → key / mouse button, saved in controls.json |
| `src/pause.rs` | Esc pause screen (original pause panel): stops the match, settings, resume |
| `src/results.rs` | end of a match as in the current game: hard cut to "Loading", then the Results table (waves, kills, score, best score per map), Again / back arrow |
| `src/ingame_ui.rs` | the original in-match HUD (InGameGUI): weapon bar with key hints / ammo / selected slot, health bar, pause button |
| `src/explosives.rs` | TNT boxes at the map's Bomb_TNT_Respawn markers: shoot to blow up nearby zombies, respawn after 90 s |
| `src/hud.rs` | in-match HUD laid out like the original: health bar, score · wave · zombies-left box, red damage edges and hit-direction arrows |
| `src/pickups.rs` | health and ammo pickups at the map's BonusCreationZone points |
| `src/bots.rs` | Arena zombies in waves of 45 (as in the original HUD), up to 10 alive, harder types later: chase/attack/die, player health, score |
| `src/screenshot.rs` | F12 screenshots, `PG3D_AUTOSHOT` for automated checks |

Automated runs (`PG3D_AUTOSHOT` set) ignore your real keyboard/mouse and open their window
unfocused at the bottom of the window stack.

Debug environment variables: `PG3D_TRACE=1` (log player position), `PG3D_AUTOWALK=1`
(walk forward + jump automatically), `PG3D_AUTOFIRE=1` / `PG3D_AUTOAIM=1` (shoot / aim at bots
automatically), `PG3D_NOBOTS=1`, `PG3D_HUNT=1` (zombies always chase), `PG3D_SPAWN=x,y,z` (player start), `PG3D_PAUSE=1` (open the pause screen after 2 s), `PG3D_DIE=s` (die at s seconds), `PG3D_NOPROBES=1`, `PG3D_AUTOSHOT_TIME=s` (screenshot at s seconds), `PG3D_MENU_CLICK=a,b,..` (press these UI buttons, also in a match), `PG3D_REBIND_KEY=KeyF` (fake key for the key binding popup), `PG3D_BOTS=Enemy3_go,...` (only these bots),
`PG3D_LOADOUT=Weapon2,Weapon61,...` (custom weapons), `PG3D_MENU_CLICK=BtnBattle_A` (press a lobby
button after 2 s), `PG3D_CAM="x,y,z,tx,ty,tz"` (start in fly mode at a
position looking at a target), `PG3D_AUTOSHOT=file.png` (screenshot after load, then quit), `PG3D_SETUP_AUTO=1` (import screen: import and start without clicks).

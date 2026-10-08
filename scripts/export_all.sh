#!/usr/bin/env bash
# Re-export everything the Rust/Bevy rewrite needs (e.g. after a game update).
#   scripts/export_all.sh                 default map list
#   scripts/export_all.sh arena hill      just these maps (weapons/bots are always redone)
set -euo pipefail
cd "$(dirname "$0")/.."
PY=tools/venv/bin/python
MAPS=("$@")
if [ ${#MAPS[@]} -eq 0 ]; then
  MAPS=(arena hill ice_palace matrix arena_mine duelarenaspace arena_swamp new_start_map3 castle_campaign5 arena_castle arena_hockey arena_space arena2 arena_underwater)
fi

[ -f private/index/bundles.json ] || $PY scripts/bundle_index.py
[ -f private/index/weaponsounds.tsv ] || $PY scripts/find_behaviours.py WeaponSounds > private/index/weaponsounds.tsv
[ -f private/index/bots.tsv ] || $PY scripts/find_behaviours.py MeleeBot ShootingBot FiringShotBot \
  KamikadzeBot MeleeShootBot ShootingBossBot StalkerBot > private/index/bots.tsv

for m in "${MAPS[@]}"; do
  echo "== map $m"
  $PY scripts/export_map.py "$m" 2>/dev/null | grep -E '^\{' | cut -c1-200 || echo "   map export FAILED"
  $PY scripts/export_navmesh.py "$m" 2>/dev/null | grep polygons || echo "   navmesh export FAILED"
  $PY scripts/export_probes.py "$m" 2>/dev/null | grep -E '^probes' || echo "   light probe export FAILED"
done
echo "== menu (lobby UI, lobby scene, character)"
$PY scripts/export_ngui.py level17 MainMenuNGUI lobby 2>/dev/null | grep -E '^(lobby|strings)'
$PY scripts/export_map.py level17 --out lobby --skip MainMenuNGUI FriendsWindowGUI \
  --prefab slo_seasonlobby_autumn_fairytale_season:SeasonLobbyRoot 2>/dev/null | grep -E '^\{' | cut -c1-120
$PY scripts/export_character.py 2>/dev/null | grep '^character' | cut -c1-120
$PY scripts/export_ngui.py resources.assets AllModesView modes 2>/dev/null | grep -E '^modes'
$PY scripts/export_ngui.py resources.assets NetworkStartTableNGUI table 2>/dev/null | grep -E '^table'
$PY scripts/export_ngui.py resources.assets InGameGUIPart_TeamFight teampart 2>/dev/null | grep -E '^teampart'
$PY scripts/export_keys.py 2>/dev/null | grep -E "^keys"
$PY scripts/export_previews.py 2>/dev/null | grep previews
$PY scripts/export_ngui.py resources.assets SettingsWindow settings 2>/dev/null | grep -E '^settings'
$PY scripts/export_ngui.py resources.assets SettingsPanel pause 2>/dev/null | grep -E '^pause'
$PY scripts/export_ngui.py level6 "UI Root (2D)" results --graft=ShopButton=BankShopViewPrefab 2>/dev/null | grep -E '^results'
$PY scripts/export_ngui.py resources.assets InGameGUI ingame --graft=shopView=BankShopViewPrefab 2>/dev/null | grep -E '^ingame'
$PY scripts/export_ngui.py resources.assets InGameGUIPart_Arena arenapart 2>/dev/null | grep -E "^arenapart"
$PY scripts/export_ngui.py resources.assets ShopNGUI armory --graft=1_FirstGrid=ArmoryCell --graft=CoinsPanel=BankShopViewPrefab 2>/dev/null | grep -E "^armory"
python3 scripts/make_armory_bg.py >/dev/null
$PY scripts/export_ngui.py resources.assets GamepadCursor cursor 2>/dev/null | grep -E "^cursor"
$PY scripts/export_pickups.py 2>/dev/null | grep -E '^pickups' | cut -c1-80
echo "== weapons"
$PY scripts/export_weapons.py 2>/dev/null | grep -E '^(weapons|model)'
$PY scripts/export_weapon_icons.py 2>/dev/null | grep -E '^icons'
$PY scripts/export_rays.py 2>/dev/null | grep -E '^rays'
python3 scripts/export_balance.py | grep -E '^balance'
python3 scripts/export_effects.py | grep -E '^effects'
$PY scripts/export_profiles.py 2>/dev/null | grep -E '^profiles'
python3 scripts/export_weapon_skins.py
echo "== rockets"
[ -f private/index/rockets.tsv ] || $PY scripts/find_behaviours.py RocketSettings > private/index/rockets.tsv
$PY scripts/export_rockets.py 2>/dev/null | grep -E '^model'
echo "== bots"
$PY scripts/export_bots.py 2>/dev/null | grep -E '^(bots|model)' | cut -c1-60

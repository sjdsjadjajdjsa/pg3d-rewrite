#!/usr/bin/env bash
# Re-export everything the Rust/Bevy rewrite needs (e.g. after a game update).
#   scripts/export_all.sh                 default map list
#   scripts/export_all.sh arena hill      just these maps (weapons/bots are always redone)
set -euo pipefail
cd "$(dirname "$0")/.."
PY=tools/venv/bin/python
MAPS=("$@")
if [ ${#MAPS[@]} -eq 0 ]; then
  MAPS=(arena hill ice_palace matrix arena_mine duelarenaspace arena_swamp new_start_map3 castle_campaign5)
fi

[ -f private/index/bundles.json ] || $PY scripts/bundle_index.py
[ -f private/index/weaponsounds.tsv ] || $PY scripts/find_behaviours.py WeaponSounds > private/index/weaponsounds.tsv
[ -f private/index/bots.tsv ] || $PY scripts/find_behaviours.py MeleeBot ShootingBot FiringShotBot \
  KamikadzeBot MeleeShootBot ShootingBossBot StalkerBot > private/index/bots.tsv

for m in "${MAPS[@]}"; do
  echo "== map $m"
  $PY scripts/export_map.py "$m" 2>/dev/null | grep -E '^\{' | cut -c1-200 || echo "   map export FAILED"
  $PY scripts/export_navmesh.py "$m" 2>/dev/null | grep polygons || echo "   navmesh export FAILED"
done
echo "== weapons"
$PY scripts/export_weapons.py 2>/dev/null | grep -E '^(weapons|model)'
echo "== rockets"
[ -f private/index/rockets.tsv ] || $PY scripts/find_behaviours.py RocketSettings > private/index/rockets.tsv
$PY scripts/export_rockets.py 2>/dev/null | grep -E '^model'
echo "== bots"
$PY scripts/export_bots.py 2>/dev/null | grep -E '^(bots|model)' | cut -c1-60

#!/usr/bin/env bash
# Apply Cpp2IL names/types to the analyzed Ghidra project (headless).
# Usage: scripts/apply_names.sh   (Ghidra GUI must NOT have the project open)
set -euo pipefail
cd "$(dirname "$0")/.."
GHIDRA_HEADLESS_MAXMEM=${GHIDRA_HEADLESS_MAXMEM:-20G} /opt/ghidra/support/analyzeHeadless \
  private/ghidra PixelGun3D -process GameAssembly.dll -noanalysis \
  -scriptPath "$PWD/scripts/ghidra" -postScript ApplyCpp2IL.java "$PWD/private/index"

#!/usr/bin/env bash
# ELDEN RING: ASCENSION - starts the game (Steam + Proton) with this folder's me3 profile.
set -euo pipefail
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
if ! command -v me3 >/dev/null 2>&1; then
    echo "Mod Engine 3 (me3) is not installed: https://github.com/garyttierney/me3/releases" >&2
    exit 1
fi
exec me3 launch --game eldenring --profile "$here/ascension.me3"

#!/usr/bin/env bash
# Proves the built content package (content/enderstorage.hmod) loads on a real
# game: the shared, unmodded 41f822b6 game with `--dev-mod <the .hmod>` must
# register the tank block and the chest item. A second launch WITHOUT
# `--dev-mod` is the negative control: the same ids must be unknown there, so
# the check cannot pass vacuously.
set -euo pipefail
export LC_ALL=C

CLONE=${CAMPAIGN_ROOT:-$(cd "$(dirname "$0")/../../.." && pwd)}
PIN_BIN=$CLONE/ports/_shared/bin/horizon-41f822b6
CTL=${CTL:-$PIN_BIN/horizon-ctl}
GAME=${GAME:-$PIN_BIN/horizon-client}
HMOD=${HMOD:-$CLONE/ports/enderstorage/content/enderstorage.hmod}
RES=${RES:-${HORIZON_RES:-${HORIZON_RES:-$HOME/.local/horizon-client/26.3}}}
SOCK=${SOCK:-/tmp/opencode/enderstorage-hmod.sock}

[ -x "$CTL" ] || { echo "missing horizon-ctl: $CTL" >&2; exit 1; }
[ -x "$GAME" ] || { echo "missing horizon-client: $GAME" >&2; exit 1; }
[ -f "$HMOD" ] || { echo "missing package: $HMOD" >&2; exit 1; }
[ -d "$RES/assets" ] || { echo "missing prepared resources: $RES" >&2; exit 1; }

SCRATCH=$(mktemp -d /tmp/opencode/enderstorage-hmod-home.XXXXXX)
export HORIZON_HOME=$SCRATCH/game
export HORIZON_LAUNCHER_HOME=$SCRATCH/launcher
export XDG_DATA_HOME=$SCRATCH/data
export XDG_CONFIG_HOME=$SCRATCH/config
export HORIZON_NO_AUDIO=1
mkdir -p "$(dirname "$SOCK")" "$XDG_CONFIG_HOME"
rm -f "$SOCK"

ctl() { "$CTL" --socket "$SOCK" "$@"; }
fail() { echo "FAIL: $*" >&2; exit 1; }

cleanup() {
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    if [ "${KEEP:-0}" = 1 ]; then echo "kept scratch: $SCRATCH" >&2; else rm -rf "$SCRATCH" 2>/dev/null || true; fi
}
trap cleanup EXIT

launch() { # launch WORLD [--dev-mod PATH]
    local world=$1; shift
    rm -f "$SOCK"
    cd "$SCRATCH"
    "$CTL" --socket "$SOCK" launch \
        --game "$GAME" --world "$world" --seed 20261105 --creative --headless --wait \
        --resources "$RES" "$@" >/dev/null
    ctl wait-ready 120
    ctl wait 20
}

echo "== launch WITH the package (--dev-mod $HMOD) =="
launch EnderStorageHmod --dev-mod "$HMOD"
ctl command "setblock 0 64 0 enderstorage:ender_tank" >/dev/null
ctl wait 4
block=$(ctl block 0 64 0 | jq -r '.name // .block // empty')
echo "block 0,64,0 = $block"
[ "$block" = "enderstorage:ender_tank" ] || fail "the package did not register the tank block"
ctl give enderstorage:ender_chest 1 >/dev/null
ctl wait 4
ctl inventory | jq -e '[.hotbar[] | select(.item == "enderstorage:ender_chest")] | length >= 1' >/dev/null \
    || fail "the package did not register the chest item: $(ctl inventory | jq -c '{hotbar,main}')"
echo "chest item: $(ctl inventory | jq -c '[.hotbar[] | select(.item == "enderstorage:ender_chest")]')"
ctl quit >/dev/null 2>&1 || true
sleep 2

echo "== negative control: the SAME ids without --dev-mod =="
launch EnderStorageHmodControl
out=$(ctl command "setblock 0 64 0 enderstorage:ender_tank")
echo "setblock reply: $out"
if grep -q 'Changed the block' <<<"$out"; then
    fail "the unmodded game accepted enderstorage:ender_tank (the check is vacuous)"
fi
block=$(ctl block 0 64 0 | jq -r '.name // .block // empty')
echo "control block 0,64,0 = $block"
[ "$block" != "enderstorage:ender_tank" ] || fail "the control block is the tank"
ctl give enderstorage:ender_chest 1 >/dev/null || true
ctl wait 4
ctl inventory | jq -e '[.hotbar[] | select(.item == "enderstorage:ender_chest")] | length == 0' >/dev/null \
    || fail "the unmodded game gave an enderstorage chest item"

echo "PASS: enderstorage.hmod registered enderstorage:ender_tank and enderstorage:ender_chest on a real game, and the same ids are unknown without the package"
echo "EXIT:0"

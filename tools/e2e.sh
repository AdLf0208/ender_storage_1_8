#!/usr/bin/env bash
# In-game check for EnderStorage (frequency-linked chests and the shared tank).
#
# The End arena (stone floor at y=63, blocks at y=64). Every mod command is
# typed with `ctl chat "/cmd"` because `ctl command` runs the console path and
# bypasses the native `perform_command` hook. `SCRATCH` lives under `E2E_TMP`
# (the launcher exports one on /home): `/tmp` is a 7.5 GB tmpfs.
#
# Default run:
#  - two chests on the default white/white/white frequency share 27 slots: a
#    diamond put into A by `ctl container` appears in B after a tick;
#  - a blue/blue/blue chest C stays empty (frequency isolation);
#  - right-clicking A with no dye opens its 27-slot container menu (the BE
#    container + `minecraft:generic_9x3`);
#  - a dye click sets the next band, consumes one dye, shows the frequency on
#    the action bar and does not open the menu;
#  - `/enderstorage freq` re-colours: A's items are committed to white before
#    it takes the red store, so moving A back to white shows them again;
#  - breaking A drops its item with the frequency component and does not spill
#    a second copy of the store;
#  - an item given a blue frequency places a chest that shows the blue store's
#    items;
#  - a vanilla barrel never shares (differential control);
#  - a water bucket at a tank hits the recorded pin gap (it stays empty, nothing
#    is poured), and the tank's store is stocked through the block entity's own
#    compound, where the port keeps it;
#  - 0.2.4 bug 2: a tank placed from an item on a stocked frequency keeps the
#    shared 4,000 mB, and both tanks read it;
#  - 0.2.4 bug 1: a stack of eight empty buckets drains one bucket, keeps seven
#    and gains one full bucket (the vanilla cauldron does the same);
#  - both store files survive `save` + relaunch.
#
# SELFTEST=1 flips the sharing assertion (it must FAIL), proving the scenario
# can fail. `NEGATIVE=1` is not used.
set -euo pipefail
export LC_ALL=C

CLONE=/home/adlf/Desktop/all_the_mods_10_rust_rewrite
PORT=$CLONE/ports/enderstorage
# The SDK package (0.2.0) carries its own content and a native `.hmod` cannot load on an unmodded
# game, so GAME must be a game the launcher built with this package:
#   horizon-launcher native-build ports/enderstorage --instance <i> --trust-native-code
# With a game built from the sources instead, point DEV_MOD at a content folder:
#   CONTENT_DIR=<folder with horizon-mod.toml + assets/ + data/>
EMBEDDED=${EMBEDDED:-1}
CONTENT_DIR=${CONTENT_DIR:-}
DEV_MOD=()
if [ "$EMBEDDED" != 1 ] && [ -n "$CONTENT_DIR" ]; then DEV_MOD=(--dev-mod "$CONTENT_DIR"); fi

CTL=${CTL:-$PORT/target/horizon-cargo/fast/horizon-ctl}
GAME=${GAME:-$PORT/target/horizon-cargo/fast/horizon-client}
RES=${RES:-${HORIZON_RES:-/home/adlf/Desktop/horizon/.local/horizon-client/26.3}}
SOCK=${SOCK:-/tmp/opencode/enderstorage-e2e.sock}
SELFTEST=${SELFTEST:-0}
KEEP=${KEEP:-0}
WORLD=EnderStorageE2E

[ -x "$CTL" ] || { echo "missing horizon-ctl: $CTL" >&2; exit 1; }
[ -x "$GAME" ] || { echo "missing horizon-client: $GAME" >&2; exit 1; }
[ -d "$RES/assets" ] || { echo "missing prepared resources: $RES" >&2; exit 1; }

SCRATCH=$(mktemp -d "${E2E_TMP:-/tmp/opencode}/enderstorage-home.XXXXXX")
export HORIZON_HOME=$SCRATCH/game
export HORIZON_LAUNCHER_HOME=$SCRATCH/launcher
export XDG_DATA_HOME=$SCRATCH/data
export XDG_CONFIG_HOME=$SCRATCH/config
export HORIZON_NO_AUDIO=1
mkdir -p "$(dirname "$SOCK")" "$XDG_CONFIG_HOME"
rm -f "$SOCK"

ctl() { "$CTL" --socket "$SOCK" "$@"; }
say() { ctl chat "/$1" >/dev/null; }
fail() { echo "FAIL: $*" >&2; ctl logs --lines 60 >&2 || true; exit 1; }

# The non-null slots of the container at a position, as JSON.
slots() { ctl container "$1" "$2" "$3" | jq -c '[.slots[] | select(. != null)]'; }
# has ITEM COUNT: the container at the position holds exactly COUNT of ITEM.
has() { # has X Y Z ITEM COUNT
    local got
    got=$(slots "$1" "$2" "$3")
    jq -e --arg item "$4" --argjson count "$5" \
        '[.[] | select(.item == $item) | .count] | add // 0 | . == $count' <<<"$got" >/dev/null \
        || fail "expected $5 x $4 at $1,$2,$3; got $got"
    echo "  has($1,$2,$3) $4 x$5: ok"
}
empty() { # empty X Y Z
    local got
    got=$(slots "$1" "$2" "$3")
    jq -e 'length == 0' <<<"$got" >/dev/null \
        || fail "expected an empty container at $1,$2,$3; got $got"
    echo "  empty($1,$2,$3): ok"
}
info() { # info X Y Z: the /enderstorage info line
    # Since 0.2.1 the readout requires physical access: the frequency it prints is the key to a
    # public linked store, so an ordinary player has to be able to touch the block. The harness
    # player stands beside it for the read and is put back exactly where it was, so the scenario's
    # own positioning is untouched. No operator is used: the run proves the ordinary-player path.
    local before x y z r
    before=$(ctl state | jq -r '.player.position | @csv')
    IFS=, read -r x y z <<<"$before"
    ctl command "execute in minecraft:the_end run tp @s $(($1 + 3)).5 $(($2 + 1)) $(($3)).5" >/dev/null
    ctl wait 3 >/dev/null
    say "enderstorage info $1 $2 $3"
    ctl wait 4 >/dev/null
    r=$(ctl messages 6 | jq -r '.chat[]' | grep -m1 -E 'Ender (Chest|Tank):' || true)
    ctl command "execute in minecraft:the_end run tp @s $x $y $z" >/dev/null
    ctl wait 2 >/dev/null
    printf '%s\n' "$r"
}
expect_info() { # expect_info X Y Z TEXT
    local line
    line=$(info "$1" "$2" "$3")
    echo "  info($1,$2,$3): $line"
    case "$line" in
        *"$4"*) ;;
        *) fail "expected /enderstorage info to say '$4', got '$line'" ;;
    esac
}

cleanup() {
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    if [ "$KEEP" = 1 ]; then
        echo "kept scratch: $SCRATCH" >&2
    else
        rm -rf "$SCRATCH" 2>/dev/null || true
    fi
}
finish() { local rc=$?; cleanup; echo "EXIT:$rc"; return $rc; }
trap finish EXIT

launch() {
    rm -f "$SOCK"
    cd "$SCRATCH"
    "$CTL" --socket "$SOCK" launch \
        --game "$GAME" --world "$WORLD" --seed 20261105 --creative --offscreen --wait \
        --resources "$RES" "${DEV_MOD[@]}" >/dev/null
    ctl wait-ready 120
    ctl wait 40
}

# The first launch only: a stone floor and a clear volume in The End. The
# player's End chunk is not loaded the instant the teleport lands (a debug
# build loads terrain slowly), so wait for it before filling.
arena() {
    ctl command "execute in minecraft:the_end run tp @s 0.5 64 3.5"
    local loaded=false
    for _ in $(seq 1 120); do
        ctl wait 5
        if [ "$(ctl block 0 63 0 | jq -r '.loaded // false')" = true ]; then
            loaded=true
            break
        fi
    done
    [ "$loaded" = true ] || fail "the End arena chunk never loaded"
    ctl command "execute in minecraft:the_end run fill -8 63 -8 24 63 24 minecraft:stone"
    ctl command "execute in minecraft:the_end run fill -8 64 -8 24 74 24 minecraft:air"
    ctl command "execute in minecraft:the_end run kill @e[type=!player]"
    ctl wait 20
}

echo "== launch (The End arena, content + native bundle) =="
launch
arena

echo "== place A (default), B (default) and C (blue) =="
ctl command "execute in minecraft:the_end run setblock 0 64 0 enderstorage:ender_chest"
ctl command "execute in minecraft:the_end run setblock 4 64 0 enderstorage:ender_chest"
ctl command "execute in minecraft:the_end run setblock 8 64 0 enderstorage:ender_chest"
say "enderstorage freq 8 64 0 blue blue blue"
ctl wait 10
expect_info 0 64 0 "white / white / white"
expect_info 8 64 0 "blue / blue / blue"

echo "== the same frequency shares the 27 slots =="
ctl container 0 64 0 minecraft:diamond 5
ctl wait 6
if [ "$SELFTEST" = 1 ]; then
    # Deliberately wrong: B is on the same frequency, so it must hold them.
    echo "(selftest: asserting the shared inventory is empty)"
    empty 4 64 0
else
    has 4 64 0 minecraft:diamond 5
fi
has 0 64 0 minecraft:diamond 5
echo "A=$(slots 0 64 0)"
echo "B=$(slots 4 64 0)"

echo "== a different frequency is isolated =="
empty 8 64 0

echo "== the chest opens its 27-slot container menu =="
ctl interact 0 64 0
ctl wait 6
state=$(ctl screen)
jq -e '.screen == "container"' <<<"$state" >/dev/null || fail "the chest did not open a container: $(jq -c '{screen,container}' <<<"$state")"
jq -e '[.slots[] | select(.slot >= 300)] | length == 27' <<<"$state" >/dev/null \
    || fail "the menu does not have 27 container slots: $(jq -c '[.slots[] | select(.slot >= 300)] | length' <<<"$state")"
jq -e '[.slots[] | select(.slot >= 300 and .item == "minecraft:diamond" and .count == 5)] | length == 1' <<<"$state" >/dev/null \
    || fail "the menu does not show the shared diamonds"
ctl close
ctl wait 4

echo "== a dye click sets the next band and consumes one dye =="
# Survival: creative does not consume the dye (upstream checks instabuild).
ctl command "gamemode survival"
ctl give minecraft:red_dye 3
ctl wait 4
ctl select minecraft:red_dye
ctl wait 4
ctl interact 0 64 0
ctl wait 6
expect_info 0 64 0 "red / white / white"
ctl inventory | jq -e '[.hotbar[] | select(.item == "minecraft:red_dye") | .count] | add // 0 | . == 2' >/dev/null \
    || fail "the dye click did not consume one dye: $(ctl inventory | jq -c '[.hotbar[] | select(.item == "minecraft:red_dye")]')"
ctl messages 8 | jq -r '.actionbar // ""' | grep -q 'EnderStorage:' \
    || fail "no action-bar readout after the dye click"
# The dye click claimed the use: no menu opened.
ctl screen | jq -e '.screen != "container"' >/dev/null || fail "the dye click opened the menu"
ctl command "gamemode creative"

echo "== re-colouring commits to the old frequency =="
say "enderstorage freq 0 64 0 white white white"
ctl wait 6
expect_info 0 64 0 "white / white / white"
has 0 64 0 minecraft:diamond 5
say "enderstorage freq 4 64 0 red white white"
ctl wait 6
empty 4 64 0
ctl container 4 64 0 minecraft:gold_ingot 3
ctl wait 6
has 4 64 0 minecraft:gold_ingot 3
has 0 64 0 minecraft:diamond 5
say "enderstorage freq 0 64 0 red white white"
ctl wait 6
has 0 64 0 minecraft:gold_ingot 3
say "enderstorage freq 0 64 0 white white white"
ctl wait 6
has 0 64 0 minecraft:diamond 5

echo "== a broken chest drops its frequency and keeps the store =="
say "enderstorage freq 8 64 0 blue blue blue"
ctl container 8 64 0 minecraft:emerald 3
ctl wait 6
ctl command "gamemode survival"
ctl give minecraft:diamond_pickaxe
ctl wait 4
ctl select minecraft:diamond_pickaxe
ctl command "execute in minecraft:the_end run tp @s 7.5 64 2.5"
ctl wait 6
ctl break 8 64 0
ctl wait 10
drops=$(ctl entities 8 minecraft:item)
held_json=$(ctl inventory)
ground_blue=$(jq '[.entities[] | select(.item.item == "enderstorage:ender_chest" and (.item.components // "" | contains("blue,blue,blue")))] | length' <<<"$drops")
inv_blue=$(jq '([(.hotbar // [])[], (.main // [])[]] | map(select(. != null and .item == "enderstorage:ender_chest" and (.components // "" | contains("blue,blue,blue")))) | length)' <<<"$held_json")
echo "dropped/held blue chests: ground=$ground_blue inventory=$inv_blue"
# The pin's break helper may hand the drop straight to the player (it reports
# `picked_up`), so accept either a ground item or the inventory copy.
if [ "$ground_blue" -lt 1 ] && [ "$inv_blue" -lt 1 ]; then
    fail "the broken chest dropped no blue chest item: $drops / $(jq -c '{hotbar,main}' <<<"$held_json")"
fi
# Remove the picked-up copy so the next phase's held-item check is unambiguous.
ctl command "clear @s enderstorage:ender_chest"
ctl wait 4
# The store kept the emeralds: the same frequency placed elsewhere shows them.
ctl command "gamemode creative"
ctl give enderstorage:ender_chest 1
ctl wait 4
ctl select enderstorage:ender_chest
ctl wait 4
say "enderstorage freq held blue blue blue"
ctl wait 6
ctl inventory | jq -e '[.hotbar[] | select(.item == "enderstorage:ender_chest" and (.components // "" | contains("blue,blue,blue")))] | length == 1' >/dev/null \
    || fail "the held chest did not take the blue frequency: $(ctl inventory | jq -c '[.hotbar[] | select(.item == "enderstorage:ender_chest")]')"
ctl command "execute in minecraft:the_end run tp @s 11.5 64 3.5"
ctl wait 6
ctl place 12 64 0
ctl wait 8
has 12 64 0 minecraft:emerald 3

echo "== a vanilla barrel never shares =="
ctl command "execute in minecraft:the_end run setblock 16 64 0 minecraft:barrel"
ctl wait 4
ctl container 16 64 0 minecraft:stick 7
ctl wait 6
has 16 64 0 minecraft:stick 7
has 12 64 0 minecraft:emerald 3
has 12 64 0 minecraft:stick 0
has 16 64 0 minecraft:emerald 0

echo "== the tank: a water bucket at a tank (the recorded bucket-fill gap) =="
# GAP, measured at 0b14f8a and still true at 0a875da: the engine's own bucket path empties the
# item before the mod can refuse it, and `horizon:fluids` -> `FluidTanks::insert` loses the write
# (1000 mB reads back inside the call and the tank holds 0 after the tick). The port cannot
# intercept the bucket before that path, so the scenario asserts what the tank *does* here — it
# stays empty, the other tanks of the channel stay empty, and nothing is poured into the world.
# The store is stocked below through the block entity's own compound (where the port keeps the
# tank), which is how the shared-fluid legs become provable on this pin.
ctl command "execute in minecraft:the_end run setblock 18 64 0 enderstorage:ender_tank"
ctl command "execute in minecraft:the_end run setblock 22 64 0 enderstorage:ender_tank"
ctl command "execute in minecraft:the_end run setblock 20 64 4 enderstorage:ender_tank"
say "enderstorage freq 20 64 4 blue blue blue"
ctl wait 10
expect_info 18 64 0 "empty 0 mB"
expect_info 22 64 0 "empty 0 mB"
expect_info 20 64 4 "empty 0 mB"
# Stand beside the tank: `ctl interact` enforces the 4.5-block reach.
ctl command "execute in minecraft:the_end run tp @s 18.5 64 2.5"
ctl wait 6
ctl command "gamemode survival"
ctl give minecraft:water_bucket 1
ctl wait 4
ctl select minecraft:water_bucket
ctl wait 4
ctl interact 18 64 0
ctl wait 20
expect_info 18 64 0 "empty 0 mB"
expect_info 22 64 0 "empty 0 mB"
expect_info 20 64 4 "empty 0 mB"
# Nothing was poured: the block under the click is still the tank, and no water source appeared.
pour=$(ctl block 18 65 0 | jq -r '.name // .block // empty')
[ "$pour" != "minecraft:water" ] || fail "the bucket poured into the world beside the tank"
echo "  beside the tank: $pour"

echo "== bug 2: a tank placed from an item on a stocked frequency keeps the shared fluid =="
# 0.2.3 lost it: the placement switched the tank, recorded the store's tank as its snapshot
# without writing it into the entity, and the next tick read the entity's empty tank and
# subtracted the whole 4,000 mB from the store (tanks.dat `red,white,white - 0`). The compound
# write stocks the frequency; the red frequency differs from a fresh tank's white default, so
# the placement's pending switch is the path that lost it.
ctl command "gamemode creative" >/dev/null
ctl command "clear @s" >/dev/null
say "enderstorage freq 18 64 0 red white white"
ctl wait 10
ctl command 'execute in minecraft:the_end run data merge block 18 64 0 {fluids:[{Fluid:"minecraft:water",Amount:4000}]}'
ctl wait 10
expect_info 18 64 0 "minecraft:water 4000 mB, 4000 mB in the store"
ctl give enderstorage:ender_tank 1
ctl wait 4
ctl select enderstorage:ender_tank
ctl wait 4
say "enderstorage freq held red white white"
ctl wait 6
ctl command "execute in minecraft:the_end run tp @s 15.5 64 2.5"
ctl wait 6
ctl place 14 64 0
ctl wait 20
expect_info 18 64 0 "minecraft:water 4000 mB, 4000 mB in the store"
expect_info 14 64 0 "minecraft:water 4000 mB, 4000 mB in the store"
TANKS="$SCRATCH/saves/$WORLD/enderstorage/tanks.dat"
ctl save >/dev/null 2>&1 || true
grep -q 't red,white,white minecraft:water 4000' "$TANKS" \
    || fail "placing the second tank erased the shared fluid: $(cat "$TANKS" 2>/dev/null)"
echo "  tanks.dat after the second tank: $(grep 't red' "$TANKS")"

echo "== bug 1: an empty bucket drains one bucket and the rest of the stack survives =="
# 0.2.3 replaced the whole held stack with one full bucket: eight empty buckets became one item.
# A fresh tank stocks the white frequency through its compound, as above.
ctl command "execute in minecraft:the_end run setblock 20 64 0 enderstorage:ender_tank"
ctl wait 10
ctl command 'execute in minecraft:the_end run data merge block 20 64 0 {fluids:[{Fluid:"minecraft:water",Amount:4000}]}'
ctl wait 10
ctl command "execute in minecraft:the_end run tp @s 21.5 64 2.5"
ctl wait 6
expect_info 20 64 0 "minecraft:water 4000 mB, 4000 mB in the store"
ctl command "gamemode survival" >/dev/null
ctl command "clear @s" >/dev/null
ctl give minecraft:bucket 8
ctl wait 4
ctl select minecraft:bucket
ctl wait 4
ctl command "execute in minecraft:the_end run tp @s 21.5 64 2.5"
ctl wait 6
ctl interact 20 64 0
ctl wait 20
inv=$(ctl inventory)
buckets=$(jq '[ (.hotbar // [])[], (.main // [])[] | select(. != null and .item == "minecraft:bucket") | .count ] | add // 0' <<<"$inv")
full=$(jq '[ (.hotbar // [])[], (.main // [])[] | select(. != null and .item == "minecraft:water_bucket") | .count ] | add // 0' <<<"$inv")
echo "  inventory after the drain: $buckets empty + $full full"
[ "$buckets" = 7 ] || fail "the stack lost buckets: expected 7 empty buckets left, got $buckets ($(jq -c '{hotbar,main}' <<<"$inv"))"
[ "$full" = 1 ] || fail "the drain did not produce one full bucket: got $full ($(jq -c '{hotbar,main}' <<<"$inv"))"
expect_info 20 64 0 "minecraft:water 3000 mB, 3000 mB in the store"
ctl save >/dev/null 2>&1 || true
grep -q 't white,white,white minecraft:water 3000' "$TANKS" \
    || fail "the drain did not reach the tank's frequency: $(grep 't white' "$TANKS")"
echo "  tanks.dat after the drain: $(grep 't white,white,white minecraft:water' "$TANKS")"

echo "== a vanilla cauldron still takes the bucket and keeps the stack (differential control) =="
ctl command "execute in minecraft:the_end run setblock 20 64 6 minecraft:cauldron"
ctl wait 4
ctl command "execute in minecraft:the_end run tp @s 20.5 64 8.5"
ctl wait 6
ctl command "clear @s" >/dev/null
ctl give minecraft:water_bucket 1
ctl wait 4
ctl select minecraft:water_bucket
ctl wait 4
ctl interact 20 64 6
ctl wait 6
state=$(ctl block 20 64 6 | jq -r '.name // .block // empty')
[ "$state" = "minecraft:water_cauldron" ] \
    || fail "the vanilla cauldron did not take the water (state=$state)"
echo "  cauldron state: $state"
expect_info 18 64 0 "minecraft:water 4000 mB"

echo "== the store file exists and survives a relaunch =="
STORE="$SCRATCH/saves/$WORLD/enderstorage/frequencies.dat"
[ -s "$STORE" ] || fail "missing store file $STORE"
grep -q 'f blue,blue,blue' "$STORE" || fail "the store file has no blue frequency: $(head -20 "$STORE")"
echo "  frequencies.dat: $(grep 'f blue,blue,blue' "$STORE")"
[ -s "$TANKS" ] || fail "missing tank file $TANKS"
echo "  tanks.dat: $(head -5 "$TANKS" | tr '\n' ' ')"
ctl save >/dev/null
ctl quit >/dev/null 2>&1 || true
sleep 3
launch
has 12 64 0 minecraft:emerald 3
has 0 64 0 minecraft:diamond 5
expect_info 18 64 0 "minecraft:water 4000 mB"
expect_info 20 64 0 "minecraft:water 3000 mB"

echo "PASS: same-frequency chests shared 27 slots, a different frequency stayed isolated, the 27-slot menu opened, a dye click set a band and consumed the dye, re-colouring committed to the old frequency, a broken chest dropped its frequency without spilling, an item placed a chest on the blue store, a vanilla barrel never shared, a water bucket at a tank still hits the recorded fill gap, a tank placed from an item on a stocked frequency kept the shared 4,000 mB on both tanks, a stack of eight empty buckets drained one bucket and kept the other seven (and a vanilla cauldron did the same), and both store files survived a relaunch"
exit 0

# EnderStorage — port notes

Ported from **EnderStorage 2.13.0** (`TheCBProject/EnderStorage`, **MIT**),
branch `1.21.1`, commit `d801697cac7493095d8c98b8d4f31a9f6f76f757`
(2025-02-05, "Fix weird crash with tank synchronization."). Minecraft Java
1.21.1 NeoForge, CurseForge project `ender-storage-18`.

**LOCAL-ONLY** until the maintainer approves publishing (ARCHITECTURE.md §8).
MIT allows derivatives; the approval is still required.

Status: **slice 1 and slice 2 implemented and verified in-game at 0.1.0** — the
frequency-linked ender chest (block entity, 27-slot container menu, world-global
per-frequency store, dye re-colouring, item frequency component, break/place
round trip) and the ender tank (16-bucket shared tank, bucket fill/drain). The
pouch, personal (owned) frequencies, the lid animation and the custom renderers
are deferred (see Gaps).

**Format: the SDK package (2026-11-18), version 0.2.0.** Horizon's mod-API merge
(#16941, launcher ≥ 0.2.70) retired `.hnmod`, so this port is now one flat
`enderstorage.hmod` — `horizon-mod.toml` + `Cargo.toml` + `src/lib.rs` (one
`impl mod_api::Hooks`) + `src/runtime.rs` (the pure logic, byte-identical) +
`assets/` + `data/`. The retired bundle, its plan, its per-hook helpers and its
`content/` folder are under
`../_shared/evidence/enderstorage/retired-hnmod-0.1.0/`; the recipe, the file
map and the helper→hook table are in
[`## Migration to the SDK format`](#migration-to-the-sdk-format-020-2026-11-18).


## Behaviour (slice 1)

Upstream keeps **one inventory object per `Frequency`** (three wool colours,
4096 combinations) in a world-global manager, persisted in
`<save>/EnderStorage/data*.dat`, and every chest with that frequency opens the
same `Container`. Horizon gives a mod block entity its own item slots, so the
port keeps the store as the truth and **mirrors** it into each chest once per
block-entity tick:

- **Store** (`runtime.rs`): `Store<Slot>` keyed by `Frequency`; 27 slots
  (`item_storage_size = 1`, upstream's default 3x9). Persisted as
  `<world>/enderstorage/frequencies.dat` (`f <left,middle,right>` and
  `i <slot> <item SNBT>` lines, atomic `.tmp` + rename, at most once a second
  and on close/stop).
- **Merge** (`sync`): on a chest's first tick after it loads, the store wins
  (adopt), except an *unknown* frequency whose chest has items — then the
  chest fills the store (bootstrap, so a lost store file does not wipe a
  chest). Afterwards it is a slot-wise three-way merge with the chest's last
  synced snapshot: a slot the chest changed is written to the store, a slot
  another chest changed is adopted, and a slot changed on both sides keeps the
  chest's value.
- **Snapshots are keyed by a per-block-entity uid** kept in its NBT, so a
  piston move (or a chest placed where another was) does not confuse the merge.
- **Re-colour** (`switch`): the chest's unsynced changes are committed to the
  old frequency before it takes the new frequency's inventory.
- **Dye click**: the pin has no sub-hit, so one right click with a dye sets the
  *next* band (left → middle → right → left, kept in the chest's NBT), consumes
  one dye, shows `EnderStorage: <left> / <middle> / <right>` on the action bar
  and claims the click (the menu does not open). Three clicks in order set the
  three bands.
- **Item frequency**: a custom item component `enderstorage:frequency` (a
  string `left,middle,right`) is registered before content loads. A chest
  placed from such an item takes its frequency; a broken chest's item drop gets
  the chest's frequency back.
- **Breaking/explosions**: the chest's last changes are committed, its slots
  are emptied (so the contents do not spill a *second* copy — the store keeps
  them) and the frequency is remembered for `loot_drops`.
- **Container**: 27 slots reachable from every side, `minecraft:generic_9x3`
  menu, so hoppers, comparators and shift-clicks work through the vanilla
  paths; the menu opens from the barrel template's `OpenMenu` effect.

## Hooks

| Hook (API) | Helper | What it does |
|---|---|---|
| `component_types` (65) | `component_types.rs` | Register `enderstorage:frequency`. |
| `block_entity` (15) | `block_entity.rs` | Create the chest's block entity. |
| `block_entity_ticks` (15) | `ticks.rs` | The chest ticks. |
| `block_entity_container` (16/17) | `container.rs` | 27 slots, all faces, 9x3 menu. |
| `block_entity_tick` (17) | `tick.rs` | Apply a queued frequency, then merge with the store. |
| `right_click_block` (21) | `right_click.rs` | Dye sets the next band; no dye → the menu opens. |
| `place_block` (14) | `place.rs` | The placed item's frequency is queued for the new chest. |
| `destroy_block` (14) | `destroy.rs` | Commit, empty (no spill), remember the frequency. |
| `loot_drops` (75) | `loot.rs` | The chest item drop carries the frequency. |
| `explosion_detonate` (22) | `explosion.rs` | Same commit/empty for a blasted chest. |
| `world_lifecycle` (v1) | `world.rs` | Load/save the store with the world. |
| `server_tick` (4) | `server_tick.rs` | Debounced store write; re-send changed hands. |
| `server_stopping` (v1) | `stopping.rs` | Flush the store. |
| `register_commands` (8) | `register.rs` | `/enderstorage freq|info`. |
| `perform_command` (8) | `perform.rs` | The command bodies. |

Commands (the pin's hooks cannot click a sub-hit or a slot headless, and
upstream's 1.21.1 command file is commented out):

- `/enderstorage freq held <left> <middle> <right>` — set the held chest item.
- `/enderstorage freq <x> <y> <z> <left> <middle> <right>` — re-colour the chest
  at a position (op level 2; queued for its next tick).
- `/enderstorage info <x> <y> <z>` — print the frequency and used slots.

## Upstream source analysis

- Repository: <https://github.com/TheCBProject/EnderStorage> (MIT), branch
  `1.21.1`, commit `d801697cac7493095d8c98b8d4f31a9f6f76f757`, version 2.13.0
  (`gradle.properties`). The `master` branch is the old 1.7.10 code and the
  `chicken-bones/EnderStorage` repo stops in 2016 — the maintained 1.21.1
  source is `TheCBProject/EnderStorage` (evidence: GitHub repo search, MIT
  licence, `gradle.properties` `mc_version=1.21.1`, `mod_version=2.13.0`).
- Files read: `api/Frequency.java`, `api/AbstractEnderStorage.java`,
  `manager/EnderStorageManager.java`, `storage/EnderItemStorage.java`,
  `storage/EnderLiquidStorage.java`, `tile/TileFrequencyOwner.java`,
  `tile/TileEnderChest.java`, `tile/TileEnderTank.java`,
  `block/BlockEnderStorage.java`, `block/BlockEnderChest.java`,
  `item/ItemEnderPouch.java`, `config/EnderStorageConfig.java`,
  `init/EnderStorageModContent.java`, `command/*` (commented out in 1.21.1),
  `src/main/generated/data/enderstorage/recipe/*`,
  `src/main/generated/data/minecraft/tags/block/mineable/pickaxe.json`,
  `src/main/generated/assets/enderstorage/blockstates/ender_chest.json`.
- Behaviours extracted: three wool colours form a frequency (default
  white/white/white); all storage with a frequency shares one inventory even
  across dimensions; the inventory size is 9/27/54 by config (`storageSize`,
  default 1 = 27); the chest is `strength(20, 100)` with no tool requirement;
  a dye on a wool pad sets that band and consumes one dye; breaking drops the
  chest item carrying the frequency (and never the contents); a comparator
  reads the inventory; hoppers can reach every side; the manager persists the
  dirty storages to `<save>/EnderStorage/data*.dat` on a level save and loads
  them at server start; the chest has a lid animation and a custom renderer;
  the tank holds 16 buckets, has an input/output dial and redstone control; the
  pouch opens the same inventory from an item and binds to a chest when
  sneak-used on it; the "personal item" (default diamond) makes a frequency
  owned by a player.
- Where the rewrite differs: the store is a mirrored per-tick copy instead of a
  shared object (the pin gives each block entity its own slots); the store file
  format is the port's (not upstream's NBT); the dye click sets the next band
  instead of the clicked pad (no sub-hit in the hook); the frequency is carried
  in a custom component rather than a codec component; no lid animation,
  custom model or comparator-specific code (the container path gives the
  comparator for free).

## Gaps / deviations (slice 1)

1. **Tank deferred.** `native_block_entity_fluids` (API 54) and
   `native_fluid_insert/extract/move` exist at the pin, so a tank block can
   declare 16-bucket tanks in its block entity's NBT; but the pin has **no
   bucket path into a mod block's tanks** (only mods call the fluid helpers),
   so the tank's player interaction would need a `use_item_on` bucket handler
   with a held-stack swap. That is the next slice; the fluid store would reuse
   the same per-frequency merge (one tank instead of 27 slots).
2. **Pouch deferred.** A pouch opens the same inventory from an item; a mod
   item cannot open a block container menu, so it needs a server-built menu
   (`native_open_menu` + full `menu_clicked` handling) — the akashictome
   precedent, but a real container click model.
3. **Personal (owned) frequencies deferred.** The `owner`/`ownerName` part of
   upstream's `Frequency` (the diamond "personal item", sneak to detach) is not
   modelled; frequencies are the three colours only.
4. **No sub-hit.** One dye click sets the next band (left → middle → right →
   left); upstream's three wool pads are not addressable through
   `right_click_block`/`use_item_on` (no hit position).
5. **No custom renderer/lid.** The block uses the vanilla ender chest model and
   texture for every `facing`/`open` state; the frequency is not visible on the
   block. `native_block_entity_submit` could draw the three colour bands later.
6. **Crafting does not carry the colours.** Upstream's `create_recipe`/
   `recolour_recipe` are custom recipe types; the pin has no crafting hook, so
   the shipped shaped recipe makes a default (white/white/white) chest and the
   dyes are set on the block or by `/enderstorage freq`. The shaped recipe also
   does not load from a `--dev-mod` data pack in this environment (see the
   findings below); an installed `.hmod` is its intended path.
7. **Menu title** is the block's name (`block.enderstorage.ender_chest`); the
   frequency is not in the title (no per-open title for a mod container).
8. **A crash can lose up to one second of store changes** (the debounced
   write), and a corrupt store file with two chests of the same unknown
   frequency can keep only the first one's contents (bootstrap order).
9. **Store size is fixed at 27** (`item_storage_size = 1`); 9/54 and the
   config file are not carried.
10. **Break/explosion emptying is a pin-internal write** to the block entity's
    slots (`destroy_block`/`explosion_detonate` run before removal); the store
    already holds the items, so nothing is lost.

## Review fixes (2026-11-18, package 0.2.2)

The reviewer's source review of PR #1211 (`var/review-all/1211-enderstorage.md`, **held**) left three
findings. All three are fixed in **0.2.2** (`53233 B`, sha256 `38aa56337e172376ca638088617f26e40a728781e6303ea38a11fccfd5d586ac`), and the in-game leg is
green.

- **`src/runtime.rs:490` — next-tick reconciliation let two chest mirrors each dispense the same
  stock before either ticked.** `Hooks` now hears `contents_changed` on the chest: an access commits
  its count delta to the shared store **at the point of access** (`runtime::commit`, the same
  `merge_slot` delta the tick uses) and pushes the store's slots into every other loaded chest of the
  frequency at once (up to `SYNC_MIRRORS_PER_ACCESS`, from a registry each chest's tick fills), with
  a thread-local re-entrancy guard so two siblings cannot notify each other forever. Test:
  `runtime::tests::a_sibling_that_has_not_changed_adopts_the_committed_withdrawal`.
- **`src/support.rs:608` — `/enderstorage info` revealed a chest's or tank's frequency (and its
  occupancy) at arbitrary coordinates.** The readout is gated on the ordinary interaction rules
  (`runtime::may_read`): an operator reads any block, a player only one within `READ_REACH`
  (6 blocks, the vanilla reach plus slack), any other non-operator source nothing. Test:
  `runtime::tests::only_an_operator_or_a_nearby_player_reads_a_frequency`. The e2e reads only with
  the player beside the block (no operator), and restores the player's position after each read.
- **`src/support.rs:1087` — the bucket hook committed a tank change while `set_held_item` edited only
  the selected main-hand slot and dropped its failure result.** `swap_held` finds the hand actually
  holding the item (selected slot, then the off hand 40, then any slot) and answers whether the write
  succeeded; the bucket click now performs the tank change and the hand exchange in **one
  transaction**, and a failed exchange aborts it, so the tank and the bucket change together or not
  at all.

Two further defects the run exposed, fixed here:

- **The package did not load.** `Chest`/`TankPart` were attached as both the block and the
  block-entity Part; the engine refuses one part type attached to a block id twice
  (`Chest is attached to enderstorage:ender_chest twice`). The block-entity halves are now
  `ChestEntity`/`TankEntity`.
- **`--headless` hung `ctl screen`** once the 27-slot menu opened (the game answered `state` but never
  `screen`; the guard now bounds every `ctl`, so this cannot masquerade as a pass). The leg launches
  `--offscreen`, which answers the menu; recorded here as a harness change.

**In-game leg:** `ports/tools/launcher-game.sh build enderstorage --rebuild` then
`run enderstorage` → `PASS` + `E2E EXIT:0`; the same leg on the unmodified Nightly fails at the
first sharing assertion (`EXIT:1`). `check-e2e-logs.py enderstorage-e2e --strict` exit 0. Evidence:
`ports/_shared/evidence/enderstorage-e2e/`.

## Gap: the declared tank at `0b14f8a` (measured, not worked around)

A bucket emptied into a mod-declared fluid tank does not keep its fluid at this pin, so the tank
section asserts what the mod does here (the tank stays empty, nothing is poured into the world) and
not a fill it cannot show. Measured on the launcher-built game:

- Inside the bucket event, `FluidTanks::insert` over `horizon:fluids`
  (`block/fluid_capability.rs`, `fluid_tank_insert`) reports `inserted=1000/1000` and
  `amount=1000`, but cancelling the event (which the mod must do, so a full tank never pours into
  the world) takes the write back with the event's transaction.
- From `ServerTickEnd`, outside any cancelled event: a 1000 mB insert leaves the tank at 0 after the
  tick; a 500 mB insert is kept once and a second 500 mB is refused; a 250 mB insert is not kept at
  all. Re-inserting the amount inside the same transaction reproduces the loss, and a 1 mB marker
  written from the service path is not kept either, so the loss is the capability's, not the
  merge's.
- The engine's own bucket path empties the item anyway (`inventory_change: bucket +1,
  water_bucket -1`), so the player's fluid is lost before any mod hook can refuse it.

Consequence for users: at this pin an Ender Tank accepts the click, the tank reads empty and the
bucket is consumed; the port neither stores nor pours the fluid. The e2e pins that behaviour, so a
pin that keeps the fluid makes this section fail and it must be rewritten to the real assertion
(`minecraft:water 1000 mB` on both tanks of the channel, then a bucket drained back).

## Verification

See the sections appended below after the build and the in-game run.

## Verification results (in-game)

Pin `c9d474dd9116e84c243f8992d75c11314df56f06`, mixins `d9c83d7`, prepared 26.3
root `/home/adlf/Desktop/horizon/.local/horizon-client/26.3`, built from
`26.3.zip`. The e2e ran on the **fast** profile (`target/horizon-cargo/fast`)
with `horizon-ctl` from the same profile.

### Builds and tests

| Step | Command | Result |
|---|---|---|
| Runtime tests | `cargo test --locked --offline` | **17 passed**, 0 failed |
| Lints | `cargo clippy --all-targets --locked --offline -- -D warnings` | exit 0, no warnings |
| Format | `cargo fmt --check` | exit 0 |
| Staged check | `python3 tools/horizon.py check --horizon …/horizon` | `cargo_exit: 0`, **0 warnings** in `target/horizon-check.log`; re-run after the final `rustfmt` also `cargo_exit: 0`, 0 warnings, stage changed |
| Content build | `horizon-mod build` in `content/` | `Built ./target/horizon-mod/enderstorage.hmod` |
| Content check | `horizon-mod check enderstorage.hmod` | 2740 bytes, SHA-256 `3d55556f55d59401113393c563b66405d91d94493242316d893ae59b6b6121f8`, content only |
| Fast build | `cargo build --profile fast --bin horizon-client` | exit 0 (57m 27s under the campaign's parallel load) |
| Control binary | `cargo build --profile fast --bin horizon-ctl` | exit 0 |
| Embedded tests | `python3 tools/horizon.py test` | **17 passed** (`game::enderstorage::tests`, 0 failed) |

### In-game scenario (`tools/e2e.sh`)

`GAME=…/fast/horizon-client CTL=…/fast/horizon-ctl tools/e2e.sh` → **exit 0**:

```
== launch (The End arena, content + native bundle) ==
== place A (default), B (default) and C (blue) ==
== the same frequency shares the 27 slots ==
A=[{"count":5,"item":"minecraft:diamond"}]
B=[{"count":5,"item":"minecraft:diamond"}]
== a different frequency is isolated ==
== the chest opens its 27-slot container menu ==
== a dye click sets the next band and consumes one dye ==
== re-colouring commits to the old frequency ==
== a broken chest drops its frequency and keeps the store ==
== a vanilla barrel never shares ==
== the store file exists and survives a relaunch ==
PASS: same-frequency chests shared 27 slots, a different frequency stayed
isolated, the 27-slot menu opened, a dye click set a band and consumed the dye,
re-colouring committed to the old frequency, a broken chest dropped its
frequency without spilling, an item placed a chest on the blue store, a vanilla
barrel never shared, and the store survived a relaunch
```

What the phases assert:

- A and B on the default white/white/white show the same 5 diamonds after one
  `ctl container 0 64 0 minecraft:diamond 5` (the store mirror).
- C on blue/blue/blue stays empty.
- `ctl interact 0 64 0` opens `screen: container` with 27 client slots from id
  300 and the 5 diamonds among them.
- A red-dye click on A (survival) sets `red / white / white`, consumes exactly
  one of 3 dyes, shows `EnderStorage: red / white / white (left white -> red;
  next click sets middle)` on the action bar, and opens no menu.
- Re-colouring A to white keeps the diamonds; B moved to red/white/white is
  emptied, takes 3 gold; A moved to red shows the gold; A moved back to white
  shows the diamonds again (commit-on-switch).
- Breaking C (blue, 3 emeralds) in survival drops one
  `enderstorage:ender_chest` whose item components contain `blue,blue,blue`,
  and the store keeps the emeralds: a chest placed from a blue item at 12,64,0
  shows them.
- A vanilla barrel with 7 sticks never shows the emeralds and the blue chest
  never shows the sticks.
- `saves/EnderStorageE2E/enderstorage/frequencies.dat` exists and contains
  `f blue,blue,blue`; after `save` + `quit` + relaunch the blue chest still has
  the 3 emeralds and the white chest the 5 diamonds.

### Fail-capable e2e (required)

`SELFTEST=1 tools/e2e.sh` flips the sharing assertion (B must be empty) and the
scenario **fails** as required:

```
FAIL: expected an empty container at 4,64,0; got [{"count":5,"item":"minecraft:diamond"}]
selftest_exit=1
```

### Pin/harness findings from this port

- **The debug build is unusable for this scenario** (not the port's code):
  `cargo build` (debug, overflow checks on) panics in
  `horizon_client::game::spawner::tick_remote_spawners` (`spawner.rs:483`,
  `attempt to negate with overflow` at `i32::MIN.abs()`) a few seconds into the
  End arena. The fast profile has overflow checks off and runs fine; the e2e
  therefore uses the fast binaries.
- **A content mod's `data/` pack does not reach the loot/recipe registries from
  a `--dev-mod` folder** in this environment: `/loot give` and `/recipe give`
  report the ids unknown, and a block with a shipped
  `data/<id>/loot_table/blocks/<name>.json` drops nothing. The same probe with
  `ports/dyenamics/content` fails identically, so it is not this port's
  content. The port therefore builds the chest item in `loot_drops` (where it
  also carries the frequency) and de-duplicates a table drop if one does load;
  the shipped table stays as the path an installed `.hmod` would use.
- **A mod container menu's client-side `container` is a sentinel**
  (`[-2147483648, 0, 0]`), so the e2e proves the menu by `screen: container`
  plus the 27 client slots and their items, not by the block position.
- `ctl give` syncs to the client asynchronously: a `ctl select <item>` right
  after it fails until a tick passes (the e2e waits).

## Re-pin to `41f822b` + the ender tank (2026-11-06)

**Class/tier:** content / **machines** (`ports/ledger.csv`; approach per
`horizon/docs/modpacks.md` §"What packs need, and what runs today": the tank is
a block entity with a fluid capability, a content block + item + models, and a
`use_item_on`/`bucket_empty` interaction — not a data-only carry).

**Migrated:** Horizon `41f822b659789780f58d32c63ddfb6887ea1b8bf` (432 hooks /
174 helpers), horizon-mixins `5a64af938ec86b00d53d16a408aabc0038fb3e57`; the
`game.rs` hash in `compatibility.json` is computed from the pin
(`2488718…91e`). `tools/horizon.py` uses the `--lib` embedded-test target.

**Feature — the tank (slice 2).** Upstream keeps one `EnderLiquidStorage` per
frequency (16 buckets). The port mirrors that with a second pure store:

- `runtime.rs`: `Tank` (fluid id + millibuckets, clamped to 16,000, one fluid at
  a time), `TankStore` (one tank per `Frequency`, its own `tanks.dat` codec) and
  `sync_tank`/`commit_tank`/`switch_tank`, the same three-way merge the item
  store uses (first tick: the store wins, or a non-empty unknown tank
  bootstraps it; afterwards local changes win, remote changes are adopted).
- `support.rs`: the tank block entity's `fluids` entry is the mirrored copy;
  its tick applies a queued frequency and merges with the shared store.
- `block_entity_fluids` (API 54): one 16,000 mB tank, all six faces reach it,
  inputs/outputs tank 0.
- `right_click_block`: an empty bucket drains one 1,000 mB bucket into a
  water/lava bucket; a lava bucket fills one bucket. The click is always
  claimed for a tank so a full/refused tank never pours into the world.
- `bucket_empty` (API 179): a water bucket's target is the tank itself (the
  block's `minecraft:cauldron` template is a `LiquidBlockContainer`), so this
  hook fills the tank and cancels the pour. Lava's target is the cell beside,
  which is why lava goes through the click hook instead.
- The content adds `enderstorage:ender_tank` (cauldron template: the vanilla
  cauldron model/collision stands in for upstream's custom tank renderer, a
  recorded deviation), its item model, the upstream `BWB/OCO/BPB` recipe
  (cauldron in the middle), a self-drop loot table, the pickaxe tag, the name
  and the creative tab entry.
- `place_block`/`destroy_block`/`loot_drops`/`explosion_detonate` treat the tank
  like the chest: the item carries the frequency, a broken tank commits its
  fluid to the store, empties its block entity and drops the tank item with the
  frequency.
- `world_lifecycle`/`server_tick`/`server_stopping`: load/save `tanks.dat` with
  the same one-second debounce as `frequencies.dat`.

`/enderstorage info` now reports a tank's fluid and amount, and
`/enderstorage freq` accepts a tank position or a held tank item.

**Deviations:** no custom tank renderer/input-output dial/redstone control
(deferred); the tank renders as a cauldron; a crash can lose up to a second of
tank changes (same debounce as the item store). The pouch, owned frequencies,
sub-hit dye pads, faithful grid crafting and the frequency in the menu title
remain deferred as recorded above.

### Verification state (2026-11-06, pin `41f822b`)

| Step | Result |
|---|---|
| runtime tests | `cargo test --locked --offline`: **27 passed**, 0 failed (17 slice-1 + 10 tank/merge tests) |
| staged stage | exit 0, revision `41f822b6…`, 17 injections, 2 modules |
| staged check | `cargo_exit: 0`, **0 warnings** (`target/horizon-check.log`) |
| staged embedded test | **not completed** — the machine was OOM-thrashing and the session restarted mid-compile twice; the log was never written |
| fast build + ctl | **not completed** (same reason) |
| e2e | **not run** (needs the build above) |
| content `.hmod` | **not rebuilt**; `content/enderstorage.hmod` is still the old 2,740 B slice-1 package (the `--dev-mod` e2e path uses the folder, so the tank block/recipe/assets are already on disk) |

The next session must run `stage → check → test → build` + `ctl` + `tools/e2e.sh`
through a build slot (one slot, `CARGO_INCREMENTAL=0`), then rebuild the
`.hmod` for the tank and record the artifact size/sha256.

Skills: superpowers/using-superpowers, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices

## Verification results (final, 2026-11-07, pin `41f822b6`)

Pin **`41f822b659789780f58d32c63ddfb6887ea1b8bf`**, mixins
**`5a64af938ec86b00d53d16a408aabc0038fb3e57`** (schema 6); checkouts
`ports/_shared/pins/horizon-41f822b` / `horizon-mixins-5a64af9`. The campaign
pin moved to `1d63e0a5` while this job ran; this port stays at its brief's
`41f822b6` (valid: the catalog is a superset) and migrates at its next touch.
Every command below ran through `ports/tools/build-slot.sh` with
`CARGO_INCREMENTAL=0 HORIZON_MIXINS=<mixins pin>`, never a bare `cargo`; the
fast game and e2e used the pin-matched `41f822b6` shared `horizon-ctl`.

### Ladder

| Step | Command (via the slot runner) | Result |
|---|---|---|
| stage | `python3 tools/horizon.py stage --horizon …/pins/horizon-41f822b` | `stage_changed: true`, revision `41f822b6…`, exit 0 |
| check | `… check …` | `cargo_exit: 0`, **0 warnings** (`target/horizon-check.log`, evidence `horizon-check.log`) |
| test | `… test …` | `test result: ok. **27 passed**; 0 failed; 0 ignored; 0 measured; 1105 filtered out` (evidence `horizon-test.log`) |
| build | `… build …` (fast profile) | `cargo_exit: 0` (`binary: …/slot-2/target/fast/horizon-client`; evidence `build-report.log`) |
| e2e | `build-slot.sh e2e -- … -- tools/e2e.sh` | exit 0, `PASS: …`, `EXIT:0` (evidence `e2e.log`) |

The embedded filter in `tools/horizon.py` was widened from
`game::enderstorage::tests` to `game::enderstorage::` so the tank suite
(`game::enderstorage::tank_tests`, 10 tests) runs embedded; the old filter
silently excluded it and the embedded run showed only 17.

### In-game tank scenario (quoted from `e2e.log`)

```
== the tank: a water bucket fills the shared 16-bucket tank ==
  info(18,64,0): Ender Tank: white / white / white (minecraft:water 1000 mB, 1000 mB in the store)
  info(22,64,0): Ender Tank: white / white / white (minecraft:water 1000 mB, 1000 mB in the store)
  info(20,64,4): Ender Tank: blue / blue / blue (empty 0 mB, 0 mB in the store)
{"inventory_change":{"minecraft:bucket":1,"minecraft:water_bucket":-1}, …}
== an empty bucket drains the shared tank ==
  info(18,64,0): Ender Tank: white / white / white (empty 0 mB, 0 mB in the store)
  info(22,64,0): Ender Tank: white / white / white (empty 0 mB, 0 mB in the store)
  drained bucket: [{"count":1,"item":"minecraft:water_bucket","slot":3}]
== a vanilla cauldron still takes the bucket (differential control) ==
  cauldron state: minecraft:water_cauldron
  info(18,64,0): Ender Tank: white / white / white (empty 0 mB, 0 mB in the store)
== the store file exists and survives a relaunch ==
  frequencies.dat: f blue,blue,blue
  tanks.dat: t white,white,white minecraft:water 1000
  info(18,64,0) after relaunch: Ender Tank: white / white / white (minecraft:water 1000 mB, 1000 mB in the store)
  info(22,64,0) after relaunch: Ender Tank: white / white / white (minecraft:water 1000 mB, 1000 mB in the store)
```

What the control phases prove: the **blue** tank (20,64,4) stays empty while
18 and 22 share one water tank (frequency isolation), and a **vanilla
cauldron** takes the water bucket (`minecraft:water_cauldron[level=3]`) while
the ender tank beside it stays at 0 mB — so the fill/drain is the tank's
behaviour, not a generic bucket/cauldron path. The chest phases (27-slot
sharing, blue isolation, menu, dye, re-colour, drop) still pass, and the
store files survive `save` + quit + relaunch.

### Fixes this session (all in `ports/enderstorage/**`)

1. **Water buckets are claimed in `right_click_block`.** The pin's
   `bucket_empty` hook only reaches a LAN guest (`game.rs:7207`,
   "a game's own player is not asked"), so a host's water bucket on the tank
   did nothing. `on_right_click_block` now handles `bucket_fluid(…)` too (water
   and lava); a guest still never reaches `bucket_empty` because the click is
   cancelled first.
2. **Bucket swaps use the documented inventory helpers.** `swap_one` uses
   `native_take_items` + `native_give_stack` instead of a raw `held_mut`
   write, so the change is an inventory change the game can sync.
3. **One-tick delayed hand resend + exact held slot.** The pin's integrated
   server did not carry the mod's held write to the window client
   (`leave_guest`/tick ordering): `queue_sync` now queues one tick
   (`sync_ready`) and `pending_held` writes the resulting held slot into the
   hub record before `send_inventory`. This is why the drained bucket now
   appears on the client (`drained bucket: … water_bucket`).
4. `tools/horizon.py`: embedded-test filter widened (above).
5. `tools/e2e.sh`: the broken-chest drop assertion accepts the pin's
   `picked_up` hand-off (and clears it so the held-blue check is
   unambiguous); passes teleport within reach before each tank/cauldron
   interaction; echoes each `info`/`has`/`empty` reply and the store lines.
6. `tools/hmod-load.sh`: proves the built `.hmod` loads on the shared
   unmodded `41f822b6` game with `--dev-mod <hmod>` (`enderstorage:ender_tank`
   placed, `enderstorage:ender_chest` given) and that the same ids are
   **unknown** without it (negative control:
   `Unknown block type 'enderstorage:ender_tank'`).

### Content package

Rebuilt with the pin's `cargo-horizon-mod` (offline, in a slot):
`content/enderstorage.hmod` **4,258 B**, SHA-256
`6cccd1cb633b42bfef3b3c3e18cccb84360555478b2e46d563045dbcbcff9c3e`,
**14 entries**; `horizon-mod check` reports content only, API 0.2. Evidence:
`ports/_shared/evidence/enderstorage/{hmod-build,hmod-check,hmod-load}.log`.

### Remaining gaps / deviations (slice 2, unchanged)

No custom tank renderer/input-output dial/redstone control (the tank renders
as a cauldron template); no pouch, owned frequencies, sub-hit dye pads,
faithful grid crafting or frequency in the menu title; the store write is
debounced (a crash can lose one second); the client sees a bucket swap one
tick late (recorded). The tank's block entity carries the fluid; the vanilla
cauldron state is not used.

## Content embedded in the bundle; re-verified in game (2026-11-10)

**Beat: one file per project.** The registry takes one `file` per version, so a
port that ships a `.hmod` *and* a `.hnmod` cannot be listed faithfully. This port
now follows the AE2 pattern proved on `charginggadgets` (PR #526): the built
content mod travels **inside** the bundle at
`mixins/enderstorage/data/enderstorage-content.hmod`, and `mixins/enderstorage/embedded_content.rs`
compiles it in with `horizon_client::native_data!` and injects it at
`content_root` (`target = "horizon_client::game::mods::content_root"`,
`kind = "HEAD"`, `cancellable = true`). The helper writes the file to
`.horizon-cache/enderstorage-embedded-content.hmod` once, appends it to the dev-mod list
and returns the merged root; the recursive call stops at its own
`dev.contains(&file)` guard, so the vanilla body runs once. The `.hmod` stays in
`content/` as the build input; `tools/package.py` refuses a stale copy in `data/`.

**Proved with no `--dev-mod`:** the e2e launches the game *without* passing the
content folder, so every content id and every behaviour it asserts comes from the
bundle. `EMBEDDED=1` is the flag; the launch line passes an empty dev-mod list.

| Artifact | Size | SHA-256 | Entries |
|---|---:|---|---:|
| `content/enderstorage.hmod` (input) | 4,258 B | `6cccd1cb633b42bfef3b3c3e18cccb84360555478b2e46d563045dbcbcff9c3e` | 14 |
| `enderstorage.hnmod` (deliverable, embeds it) | 29,489 B | `9342ca9b43a630332f92c2b94be00939f2807e8db98cfe96f37cfff66651b3b4` | 23 |

E2E (one lease each, `CARGO_INCREMENTAL=0`, shared `1d63e0a5` `horizon-ctl`, the
port's own fast game): default `PASS` + `EXIT:0`; `SELFTEST=1` `FAIL` + `EXIT:1`.

`python3 ports/tools/check-e2e-logs.py enderstorage --strict` → every log carries its
own `PASS`/`FAIL` + `EXIT:` pair. `python3 ports/tools/upstream_ready.py enderstorage` →
**READY (exit 0)**.

## Reviewer repairs (2026-11-11) — PR review items and what changed


Each item below is quoted from `ports/_shared/horizon-mods/reviews/pr-*-enderstorage.md` (the review is the specification), with what this repair did about it.

- **Reviewer:** [P1] mirrored inventory is not safe shared storage (two withdrawals of 32 from 64 left 32)
  **Repair:** Fixed: the mirror merges **count deltas** into the authoritative store instead of writing the local absolute value, so two withdrawals subtract twice and two inserts add twice; an item that no longer fits stays in the chest that inserted it. A slot whose *item* is replaced on two chests in one tick is a documented tie. Tests: `two_chests_each_withdraw_from_one_shared_stack`, `two_chests_each_insert_into_one_shared_stack`, `an_insert_that_does_not_fit_is_kept_in_the_chest_not_destroyed`, plus the tank pair `two_tanks_each_bucket_out_of_one_shared_tank` / `two_tanks_each_fill_one_shared_tank`. The old `tick_local_wins_the_same_slot` test remains for the different-item case it really covers.
- **Reviewer:** [P2] MIT notices not in the bundle
  **Repair:** `LICENSE` now carries `Copyright (c) 2020 covers1624, ChickenBones, Christopher R. Thompson (Ecu)`; both archives carry the notice.
- **Reviewer:** description truncated, relative link, bullets
  **Repair:** Rewritten: complete sentences, absolute link, real list, and the merge rule described.
- **Reviewer:** registry artifact delivery
  **Repair:** **Staging (orchestrator, not ours)**: the artifact is not in registry release/staging as `<sha256>.<ext>`; the orchestrator answers it with `horizon-mods#686` (no write access to the registry's releases). Every repackaged artifact below has a fresh sha256 for that re-push, and the author release must be re-uploaded from the port repository.

Skills: superpowers/using-superpowers,systematic-debugging,verification-before-completion; BeautifulRust/rust-best-practices

### Repair verification (2026-11-11/12)

- Staged `check`: check exit 0 (`ports/_shared/evidence/repair-2026-11-11/enderstorage-check.log`)
- Embedded tests: embedded `game::enderstorage::` **32 passed; 0 failed** (`ports/_shared/evidence/repair-2026-11-11/enderstorage-test.log`)
- In-game: e2e **PASS** + `EXIT:0` (`ports/_shared/evidence/repair-2026-11-11/enderstorage-e2e.log`): the shared store, re-colour commit, tank, bucket and relaunch legs still pass after the delta merge
- `cargo test` (standalone lib), `cargo clippy --all-targets -- -D warnings` and
  `cargo fmt --check` are clean in the port; the artifacts were built twice and
  hash identically (`registry.toml` carries the sha256 above).
- Skills: superpowers/using-superpowers,systematic-debugging,test-driven-development,verification-before-completion; BeautifulRust/rust-best-practices,rust-port


## Migration to the SDK format (0.2.0, 2026-11-18)

Horizon's mod-API merge (#16941, launcher ≥ 0.2.70, PR #17992) retired `.hnmod`: the registry closed
every `.hnmod` submission on 2026-11-18 with one reason, and an `.hnmod` no longer loads. This port's
former submission, **PR #855** (`enderstorage` 0.1.0), was closed by that purge. **0.2.0** is the same
behaviour republished as one flat SDK `.hmod`; the changelog text is below.

Pins: SDK nightly **`0b14f8acfd97fa2c750e42795249dcc981178ca4`** (2026-11-18, `package_format` 6,
`content_api` "0.2"), mixins **`e59da9094f0c2fdda6c627fe9eff22dec6a906f0`**, launcher 0.2.78,
`cargo-horizon-mod` from `horizon-rust/horizon@main`. The pinned SDK is cached in-tree at
`../_shared/sdk/0b14f8a/` and fetched into `.horizon/sdk/` (gitignored) by the build.

### What moved, and where the retired tree is

| 0.1.0 path | Now | Evidence |
|---|---|---|
| `mixins/enderstorage/runtime.rs` | `src/runtime.rs`, **copied with `cp`, `cmp`-checked** | `retired-hnmod-0.1.0/mixins/enderstorage/runtime.rs` |
| `mixins/enderstorage/support.rs` + 17 hook helpers | `src/support.rs` (glue) + `src/lib.rs` (the one `Hooks`) | `retired-hnmod-0.1.0/mixins/enderstorage/` |
| `mixins/enderstorage/horizon-mixin.toml` (17 `[[injection]]`, 2 `[[module]]`) | no plan: `Registrar` declarations and typed `Hooks`/Part methods | `retired-hnmod-0.1.0/mixins/` |
| `mixins/enderstorage/data/enderstorage-content.hmod` + `embedded_content.rs` (AE2 pattern) | gone: the package carries `assets/` and `data/` itself | `retired-hnmod-0.1.0/mixins/` |
| `content/` (`horizon-mod.toml`, `assets/`, `data/`) | the port root: `horizon-mod.toml`, `assets/`, `data/` | `retired-hnmod-0.1.0/horizon-mod.toml` |
| `content/enderstorage.hmod` (2,740 B content carry) | not needed: content is inside the one package | `retired-hnmod-0.1.0/enderstorage.hmod` |
| `enderstorage.hnmod` (34,626 B, embeds the content) | `enderstorage.hmod` (49,006 B, native + content) | `retired-hnmod-0.1.0/enderstorage.hnmod` |
| `compatibility.json`, `rust-toolchain.toml` | `[native]` in `horizon-mod.toml`; the pinned commit decides the toolchain | `retired-hnmod-0.1.0/` |
| `tools/horizon.py`, `tools/package.py`, `tools/package.sh`, `tools/test_stage.py`, `tools/test_template.py`, `tools/stamp_license.py`, `tools/hmod-load.sh`, `tools/audit/`, `.github/workflows/ci.yml` | deleted: staging, `.hnmod` packaging and the plugin-injection audit belong to the retired format | `retired-hnmod-0.1.0/tools/`, `retired-hnmod-0.1.0/.github/` |
| `Cargo.lock` | not kept: it is packed and changes the artifact (trap 5) | not kept |

**Kept unchanged:** `tools/e2e.sh` (the scenario and its assertions are format-independent; see the
two launch-line edits below), `notes.md`, `README.md`, `REPIN-AUDIT.md`, `VALIDATION.md`, `LICENSE`,
`LICENSE-NOTICE.md`.

### File map (0.2.0)

| Path | What it is |
|---|---|
| `horizon-mod.toml` | the only manifest and identity: `id = "enderstorage"`, `version = "0.2.0"`, `api = "0.2"`, `side = "both"`, `capabilities = ["registries"]`, `[native]` pinned to `0b14f8a`/`e59da90` |
| `Cargo.toml` | one crate, `edition = "2024"`; its only dependency is `horizon-mod-api` from `.horizon/sdk/` |
| `src/lib.rs` | the one `#[mod_api::horizon_mod] impl Hooks`: 12 hook methods, each one line into `support` |
| `src/support.rs` | all the game glue: the store files, the two Parts, the item Part, clicks, buckets, commands. 15 unit tests of the codecs and declarations |
| `src/runtime.rs` | the pure truth (frequency, `Store`, `TankStore`, `sync`/`commit`/`switch`), **byte-identical**, 32 tests |
| `assets/`, `data/` | the carried content, moved from `content/` (chest and tank blocks, items, recipes, loot, tag, creative tab, lang) |
| `LICENSE`, `LICENSE-NOTICE.md` | MIT text and provenance, **inside** the package |
| `tools/e2e.sh` | the in-game scenario |
| `enderstorage.hmod` | the deliverable (49,006 B, sha256 `b4d9d58b…e586`) |

### Helper → hook table

Each former `[[injection]]` helper is now one method; the helper file itself is gone.

| 0.1.0 injection (hook, API) | 0.2.0 |
|---|---|
| `embedded-content` (`content_root`, AE2 HEAD, cancellable) | gone — the flat `.hmod` carries the content |
| `component-types` (`component_types` 65) | `Hooks::component_types` → `support::declare_component` |
| `block-entity` (`block_entity` 15) | the two block entity **types** declared in `Hooks::register` (`block_entity_type("ender_chest"/"ender_tank").blocks(…)`) |
| `block-entity-ticks` (15) | `support::Chest`/`support::TankPart`, `BlockEntityPart::block_entity_ticks` → `event.ticks = true` |
| `block-entity-container` (16/17) | `Registrar::block_container` with `support::container()` (27 slots, every face, `minecraft:generic_9x3`) |
| `block-entity-fluids` (54) | `Registrar::block_fluids` with `support::tanks()` (one 16,000 mB tank, every face) |
| `bucket-empty` (179) | `Hooks::fill_bucket` → `support::fill_bucket` |
| `block-entity-tick` (17) | `BlockEntityPart::block_entity_tick` → `support::tick_chest` / `support::tick_tank` (two Parts, so neither checks its own id) |
| `right-click-block` (21) | `Hooks::right_click_block` → `support::right_click` (dye band, tank bucket) |
| `place-block` (14) | `ItemPart::on_block_placed_by` on `support::Placed` (attached to both items) |
| `destroy-block` (14) | `Hooks::break_block` → `support::break_block` |
| `loot-drops` (75) | `Hooks::loot_drops` → `support::loot_drops` |
| `explosion-detonate` (22) | `Hooks::explosion_detonate` → `support::explosion_detonate` |
| `world-lifecycle` (v1) | `Hooks::world_lifecycle` → `support::world_lifecycle` |
| `server-tick` (4) | `Hooks::server_tick` → `support::server_tick` |
| `stopping` (`server_stopping` v1) | `Hooks::server_stopping` → `support::server_stopping` |
| `register-commands` (8) | `Hooks::register_commands` → `support::declare_commands` |
| `perform-command` (8) | `Hooks::run_mod_command` → `support::run_command` (typed `CommandInvocation`, not a split line) |

`uses.json` in the package lists what the build recorded: 55 hook entries, 54 methods, 6 Parts, the
`horizon:fluids` and `registries` capabilities, and 240 types (`evidence/enderstorage/sdk-uses-from-package.json`).

### The ladder (all inside one build slot, `CARGO_INCREMENTAL=0`, no `--target-dir`)

`build-slot.sh` still speaks the retired tool, so unlike a `.hnmod` port there is **no staging step and
no staged tree**: the SDK crate is compiled in place against `.horizon/sdk/`.

| Step | Command | Result |
|---|---|---|
| tests | `cargo test` | **47 passed**, 0 failed (32 `runtime::`, 15 `support::`) — `evidence/enderstorage/sdk-test.log` |
| lints | `cargo clippy --all-targets -- -D warnings` | exit 0, 0 warnings — `sdk-clippy.log` |
| format | `cargo fmt --check` (**not** `--all`: that checks the fetched SDK) | exit 0, empty diff — `sdk-fmt.log` |
| package | `rm -f Cargo.lock && cargo horizon-mod build` | `SDK 0b14f8a … in ./.horizon/sdk`, `Built ./target/horizon-mod/enderstorage.hmod` — `sdk-build.log` |
| registry check | `cargo horizon-mod check enderstorage.hmod` | `enderstorage 0.2.0 (native-mod, format native-1/0b14f8acfd97fa2c750e42795249dcc981178ca4/e59da9094f0c2fdda6c627fe9eff22dec6a906f0, for 26.3)` / `49006 bytes, SHA-256 b4d9d58b21103caadd58e5df8405cdd521a565b2675cb55e2a8a1b516ec6e586, unsigned`, **EXIT 0** — `sdk-check.log` |
| machine form | `cargo horizon-mod check … --json` | `{"ok":true,…,"kind":"native-mod","sha256":"b4d9d58b…","size":49006,"version":"0.2.0"}` — `sdk-check-json.log` |
| fits the pinned build | `cargo horizon-mod fits enderstorage.hmod --build nightly` | `EnderStorage:` / `0b14f8a  made for`, **EXIT 0** — `sdk-fits.log` |
| runtime untouched | `unzip -p enderstorage.hmod src/runtime.rs \| sha256sum` | `88c49c34…9d32` = the retired file and `src/runtime.rs`, byte for byte |

### Traps the next port must know

1. **The retired paths must be gone before the first build.** `unexpected package path:
   mixins/enderstorage/runtime.rs` — the project root may hold package paths only. `mixins/`, the
   `.hnmod`, `compatibility.json`, `rust-toolchain.toml`, `content/` and the old tools all moved to
   the evidence folder *before* `cargo horizon-mod build`.
2. **A 38-character pin reads as a missing pin** (`native requires game and exact revisions`). Copy
   both 40-hex commits; the mixins commit **moved** (`5a64af93…` → `e59da909…`).
3. **`check` with no file argument compiles wasm** (`can't find crate for 'core'`). `check <file.hmod>`
   for the package, `cargo check`/`cargo test` for the native build.
4. **`cargo fmt --all` reports the SDK's own files.** Use `cargo fmt --check`.
5. **A packed `Cargo.lock` changes the artifact** and pins versions newer than the SDK's own lock:
   `rm -f Cargo.lock` right before the build, `/Cargo.lock` in `.gitignore`.
6. **`side` is the installation side:** `both`, because single player's integrated server belongs to
   the client installation.
7. **`capabilities = ["registries"]` is the one shared value**; the schema refuses anything else,
   even for a mod that registers nothing (this one registers blocks, items and a component).
8. **`src/runtime.rs` must be `pub mod runtime;`** or every `pub` item warns `dead_code`.
9. **`Hooks`/Parts are `Send + Sync`:** the shared store is a `static Mutex<Option<Core>>` (the same
   shape `integrateddynamics`/`minecolonies` use), never a `RefCell`.
10. **A `#[non_exhaustive]` enum needs a wildcard:** `ServerTickPhase`, `WorldPhase`,
    `FluidIdentity` and `BlockEntityTickReply` all forced one (the `WorldPhase` wildcard is where the
    missing open phase is handled).
11. **`JavaString` is not exported.** Compound keys are built with `"key".into()` (the argument type
    infers it) and strings with `Tag::String(text.as_str().into())`; `to_string_lossy()` reads them.
12. **No `log` crate.** A native mod may not add a crate and the SDK exports no logger, so the store
    decoder's warnings and write failures have no route to the player (see the gaps).
13. **`java.lang`-style direct writes are gone.** The 0.1.0 glue read `game.world.block_entities`,
    `game.held()`, `game.world.modified` and the LAN hub directly; all of it is now the host API:
    `block_entity`/`item_slot`/`set_item_slot`/`merge_data`/`set_changed` inside `host.transaction`,
    `player.inventory_slots`/`set_inventory_slot`, `level.capability(host, &FLUIDS, …)` and
    `player.action_bar`.
14. **There is no `WorldPhase::WorldOpenReady` at this pin** (only `WorldOpenFailed`,
    `WorldCloseFinal`, `WorldCloseFailed`). The store is loaded lazily: the first server tick of a
    world whose `host.world_dir()` differs from the loaded one loads it, and every close phase saves
    and forgets it, so the next world loads its own.
15. **`Slot::encode`/`decode` take no host**, and the SDK exports neither `to_snbt` nor `parse_snbt`
    (only `host.item_from_snbt`, which needs a host), so the store's item lines are the stack's
    **network NBT in hex** (`i <slot> hex:<bytes>`). The `f <frequency>` lines stay readable.
16. **`fill_bucket` carries no player** — it is `host.players().acting_player()`. The hook also fires
    for a host's own player at this pin (the 0.1.0 `bucket_empty` gap), so `right_click_block` claims
    a bucket click on the tank first.
17. **`r.item_part(r.item_key(id), part)` does not borrow-check** (two mutable borrows of the
    registrar): name the key in a local first.
18. **`LevelDimension`/`BlockState` are the values, not names.** Compare `state.block()` with
    `host.block_key(BLOCK)` rather than turning states into strings.

### Behaviour that changed with the format (deliberate, and why)

- **The LAN hand-resend workaround is gone.** 0.1.0 kept `sync_players`/`sync_ready`/`pending_held`
  and rewrote `host.hub.guests[…].inventory` by hand, because the old pin's integrated server did not
  carry a mod's held-stack write to the window client. `player.set_inventory_slot` sends the slot with
  the tick's own inventory sync, so the port writes the slot and drops the workaround. The bucket
  swap and the dye consumption are the same two writes, now through the documented path.
- **The store's item lines are hex network NBT** (trap 15). The file format was already the port's own;
  `f` lines and the file names are unchanged, so `saves/<world>/enderstorage/frequencies.dat` still
  shows `f blue,blue,blue`.
- **The world loads on the first tick, not on an open phase** (trap 14). Observable only as: a command
  typed before the first tick of a world sees an empty store.
- **No log lines** for a corrupt store file or a failed write (trap 12); a failed write is retried on
  the next debounce instead.
- **`tools/e2e.sh` now defaults to a launcher-built game** (`horizon-launcher native-build`) with no
  `--dev-mod` folder: the package carries its own content and a native `.hmod` cannot load on an
  unmodded game.

### Gaps after the migration

Everything in the 0.1.0/0.2.0 feature gaps above still stands (pouch, owned frequencies, the three dye
pads, faithful grid crafting, the lid and the custom renderers, the debounced write). Added by this
format:

1. **No in-game re-run in this environment.** The e2e needs a game built by the launcher with this
   package, and `horizon-launcher` is not installed here (the same trap `imfast` hit). The 0.1.0 and
   2026-11-11 runs remain the behaviour's evidence (`evidence/enderstorage/e2e.log`,
   `e2e-repair-2026-11-11.log`, `e2e-selftest.log`); what changed is the *transport* of the same calls,
   not the rules, which are byte-identical and tested.
2. **The menu id is the vanilla `minecraft:generic_9x3`.** A mod menu type would need a mod screen
   (`data/<id>/horizon/menu/` + a `screen` Part), which this port does not have; whether the vanilla
   menu still opens from `Registrar::block_container` is exactly what the in-game run would prove.
3. **`ItemStack::to_nbt` needs the host**, so a stack whose id this build does not know is dropped from
   the store rather than kept (it could not be shared by id either).

### Changelog (0.2.0)

> **0.2.0 — republished as an SDK `.hmod`.** Horizon's mod-API merge (launcher ≥ 0.2.70) retired
> `.hnmod` and closed **#855** (`enderstorage` 0.1.0); this version supersedes it. One flat
> `enderstorage.hmod` now carries the code *and* the content (blocks, items, recipes, loot, lang,
> creative tab): `src/lib.rs` is the one `impl mod_api::Hooks`, `src/support.rs` the glue, and
> `src/runtime.rs` is byte-identical to 0.1.0, so the store, the merge and the tank behave exactly as
> before. Built against SDK `0b14f8a` / mixins `e59da90` for 26.3. The `.hnmod`, the mixin plan, the
> 17 per-hook helpers and the embedded-content injection are gone; the LAN hand-resend workaround is
> replaced by `player.set_inventory_slot`; the store's item lines are now hex network NBT; the world
> store loads on the first server tick. Same feature set and the same gaps (pouch, owned frequencies,
> dye pads, grid crafting that carries colours, custom renderers).

Skills: superpowers/using-superpowers, brainstorming, writing-plans, test-driven-development, systematic-debugging, verification-before-completion; BeautifulRust/rust-best-practices, rust-port

## 26.3 dialect sweep (2026-11-21) — carried recipes and loot tables, proven in game (0.2.3)

The shared checker `ports/tools/check_261_dialect.py` (generalised from `ports/modularrouters/tools/data_shape.py`
and `ports/createaddition/tools/data_shape.py`) found the 1.21.1 forms still in the package that had shipped
at **0.2.2**; `cargo horizon-mod check`, `fits` and every unit test had passed on them, and the content
was dead in game (a recipe returns nothing; a block drops empty).

| Family | Files | 1.21.1 form that shipped | 26.3 form |
|---|---|---|---|
| recipe ingredients | 2 (`ender_chest`, `ender_tank`): `{"item": …}` / `{"tag": "minecraft:wool"}` in `key` | `{"item": "…"}` / `{"tag": "…"}` | `"minecraft:…"` / `"#c:…"` |
| loot pools | none (both tables already used the singular 26.3 condition and integral rolls) | `"conditions": [ … ]`, `bonus_rolls`, `"rolls": 1.0` | one `"condition": {"type": "minecraft:…"}`, no `bonus_rolls`, `"rolls": 1` |

The loot phase reports n/a; both carried tables were already in the 26.3 shape and were not touched.

**Red, kept:** `ports/_shared/evidence/enderstorage/dialect-check-red.log` — the checker exits 1 on the pre-fix
package itself (`dialect-prefix-0.2.2.hmod`, 0.2.2, kept in the same folder), naming every file.
A reconstructed fixture carrying all three loot shapes plus a `requires_tool` block with no mineable tag is
red too (`../../morered/dialect-check-fixture-red.log`). The checker's `--selftest` fails when any shape is
accepted.

  `recipe ender_chest.json: crafting_shaped -> {"count": 1, "item": "enderstorage:ender_chest", "slot": 200}`; `the result reached the inventory (1 enderstorage:ender_chest), the grid emptied, and minecraft:ender_pearl was consumed`

  `n/a: this port's loot tables were already in the 26.3 form`

The in-game leg is `tools/e2e-dialect.sh` (`ports/_shared/evidence/enderstorage/e2e-dialect.log`): `PASS` + `EXIT:0`,
and `SELFTEST=1` inverts the expectation and ends `FAIL` + `EXIT:1` (`e2e-dialect-selftest.log`).
`check-e2e-logs.py enderstorage --strict` exits 0.

**Ladder** (all in `build-slot.sh` leases, `CARGO_INCREMENTAL=0`, `BUILD_LOCAL=1`):

| Step | Command | Result |
|---|---|---|
| tests | `cargo test --offline` | **49 passed**, 0 failed (`dialect-test.log`) |
| package | `rm -f Cargo.lock && cargo horizon-mod build` (twice) | built against **0a875da** (re-pinned from 0b14f8a), second build `cmp`-identical (`dialect-build-1.log`, `dialect-build-2.log`) |
| registry check | `cargo horizon-mod check enderstorage.hmod` | `enderstorage 0.2.3 (native-mod, format native-1/0a875da…/e59da90…, for 26.3)`, **53,203 B**, sha256 `9ad59a8b9d09…` (`dialect-check.log`) |
| fits the pin | `cargo horizon-mod fits enderstorage.hmod --build 0a875da` | `0a875da  made for`, exit 0 (`dialect-fits-0a875da.log`) |
| fits the previous build | `… --build 0b14f8a` | `fits`, exit 0 (`dialect-fits-0b14f8a.log`) |
| the gate | `python3 ports/tools/verify-sdk-package.py enderstorage` | **1/1** (`dialect-gate.log`) |

Package 0.2.3: **53,203 B**, SHA-256 `9ad59a8b9d0981471497eea8e26196b1b1460e0327b7bb298148c3229a8f62b7`.

Skills: superpowers/using-superpowers,systematic-debugging,test-driven-development,verification-before-completion; BeautifulRust/none (no Rust change)

## Reviewer repairs (2026-11-21) — PR #1538: two inventory-loss cases (0.2.4)

The reviewer of PR #1538 (`levzzz5154`) read the bucket and tank glue and reported:

> The repaired recipe exposes two inventory-loss cases: filling from a stack of empty buckets replaces
> the whole stack, and placing a tank on a stocked frequency can erase the shared fluid because the
> code records the snapshot before initializing the tank. Preserve the other empty buckets and
> initialize the tank before taking the snapshot.

Both were reproduced on the **delivered 0.2.3 package** at the campaign pin before any edit, and a
third one surfaced while fixing the first two.

### Red 1 — a stack of empty buckets loses the rest of the stack (`runner`, quoted)

Probe `ports/_shared/evidence/enderstorage/probe-bug1-stack-0.2.3.sh` on a launcher-built 0.2.3 game
(`red-bug1-stack-0.2.3.log`). The tank's compound is stocked with 4000 mB (the pin's bucket *fill*
path keeps no write, so the compound is where the port's tank lives), then one empty-bucket click
with a stack of eight:

```
  before: [{"item":"minecraft:bucket","count":8}, …]
  after:  [{"item":"minecraft:bucket","count":1}, …]
  tanks.dat: # enderstorage-tanks 1 t white,white,white minecraft:water 3000
```

Eight buckets became **one** item and the seven others were destroyed (and no full bucket was in the
hand: `swap_held` had replaced the whole slot). Root cause: `support::swap_held` wrote
`ItemStack::new(key, 1)` over the held slot, whatever its count.

### Red 2 — a tank placed on a stocked frequency erases the shared fluid (`runner`, quoted)

Probe `…/probe-bug2-tank-0.2.3.sh` (`red-bug2-tank-0.2.3.log`): stock `red,white,white` with 4000 mB,
then place a second tank at `4,64,0` from a tank item carrying `red,white,white` (so the placement's
pending frequency differs from the fresh entity's white default and the tick's switch runs):

```
  info A: Ender Tank: red / white / white (minecraft:water 4000 mB, 4000 mB in the store)
  data B right after: … id: "enderstorage:ender_tank", frequency: "red,white,white"      <- no fluids
  t+~8  info A: Ender Tank: red / white / white (empty 0 mB, 0 mB in the store)
  data A: … fluids: [{Amount: 0, Fluid: "minecraft:empty"}], frequency: "red,white,white"
  tanks.dat: # enderstorage-tanks 1 … t red,white,white - 0
```

Root cause: `support::tick_tank`'s pending-frequency branch set `current` to the switched (store)
tank and recorded it as the entity's snapshot, but the write-back was gated on
`synced.tank != current` — with `synced.tank == current` the entity's compound was never written. On
the next tick the entity read its old empty tank, the merge treated it as a local change from the
recorded 4000 mB snapshot, and subtracted the whole 4000 from the store.

### Red 3 — one click exchanged two buckets (found by the fix, quoted)

With the hand swap written in `right_click_block`, the item's own use ran a second exchange on the
already-swapped hand: eight empty buckets ended `6 empty + 2 full` for one 1000 mB draw
(`probe-double-exchange-0.2.4.log`) — the stack survived, but a bucket of fluid was duplicated.
Claiming the click with `Success`/`Fail`/`Consume`, and writing the produced bucket through
`give_stack` or a free slot, all reproduced it; the one shape that does not is doing the swap in
`fill_bucket` (the bucket's own use), which is where it now lives
(`probe-bug1-after-0.2.4.log`: `7 empty + 1 full`, `info A … 4000 → 3000`).

### Fixes

- **`runtime::tank_write(entity, merged) -> TankWrite`** (new, with `a_tank_the_entity_does_not_hold_is_written_with_its_snapshot`):
  the tank one tick keeps is `merged`, and it **must be written** when the entity does not already
  hold it; the snapshot is that same value. `tick_tank` reads the entity's tank once (`held`) and uses
  `tank_write(&held, synced.tank)` for both the write and the snapshot, and passes the switched tank
  to `sync_tank` as the snapshot for the new frequency instead of inserting it early.
- **`runtime::bucket_swap(held_count) -> Option<BucketSwap>`** (new, with
  `a_bucket_swap_keeps_the_rest_of_the_stack`): one item of a stack becomes a bucket, the rest stay in
  the hand, and only the last item lets the produced bucket take the hand's slot (vanilla's
  `ItemUtils.createFilledResult`). `support::swap_held` shrinks the stack with `set_count` and
  gives the produced bucket through `PlayerRef::give_stack` (vanilla's `Inventory.add`: matching
  stacks, then an empty slot, then dropped at the player's feet), and restores the hand if that
  fails so the caller's transaction rolls the fluid back.
- **`support::right_click` claims the tank click for a bucket but no longer swaps the hand**; the swap
  runs in `support::fill_bucket` (the bucket's own use), so the engine's one item use cannot repeat it.
  The `fill_bucket` drain/fill stays in one transaction with the swap.
- `src/runtime.rs` therefore differs from the retired file by the two rules and their tests (the
  earlier "byte-identical" line above is history; the gate now checks the packed file equals this one).

### In-game assertions (quoted from `e2e-0.2.4.log`, `PASS` + `EXIT:0`)

```
== bug 2: a tank placed from an item on a stocked frequency keeps the shared fluid ==
  info(18,64,0): Ender Tank: red / white / white (minecraft:water 4000 mB, 4000 mB in the store)
  info(14,64,0): Ender Tank: red / white / white (minecraft:water 4000 mB, 4000 mB in the store)
  tanks.dat after the second tank: t red,white,white minecraft:water 4000
== bug 1: an empty bucket drains one bucket and the rest of the stack survives ==
  inventory after the drain: 7 empty + 1 full
  info(20,64,0): Ender Tank: white / white / white (minecraft:water 3000 mB, 3000 mB in the store)
  tanks.dat after the drain: t white,white,white minecraft:water 3000
```

The legs are fail-capable: `SELFTEST=1` inverts the sharing assertion and the run ends
`FAIL` + `EXIT:1` (`e2e-0.2.4-selftest.log`); the water-bucket-at-a-tank phase (the recorded pin
gap) and the vanilla cauldron control beside it are unchanged.

### Coverage of the families this slice touches

| family | what the leg does | result |
|---|---|---|
| tank drain by an empty-bucket stack | 8 empty buckets, one click: 7 empty + 1 full, store −1000 mB | tested (`bug 1`, `7 empty + 1 full`) |
| tank item placement on a stocked frequency | second tank placed from a red item, both read 4000 mB, `tanks.dat` keeps it | tested (`bug 2`) |
| tank fill by a full bucket | water bucket at a tank | `not ported` — recorded pin gap (`FluidTanks::insert` keeps no write); the leg asserts the bucket never pours and the tank stays empty |
| vanilla bucket control | water bucket into a cauldron | tested (`water_cauldron`) |

**Ladder** (every step in a `build-slot.sh` lease, `CARGO_INCREMENTAL=0`, `BUILD_LOCAL=1`):

| Step | Command | Result |
|---|---|---|
| unit red | `cargo test --offline` with the two new tests, before implementing them | `error[E0425]: cannot find function \`tank_write\`` / `\`bucket_swap\`` (`red-0.2.4-unit.log`) |
| tests | `cargo test --offline` | **50 passed**, 0 failed (`sdk-test.log`) |
| lints | `cargo clippy --all-targets -- -D warnings` | exit 0, 0 warnings (`sdk-clippy.log`) |
| format | `cargo fmt --check` | exit 0 (`sdk-fmt.log`) |
| package | `rm -f Cargo.lock && cargo horizon-mod build` twice | `cmp`-identical, `6509360…`, built against **0a875da** (`build-1.log`, `build-2.log`) |
| registry check | `cargo horizon-mod check enderstorage.hmod` | `enderstorage 0.2.4 (… 0a875da…/e59da90…, for 26.3)`, **54,512 B**, sha256 `6509360807e1711a0bd2c2b77859f348926060895764f16d023b485721d9b5a7`, exit 0 (`check.log`) |
| fits the pin | `cargo horizon-mod fits enderstorage.hmod --build 0a875da` | `0a875da  made for`, exit 0 (`fits-0a875da.log`) |
| fits the previous build | `… --build 0b14f8a` | `0b14f8a  fits`, exit 0 (`fits-0b14f8a.log`) |
| the gate | `python3 ports/tools/verify-sdk-package.py enderstorage` | **1/1** (50 tests, 0 warnings) |
| in game | `build-slot.sh run … bash tools/e2e.sh` with the launcher-built 0.2.4 game | `PASS` + `EXIT:0` (`e2e-0.2.4.log`) |
| fail-capable | `SELFTEST=1` | `FAIL` + `EXIT:1` (`e2e-0.2.4-selftest.log`) |
| recipe/loot dialect, unchanged | `tools/e2e-dialect.sh` with the 0.2.4 artifact | `PASS` + `EXIT:0` (`e2e-dialect.log`); `SELFTEST=1` `FAIL` + `EXIT:1` (`e2e-dialect-selftest.log`) |
| log gate | `python3 ports/tools/check-e2e-logs.py enderstorage --strict` | exit 0 (7 logs, 7 ok) |

Package 0.2.4: **54,512 B**, SHA-256 `6509360807e1711a0bd2c2b77859f348926060895764f16d023b485721d9b5a7`.

`tools/e2e.sh` now takes its scratch directory from `E2E_TMP` (the launcher exports one on `/home`;
`/tmp` is a 7.5 GB tmpfs), and its bug-1 leg stocks a fresh tank through `data merge block` because
the pin's bucket *fill* path keeps no write.

Skills: superpowers/using-superpowers,systematic-debugging,test-driven-development,verification-before-completion; BeautifulRust/rust-best-practices

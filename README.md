# Ender Storage 1.8.+ — a Horizon port

**Ender Storage 1.8.+** is a Minecraft mod. This repository is a **port of it to [Horizon](https://github.com/horizon-rust/horizon)**, the Rust engine for Minecraft Java 26.3. It carries the complete source of the port plus the content files it ships, and installs as a single `.hmod` package.

Upstream: [Chicken-Bones/EnderStorage](https://github.com/Chicken-Bones/EnderStorage) — licence **MIT**.

| | |
|---|---|
| Upstream mod | Ender Storage 1.8.+ ([source](https://github.com/Chicken-Bones/EnderStorage)) |
| Ported to | Horizon 0b14f8a, Minecraft Java 26.3, mod API `0.2` |
| Package | `enderstorage.hmod` (install it with the Horizon launcher) |
| Licence | **MIT** — the upstream mod's licence; see [Licence and credit](#licence-and-credit) |

## What this port implements

- **Store** (`runtime.rs`): `Store<Slot>` keyed by `Frequency`; 27 slots
- **Merge** (`sync`): on a chest's first tick after it loads, the store wins
- **Snapshots are keyed by a per-block-entity uid** kept in its NBT, so a
- **Re-colour** (`switch`): the chest's unsynced changes are committed to the
- **Dye click**: the pin has no sub-hit, so one right click with a dye sets the
- **Item frequency**: a custom item component `enderstorage:frequency` (a
- **Breaking/explosions**: the chest's last changes are committed, its slots
- **Container**: 27 slots reachable from every side, `minecraft:generic_9x3`

## Not yet ported

- **Tank deferred.** `native_block_entity_fluids` (API 54) and
- **Pouch deferred.** A pouch opens the same inventory from an item; a mod
- **Personal (owned) frequencies deferred.** The `owner`/`ownerName` part of
- **No sub-hit.** One dye click sets the next band (left → middle → right →
- **No custom renderer/lid.** The block uses the vanilla ender chest model and
- **Crafting does not carry the colours.** Upstream's `create_recipe`/
- **Menu title** is the block's name (`block.enderstorage.ender_chest`); the
- **A crash can lose up to one second of store changes** (the debounced

## Installing

1. Install the [Horizon launcher](https://github.com/horizon-rust/horizon/releases) (0.2.70 or newer).
2. Add `enderstorage.hmod` to the instance's mods.
3. The package is pinned to a specific Horizon build; the launcher checks that pin.

## Licence and credit

Ender Storage 1.8.+ was created by its upstream authors; the code, models and assets in the original mod
remain theirs and are used here under **MIT**. The Rust in this repository is a clean-room
rewrite for Horizon, not a translation of the original source.

- `LICENSE` — the upstream licence text.
- `LICENSE-NOTICE.md` — what this port carries and where it came from.
- The upstream project: https://github.com/Chicken-Bones/EnderStorage

## Technical details

```
horizon-mod.toml   package manifest (id, version, api, [native] pin)
Cargo.toml         the crate; depends on `horizon-mod-api` from the fetched SDK
src/               lib.rs (the one `impl Hooks`), runtime.rs (pure logic + tests), glue
assets/ data/      the content the package ships
tools/             content generators and the e2e scenario
```

- Build: `cargo horizon-mod build` (packages; it does not compile against the game).
- Test: `cargo test`, `cargo clippy --all-targets -- -D warnings`.
- In-game: `ports/tools/launcher-game.sh run enderstorage` in the campaign tree, or install the
  `.hmod` and play; the harness scenario is `tools/e2e.sh`.

This repository is generated from the campaign's port at `ports/enderstorage/`; the port's own
`notes.md` is the full record (decisions, traps, evidence).

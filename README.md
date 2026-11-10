# enderstorage (Horizon port)

A Horizon port of [enderstorage](chicken-bones/EnderStorage) — MIT.

**Status: first slices.** What is below is what the port actually does today.

## Implemented so far

- Upstream keeps **one inventory object per `Frequency`** (three wool colours,
- combinations) in a world-global manager, persisted in
- `<save>/EnderStorage/data*.dat`, and every chest with that frequency opens the
- same `Container`. Horizon gives a mod block entity its own item slots, so the
- port keeps the store as the truth and **mirrors** it into each chest once per
- block-entity tick:
- Store** (`runtime.rs`): `Store<Slot>` keyed by `Frequency`; 27 slots
- (`item_storage_size = 1`, upstream's default 3x9). Persisted as

## Not yet ported

- Tank deferred.** `native_block_entity_fluids` (API 54) and
- `native_fluid_insert/extract/move` exist at the pin, so a tank block can
- declare 16-bucket tanks in its block entity's NBT; but the pin has **no
- bucket path into a mod block's tanks** (only mods call the fluid helpers),
- so the tank's player interaction would need a `use_item_on` bucket handler
- with a held-stack swap. That is the next slice; the fluid store would reuse
- the same per-frequency merge (one tank instead of 27 slots).
- Pouch deferred.** A pouch opens the same inventory from an item; a mod

## Install

Build the bundle with a native-mod capable launcher, or add the file from the
registry once it is listed. Pinned to Horizon `1d63e0a` and horizon-mixins
`5a64af93` for game 26.3.

## Build from source

```sh
cargo test
python3 tools/horizon.py check --horizon <horizon checkout>
python3 tools/horizon.py build --horizon <horizon checkout>
python3 tools/package.py
```

## Credits and licence

Original mod: chicken-bones/EnderStorage. Port: AdLf0208. Licence: MIT — see [LICENSE](LICENSE);
the original notices are preserved.

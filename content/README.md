# EnderStorage — port-owned content mod

The `.hmod` for EnderStorage slice 1 (the frequency-linked ender chest). This
folder **is** the mod root (`horizon-mod.toml` + `assets/` + `data/`);
`enderstorage.hmod` is the built artifact.

| Path | What |
|---|---|
| `data/enderstorage/horizon/block/ender_chest.json` | The block: `minecraft:barrel` template (27-slot container + `facing`/`open`), hardness 20 / resistance 100 (upstream `strength(20, 100)`), stone sounds and map colour, no tool requirement. |
| `data/enderstorage/horizon/creative_tab/enderstorage.json` | One EnderStorage tab. |
| `data/enderstorage/recipe/ender_chest.json` | Upstream's shape (`BWB/OCO/BPB`); the three wool colours are *not* carried (no crafting hook — see notes). |
| `data/enderstorage/loot_table/blocks/ender_chest.json` | Drops the chest item (the native half adds the frequency component). |
| `data/minecraft/tags/block/mineable/pickaxe.json` | Upstream's tag. |
| `assets/enderstorage/blockstates/ender_chest.json` | All 12 `facing` × `open` states bind the vanilla ender chest model. |
| `assets/enderstorage/items/ender_chest.json` | 26.3 special chest item model (ender texture). |
| `assets/enderstorage/lang/en_us.json` | `block.`/`item.enderstorage.ender_chest`. |

The block gets its block entity, container and menu from the native half
(`ports/enderstorage/mixins/`), not from the content format (which has no
block-entity declaration).

Built with:

```sh
cd ports/enderstorage/content
CARGO_TARGET_DIR=/tmp/opencode/enderstorage-mod-target cargo run --locked --offline --quiet \
  --manifest-path ../../../horizon/crates/cargo-horizon-mod/Cargo.toml -- horizon-mod build
cp target/horizon-mod/enderstorage.hmod .
```

`horizon-mod check enderstorage.hmod` reports 2740 bytes, SHA-256
`3d55556f55d59401113393c563b66405d91d94493242316d893ae59b6b6121f8`, content
only.

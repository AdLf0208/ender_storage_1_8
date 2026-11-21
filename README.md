# EnderStorage for Horizon

A native Horizon port of **EnderStorage 2.13.0** (`TheCBProject/EnderStorage`,
MIT) for Minecraft 26.3: ender chests and ender tanks linked by a three-colour
frequency.

Every chest with the same frequency (`left / middle / right` wool colours, 4096
combinations) shows the same 27 slots in any dimension, and every tank with the
same frequency shares one 16-bucket tank. Dye sets the colours, a bucket fills or
drains the tank, the item carries the frequency when broken and placed, and the
stores persist in `<world>/enderstorage/frequencies.dat` and `tanks.dat`.

**Format: the SDK package (Horizon mod API 0.2).** One flat `enderstorage.hmod`
holds `horizon-mod.toml`, `Cargo.toml`, `src/lib.rs` (the one
`impl mod_api::Hooks`), `src/support.rs` (game glue), `src/runtime.rs` (the pure
store and merge, 32 unit tests, byte-identical to the retired `.hnmod` port), and
the content in `assets/` + `data/`. See
[notes.md § "Migration to the SDK format"](notes.md#migration-to-the-sdk-format-020-2026-11-18)
for the file map, the helper→hook table and the traps.

The pouch, personal (owned) frequencies, the three dye pads, grid crafting that
carries colours, the lid animation and the custom renderers are deferred; see
[notes.md](notes.md) for the upstream analysis, the hook table, the deviations
and the in-game evidence.

## Build and test

No staging step and no mixin plan: the crate is compiled in place against the
SDK the build fetches into `.horizon/sdk/` (gitignored).

```sh
CARGO_INCREMENTAL=0 cargo test                        # runtime + glue tests
CARGO_INCREMENTAL=0 cargo clippy --all-targets -- -D warnings
cargo fmt --check                                     # not --all: that checks the SDK
rm -f Cargo.lock                                      # a packed lock changes the artifact
CARGO_INCREMENTAL=0 cargo horizon-mod build           # -> target/horizon-mod/enderstorage.hmod
cp target/horizon-mod/enderstorage.hmod ./enderstorage.hmod
cargo horizon-mod check enderstorage.hmod             # the registry's own check
cargo horizon-mod fits enderstorage.hmod --build nightly
sha256sum enderstorage.hmod
```

In the campaign tree every step runs through a build slot, e.g.

```sh
BUILD_LOCAL=1 ports/tools/build-slot.sh run --job sdk-es --port ports/enderstorage \
    --step check -- bash -c 'cd ports/enderstorage && cargo horizon-mod check enderstorage.hmod'
```

`tools/e2e.sh` is the in-game scenario. It needs a game the launcher built with
this package (`horizon-launcher native-build <port> --instance <i>
--trust-native-code`) and `horizon-ctl` at the same pin:

```sh
GAME=<launcher-built horizon-client> CTL=<horizon-ctl> tools/e2e.sh
```

The pinned revisions are `[native]` in `horizon-mod.toml` (SDK `0b14f8a`,
mixins `e59da90`, game 26.3). Minecraft 26.3 assets come only from the campaign's
`26.3.zip` prepared root (`/home/adlf/Desktop/horizon/.local/horizon-client/26.3`).

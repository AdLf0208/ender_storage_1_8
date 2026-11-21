# Validation

Pinned to Horizon `41f822b659789780f58d32c63ddfb6887ea1b8bf` and
horizon-mixins `5a64af938ec86b00d53d16a408aabc0038fb3e57` (schema 6); see
`compatibility.json` and `notes.md` §"Verification results (final, 2026-11-07)".

| Check | Command (through `ports/tools/build-slot.sh`) | Result |
|---|---|---|
| Embedded tests | `python3 tools/horizon.py test --horizon …/pins/horizon-41f822b` | **27 passed**, 0 failed (`game::enderstorage::`) |
| Staged check | `python3 tools/horizon.py check …` | `cargo_exit: 0`, **0 warnings** |
| Fast build | `python3 tools/horizon.py build …` | `cargo_exit: 0` (fast profile) |
| Content package | `horizon-mod build` + `horizon-mod check` | 4,258 B, SHA-256 `6cccd1cb…9c3e`, 14 entries, content only |
| Package load | `tools/hmod-load.sh` (shared unmodded `41f822b6` game) | tank block + chest item load; unknown without `--dev-mod` |
| In-game | `build-slot.sh e2e … -- tools/e2e.sh` | exit 0, `PASS`, `EXIT:0` |

The e2e is fail-capable: the blue tank stays empty while the two white tanks
share one tank, and a vanilla cauldron takes the bucket while the ender tank
stays empty. `SELFTEST=1` flips the chest-sharing assertion and exits 1.
Evidence (commands, counts, PASS lines, ctl replies) is in
`notes.md` and `ports/_shared/evidence/enderstorage/`.

## Embedded content + re-verification (2026-11-10)

Horizon `1d63e0a5a37a7a708eb0b32b49bf0dfeb28ed8a9` (game.rs `64813cf4…`),
horizon-mixins `5a64af938ec86b00d53d16a408aabc0038fb3e57` (schema 6), every step
through `ports/tools/build-slot.sh` with `CARGO_INCREMENTAL=0`.

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --all-targets --locked --offline -- -D warnings` | exit 0, 0 warnings |
| `cargo test --locked --offline` | green |
| `horizon.py stage` / `check` / `test` | exit 0 / `cargo_exit: 0`, 0 warnings / embedded tests green |
| `tools/package.sh` | `content/enderstorage.hmod` 4,258 B `6cccd1cb633b42bf…`, `enderstorage.hnmod` 29,489 B `9342ca9b43a63033…` (23 entries, embeds the content) |
| e2e (no `--dev-mod`) | default `PASS` + `EXIT:0`; `SELFTEST=1` `FAIL` + `EXIT:1` |
| `upstream_ready.py enderstorage` | **READY (exit 0)** |

## 0.2.0 — the SDK package (2026-11-18)

The sections above describe the retired `.hnmod` format (`.hnmod` + `content/` + the mixin plan);
their evidence still stands for the behaviour, which `src/runtime.rs` carries byte-identically.
This port is now one flat `enderstorage.hmod` at the SDK pin **`0b14f8a`** (mixins `e59da90`), and
there is no staging step: the crate compiles in place against `.horizon/sdk/`.

| Check | Command (inside one build slot, `CARGO_INCREMENTAL=0`) | Result |
|---|---|---|
| Runtime + glue tests | `cargo test` | **47 passed**, 0 failed (32 `runtime::`, 15 `support::`) — `../_shared/evidence/enderstorage/sdk-test.log` |
| Lints | `cargo clippy --all-targets -- -D warnings` | exit 0, 0 warnings — `sdk-clippy.log` |
| Format | `cargo fmt --check` | exit 0, empty diff — `sdk-fmt.log` |
| Package | `rm -f Cargo.lock && cargo horizon-mod build` | `Built ./target/horizon-mod/enderstorage.hmod` — `sdk-build.log` |
| Registry check | `cargo horizon-mod check enderstorage.hmod` | `enderstorage 0.2.0 (native-mod, format native-1/0b14f8a…/e59da90…, for 26.3)`, 49,006 B, sha256 `b4d9d58b…e586`, **EXIT 0** — `sdk-check.log`, `sdk-check-json.log` |
| Fits the pinned build | `cargo horizon-mod fits enderstorage.hmod --build nightly` | `0b14f8a  made for`, **EXIT 0** — `sdk-fits.log` |
| `runtime.rs` untouched | `unzip -p enderstorage.hmod src/runtime.rs \| sha256sum` | `88c49c34…9d32` = `src/runtime.rs` = the retired file |
| What it uses | `unzip -p enderstorage.hmod uses.json` | 55 hook entries, 54 methods, 6 Parts, `horizon:fluids` + `registries`, 240 types — `sdk-uses-from-package.json` |
| Skills line | `python3 ports/tools/check-skills.py --notes enderstorage --strict` | `Skills line present`, **SKILLS GATE: PASS** |

**Not re-run here:** the in-game leg. It needs a game the launcher built with this package
(`horizon-launcher native-build`) and `horizon-launcher` is not installed in this environment; the
0.1.0/2026-11-11 `PASS`/`EXIT:0` runs remain the behaviour's evidence. See
`notes.md § "Gaps after the migration"`.

## 26.3 dialect sweep (2026-11-21) — 0.2.3

| Check | Command | Result |
|---|---|---|
| static dialect | `python3 ports/tools/check_261_dialect.py enderstorage.hmod` | exit 0; pre-fix package exit 1 (`../_shared/evidence/enderstorage/dialect-check-red.log`) |
| tests | `cargo test --offline` | exit 0 (`dialect-test.log`) |
| package | `cargo horizon-mod build` twice after `rm -f Cargo.lock` | `cmp`-identical, built against 0a875da |
| registry check | `cargo horizon-mod check enderstorage.hmod` | **53,203 B**, SHA-256 `9ad59a8b9d0981471497eea8e26196b1b1460e0327b7bb298148c3229a8f62b7`, exit 0 |
| fits the pin | `cargo horizon-mod fits enderstorage.hmod --build 0a875da` | `0a875da  made for`, exit 0 |
| the gate | `python3 ports/tools/verify-sdk-package.py enderstorage` | 1/1 |
| in game | `tools/e2e-dialect.sh` | `PASS` + `EXIT:0`; `SELFTEST=1` `FAIL` + `EXIT:1`; `check-e2e-logs.py enderstorage --strict` exit 0 |

Changed: 2 recipes. Proof: ender_chest crafts 1; loot already correct (not touched); pre-fix red. Full record in [`notes.md`](notes.md) §"26.3 dialect sweep".

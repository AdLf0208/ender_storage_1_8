# EnderStorage — re-pin audit (read-only, no code changed, no builds)

Target pin: **Horizon `83ad384f`** (427 hooks / 159 helpers) at
`ports/_shared/pins/horizon-83ad384f`, mixins **`e8cd0aa`** at
`ports/_shared/pins/horizon-mixins-e8cd0aa`. Port is currently pinned at
`c9d474d` / `d9c83d7` (`compatibility.json`).

Method: read `notes.md` §Gaps/§"Upstream source analysis" + ARCHITECTURE
item 41, then searched `native-hooks.json` and
`crates/horizon-client/src` at the new pin. No port code, bundle, content or
compatibility file was modified; nothing was built or run in-game.

**Evidence baseline.** `fbb17c980` (the campaign's read-only `horizon/`
provider) has 186 hooks and is newer than `c9d474d` (108). The new pin contains
every hook `fbb17c980` has (no removals in the diff), so a hook **absent from
`fbb17c980` did not exist at `c9d474d`**; a hook present there may predate
`c9d474d`. Verdicts below use that conservative reading.

Class: **usable** = writable/cancel/replace/collect or a helper the bundle can
call; **hook-only** = listen-only; **absent** = none.

## Deferred features (9)

| # | Deferred (notes) | Evidence at `83ad384f` | Class | Verdict | Next step |
|---|---|---|---|---|---|
| S1 | **Tank** (16 buckets; notes gap 1: "no bucket path into a mod block's tanks") | `native_block_entity_fluids` (`native_hooks.rs:3050`, **API 54**: up to 16 tanks, capacities, per-face reach, contents saved under `fluids` in the BE compound); fluid helpers `native_fluid_insert`/`extract`/`move_fluids` (`machines/native_fluids.rs:221,378,474`); a player's bucket click: `native_bucket_empty` (`native_hooks.rs:4399`, **API 179**, cancel before the fluid is placed), `native_fill_bucket` (`:6840`), `native_use_item_on` (`:6482`, cancel) + `native_right_click_block` (`:6790`); held-stack swap via `native_held`/`give_stack`/`take_items`; **item** tanks for the port's own bucket/pouch item: `native_item_fluid_tank` (`:3081`, API 147) + `native_item_fluid`/`_fill`/`_drain` (`native_item_fluids.rs:199,209,228`); `native_fluid_level_change` (`:1214`, listen) | usable | **Realizable now — the recorded blocker is closed.** The tank block declares its tanks through API 54 (already planned in the notes) and the "no bucket path" gap is filled by `bucket_empty`/`use_item_on` + the fluid helpers. `bucket_empty` is **new since `fbb17c980`**. | Add the tank slice: one `block_entity_fluids` tank per frequency, `use_item_on` with a bucket → `native_fluid_insert`/`extract` + held-stack swap, and reuse the existing per-frequency store/merge with a fluid payload. Probe fill, drain, and a bucket from a second frequency. |
| S2 | **Pouch** (an item opens the same 27 slots; notes gap 2: "a server-built menu") | `native_open_menu` with an **arbitrary title** (`native_menus.rs:124`), `native_set_menu_items` (`:143`), `menu_clicked`/`menu_closed` (API 10); `native_item_container` (`native_hooks.rs:3109`, **API 370**, 1–256 slots kept in the stack's `minecraft:container`) + `native_item_container_slots`/`_insert`/`_extract` (`native_item_container.rs:166,181,194`); `native_menu_extra_data` (`:4860`, **API 401**, up to 32 KiB to the client screen) and `native_screen_extra_data`/`native_screen_pos`; `native_player_container_open/close` (`:7066`, **API 152**) | usable | **Realizable now.** `item_container` (API 370) is **new since `fbb17c980`**, so the pouch's carried slots are a first-class capability; the menu/title path already existed (akashictome precedent). | Implement the pouch as an item-container item (or as a per-frequency store view) + `open_menu` titled with the frequency; mirror the store in `menu_clicked` and send the frequency via `menu_extra_data`. Probe the pouch→chest round trip and a second pouch on the same frequency. |
| S3 | **Personal/owned frequencies** (`owner`/`ownerName`, diamond "personal item", sneak to detach) | No API gap: item components (`component_types`, API 65), `native_use_item`/`use_item_on`/`interact_entity`, `native_acting_player` (uuid). Save/merge is the port's own runtime. | usable | **Realizable now** (scope, not API). | Add `owner`/`owner_name` to the store key and the item component; gate `open`/`switch` on `native_acting_player().uuid`; sneak-click detaches. |
| S4 | **Sub-hit** for the three dye pads (notes gap 4: "no hit position") | Still absent: `native_use_item_on(stack, pos, face, claimed)` (`native_hooks.rs:6482-6489`) and `native_right_click_block(pos, face)` (`:6783-6790`) carry only the face, **no hit vector**; `native_view_hit`/`native_raycast` are the **client's local** player (`native_claimed_items.rs:137-142`) and cannot answer a server hook | absent | **Blocked.** One click per block cannot address one of three pads. | Keep the cycle gesture (left→middle→right). Issue-shaped request: "the exact hit point in `native_use_item_on`/`native_right_click_block`". Alternative: three separate pad blocks (a content change) if the cycle is unacceptable. |
| S5 | **Custom renderer / lid** (notes gap 5) | `native_block_entity_submit` (`native_hooks.rs:3190`, API 71) with `native_draw_quad`/`draw_cube`/`draw_block`/`draw_item` (`native_block_entity_draw.rs:548,620,653,719`), `native_block_entity_update_tag` (`:3141`, sync the frequency to the client), `native_block_entity_render_bounding_box` (`:3215`), `native_mod_container_viewers` (`:7025`, **API 124**) to drive an `open`/lid state | usable | **Realizable now.** The lid `open` state can follow the viewer count (API 124) on a chest template; the three frequency bands can be drawn by `block_entity_submit`. | Add the renderer/lid after S1/S2; start with the viewer-count `open` state (cheap, no geometry), then the colour bands. |
| S6 | **Crafting carries the frequency** (notes gap 6: custom `create`/`recolour` recipes; "no crafting hook") | `native_prepare_item_craft(player, name, pos, result)` (`native_hooks.rs:5659`, **API 290**) can change the **result** but is not given the grid; `native_item_crafted(player, recipe, stack)` (`:7525`, **API 161**) hears the matched recipe id and the taken stack **after** the take; no hook exposes the crafting grid; `native_ingredient` (API 377) matches a custom ingredient but does not read it | usable (result) / absent (grid) | **Partially realizable.** Faithful "dye in the grid chooses the frequency" is blocked. A workaround: 16 distinct shaped recipes whose **output item** differs per band (or a per-dye intermediate), mapped to the frequency in `prepare_item_craft`. | Either ship the 16-output workaround (documented deviation) or keep the dye-click/command path. Issue-shaped request: "the crafting grid in `native_prepare_item_craft`, or custom recipe-type execution". |
| S7 | **Menu title shows the frequency** (notes gap 7) | `native_open_menu(rows, title: Text, …)` takes any title (`native_menus.rs:117-136`); `native_menu_extra_data` (API 401) + `native_screen_extra_data`/`native_screen_pos` send/show per-open data; content `menu` types (`content.rs:402-420`) | usable | **Realizable now** through a **mod menu type** opened by the block (or the pouch). The *template-opened* barrel menu keeps the block name — that part is unchanged. | Add a content `menu` type + `native_open_menu` titled `EnderStorage: <l>/<m>/<r>`; keep the vanilla barrel menu as the fallback. |
| S8 | **Store size 9/54 + config** (notes gap 9) | `native_block_entity_container` allows up to 256 slots (`native_hooks.rs:3035`); content menu types 1–256 slots (`content.rs:402-420`); `native_register_configs` (`native_hooks.rs:2424`, **API 397**) for `storageSize` | usable (new) | **Realizable now.** API 397 is **new since `fbb17c980`**. | Register a `Server` config for `storageSize` (9/27/54) and size the store/menu from it; probe a 9-slot and a 54-slot chest. |
| S9 | **Up to 1 s of store changes lost on a crash** (notes gap 8) | `native_world_saved(wait)` (`native_hooks.rs:5915`, **API 306**), `native_player_container_open/close` (`:7066`, API 152), `server_stopping` (existing) | usable (new) | **Realizable now (mitigation).** Flush on world save and on every container close; the debounce can shrink to "dirty, flushed at the next save/close". API 306 is **new since `fbb17c980`**. | Flush the store in `world_saved` and on `menu_closed`/`player_container_close`; keep the 1 s write as a backstop. |

## Summary

- **Deferred: 9** (S1–S9).
- **Realizable now: 7** — S1 (tank), S2 (pouch), S3 (owned frequencies), S5
  (renderer/lid), S7 (title via a mod menu), S8 (size/config), S9 (flush).
  Of these, **S2 (`item_container`, API 370) and S8/S9 (`register_configs`,
  `world_saved`, API 397/306) are genuinely new since `fbb17c980`**, i.e. new
  versus the old pin; S1's missing piece (`bucket_empty`, API 179) is new too.
- **Still blocked: 2** — S4 (sub-hit: no hit vector) and S6 as *faithful*
  crafting (no grid in the prepare hook; a 16-output workaround exists).
- **Bonus, no longer deferred:** the comparator path can be explicit now —
  `has_analog_output` / `analog_output` (`native_hooks.rs:1539,1555`, both new
  since `fbb17c980`) let the chest answer for its own block instead of relying
  on the mirrored container.
- **Highest-value next step:** **S1, the tank.** It doubles the mod's content,
  is fully specified by upstream (`EnderLiquidStorage`), and every previously
  missing piece now exists: block fluid tanks (API 54), the bucket click
  (`bucket_empty`/`fill_bucket`, API 179), and the fluid helpers. It reuses the
  per-frequency store/merge the port already proved, with one tank instead of
  27 slots. The pouch (S2) is the natural follow-up.

No build, test or e2e was run for this audit; all claims above are source
reads at `83ad384f`.

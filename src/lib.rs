//! EnderStorage for Horizon — frequency-linked ender chests and ender tanks.
//!
//! Ported from EnderStorage 2.13.0 (`TheCBProject/EnderStorage`, MIT). [`runtime`] is the pure
//! truth — the frequency, the per-frequency store and tank store, and the three-way merge with its
//! tests — and is byte-identical to the retired `.hnmod` port's file. [`support`] is the glue: block
//! entities, store files, clicks, buckets and commands. This file is the one `impl Hooks`.
//!
//! # Where the retired container's hooks went
//!
//! The 2026-11-17 mod-API merge (#16941, launcher ≥ 0.2.70) retired `.hnmod`: the plan, the injection
//! points and the per-hook helper files are gone. A hook is a typed method here, and a moment that
//! belongs to one block or one item is a **Part** attached to it.
//!
//! | `.hnmod` hook (API) | here |
//! |---|---|
//! | the `content_root` injection (AE2 pattern) | gone: one flat `.hmod` carries `assets/` and `data/` itself |
//! | `component_types` (65) | [`Mod::component_types`] → `support::declare_component` |
//! | `block_entity` (15) | the block entity **types** declared in [`Mod::register`] |
//! | `block_entity_ticks` (15) | `support::Chest`/`support::TankPart` (`BlockEntityPart::block_entity_ticks`) |
//! | `block_entity_container` (16/17) | `Registrar::block_container` in [`Mod::register`] |
//! | `block_entity_fluids` (54) | `Registrar::block_fluids` in [`Mod::register`] |
//! | `block_entity_tick` (17) | `BlockEntityPart::block_entity_tick` on both Parts |
//! | `right_click_block` (21) | [`Mod::right_click_block`] |
//! | `bucket_empty` (179) | [`Mod::fill_bucket`] |
//! | `place_block` (14) | `ItemPart::on_block_placed_by` on `support::Placed` |
//! | `destroy_block` (14) | [`Mod::break_block`] |
//! | `loot_drops` (75) | [`Mod::loot_drops`] |
//! | `explosion_detonate` (22) | [`Mod::explosion_detonate`] |
//! | `world_lifecycle` (v1) | [`Mod::world_lifecycle`] |
//! | `server_tick` (4) | [`Mod::server_tick`] |
//! | `server_stopping` (v1) | [`Mod::server_stopping`] |
//! | `register_commands` (8) | [`Mod::register_commands`] |
//! | `perform_command` (8) | [`Mod::run_mod_command`] |
pub mod runtime;
pub mod support;

use mod_api::{
    self, ComponentTypesEvent, Flow, Hooks, RegisterCommandsEvent, Registrar, RegistrationContext,
    RunModCommandEvent, ServerHost, ServerStoppingEvent, ServerTickEvent, WorldLifecycleEvent,
};

/// The mod keeps its state in the shared store, not in this struct: one store serves every chest in
/// every dimension (see `support`).
#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    /// Declare what the game answers for the two blocks (`EntityBlock` plus NeoForge's
    /// `RegisterCapabilitiesEvent`): a block entity type each, the chest's 27-slot container and
    /// menu, the tank's 16-bucket tank, and the item Parts that carry a frequency to a placed block.
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        support::declare(r);
    }

    /// The item component that carries a frequency, registered before content (and so the item's
    /// stack codec) loads.
    fn component_types(&self, _context: &RegistrationContext, event: &mut ComponentTypesEvent<'_>) {
        support::declare_component(event.types);
    }

    /// `/enderstorage freq|info` (upstream's own command file is commented out in 1.21.1).
    fn register_commands(&self, _host: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        support::declare_commands(event.commands);
    }

    /// Run one of the commands above and take it.
    fn run_mod_command(
        &self,
        host: &mut dyn ServerHost,
        event: &mut RunModCommandEvent<'_>,
    ) -> Flow {
        support::run_command(host, event)
    }

    /// A right click: a dye sets the chest's next colour band, a bucket fills or drains the tank.
    fn right_click_block(
        &self,
        host: mod_api::HostSide<'_>,
        event: &mut mod_api::RightClickBlockEvent<'_>,
    ) -> Flow {
        support::right_click(host, event)
    }

    /// A bucket aimed at the tank: one bucket in, and nothing poured into the world.
    fn fill_bucket(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::FillBucketEvent<'_>,
    ) -> Flow {
        support::fill_bucket(host, event)
    }

    /// A player broke a chest or a tank: commit its contents to the store and remember the
    /// frequency for the drop.
    fn break_block(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::BreakBlockEvent<'_>,
    ) -> Flow {
        support::break_block(host, event)
    }

    /// The broken block's item drop carries its frequency.
    fn loot_drops(&self, host: &mut dyn ServerHost, event: &mut mod_api::LootDropsEvent<'_>) {
        support::loot_drops(host, event);
    }

    /// An explosion about to break chests or tanks: the store keeps their contents.
    fn explosion_detonate(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::ExplosionDetonateEvent<'_>,
    ) {
        support::explosion_detonate(host, event);
    }

    /// The world opened or closed: load the store lazily, save and forget it on a close.
    fn world_lifecycle(&self, host: &mut dyn ServerHost, event: &mut WorldLifecycleEvent<'_>) {
        support::world_lifecycle(host, event);
    }

    /// End of a running tick: the first tick loads the world, then the store write is debounced.
    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        support::server_tick(host, event);
    }

    /// The server is stopping: flush the store.
    fn server_stopping(&self, host: &mut dyn ServerHost, _event: &mut ServerStoppingEvent) {
        support::server_stopping(host, _event);
    }
}

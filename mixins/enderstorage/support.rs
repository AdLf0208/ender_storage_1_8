// Game-side glue for EnderStorage slice 1 (frequency-linked ender chests).
//
// Injected into `crate::game` as `enderstorage_state`; it needs the game, so
// it is not part of the standalone library (only `runtime.rs` is).
//
// Upstream keeps one inventory object per frequency and every chest points at
// it. Horizon gives each mod block entity its own slots, so the port keeps the
// store in memory (and in `<world>/enderstorage/frequencies.dat`), mirrors it
// into each chest once per tick, and merges the two sides per slot. The
// store file is written at most once a second and on close/stop.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use horizon_client::containers::{BlockEntity, ModBlockEntity, ModContainer, ModFluidTanks};
use horizon_client::items::ItemStack;
use horizon_client::text::Text;
use horizon_core::geometry::Direction;
use horizon_world::nbt::{Compound, JavaString, Tag};
use horizon_world::registry::BlockStateId;

use crate::game::enderstorage::{
    BUCKET, Band, Colour, Frequency, SLOTS, Slot, Store, TANK_CAPACITY, Tank, TankStore, commit,
    commit_tank, switch, switch_tank, sync, sync_tank,
};
use crate::game::native_hooks::NativeLootDrops;
use crate::game::native_lifecycle::{ServerTickPhase, WorldPhase};
use crate::game::native_server::{NativeCommand, NativeServerPlayer};

/// The chest of slice 1 and the tank of slice 2 (each item has the same id).
const BLOCK: &str = "enderstorage:ender_chest";
const ITEM: &str = "enderstorage:ender_chest";
const TANK: &str = "enderstorage:ender_tank";
const TANK_ITEM: &str = "enderstorage:ender_tank";
/// The item component that carries a frequency (`native_component_types`).
const COMPONENT: &str = "enderstorage:frequency";
/// The store lives beside the world, so backups and the world travel together.
const STORE_DIR: &str = "enderstorage";
const STORE_FILE: &str = "frequencies.dat";
const TANK_FILE: &str = "tanks.dat";
/// Write at most once a second; a crash loses at most that much.
const SAVE_INTERVAL_TICKS: u64 = 20;

/// One item slot as the store persists it: the game's `ItemStack.CODEC` as
/// SNBT (`{id:"minecraft:diamond",count:5}`), which is lossless and canonical.
impl Slot for ItemStack {
    fn encode(&self) -> String {
        horizon_world::snbt_text::to_snbt(&horizon_client::save::item_to_nbt(self))
    }

    fn decode(text: &str) -> Option<Self> {
        let (tag, _) = horizon_world::nbt::parse_snbt(text).ok()?;
        horizon_client::save::item_from_nbt(&tag)
    }
}

/// The port's state: the store, each chest's last synced slots, and the
/// deferred work a hook cannot do in place.
struct Core {
    world: Option<PathBuf>,
    store: Store<ItemStack>,
    /// Each frequency's tank (slice 2), the world's truth.
    tanks: TankStore,
    /// Each tank block entity's last synced fluid, by its own uid.
    tank_snapshots: HashMap<u64, Tank>,
    /// Each block entity's last synced slots, by its own uid (it moves with
    /// the block entity, so a piston move does not confuse the merge).
    snapshots: HashMap<u64, Vec<Option<ItemStack>>>,
    /// Frequencies a chest must take: from its item on placement, or a command.
    pending: Vec<(String, [i32; 3], Frequency)>,
    /// The frequency of a chest that just broke, for its item's drop.
    broken: HashMap<(String, [i32; 3]), Frequency>,
    /// Players whose inventory must be re-sent after a held-stack change.
    sync_players: HashSet<u128>,
    /// The previous tick's queued resends: sent one tick later, after the
    /// guest turn has written its held stack back into the hub record.
    sync_ready: HashSet<u128>,
    /// The exact held slot a bucket swap left for each player, applied to the
    /// hub record before the delayed resend (the pin's integrated server does
    /// not carry a mod's held write to the client otherwise).
    pending_held: HashMap<u128, (usize, Option<ItemStack>)>,
    dirty: bool,
    tanks_dirty: bool,
    ticks: u64,
    last_save: u64,
}

impl Default for Core {
    fn default() -> Self {
        Core {
            world: None,
            store: Store::new(),
            tanks: TankStore::new(),
            tank_snapshots: HashMap::new(),
            snapshots: HashMap::new(),
            pending: Vec::new(),
            broken: HashMap::new(),
            sync_players: HashSet::new(),
            sync_ready: HashSet::new(),
            pending_held: HashMap::new(),
            dirty: false,
            tanks_dirty: false,
            ticks: 0,
            last_save: 0,
        }
    }
}

static CORE: Mutex<Option<Core>> = Mutex::new(None);

fn core() -> std::sync::MutexGuard<'static, Option<Core>> {
    CORE.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// A uid for a block entity, unique across a save: the time and a counter.
fn fresh_uid() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let count = NEXT.fetch_add(1, Ordering::Relaxed);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos() as u64);
    now.rotate_left(17) ^ count.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

fn nbt_key(name: &str) -> JavaString {
    JavaString::from(name)
}

/// The chest's frequency from its saved compound; the default when absent or
/// unreadable. Inserts the default so the chest keeps it.
fn be_frequency(entity: &mut ModBlockEntity) -> Frequency {
    if let Some(Tag::String(text)) = entity.nbt.get(&nbt_key("frequency"))
        && let Some(frequency) = Frequency::parse(&text.to_string_lossy())
    {
        return frequency;
    }
    let frequency = Frequency::DEFAULT;
    entity.nbt.insert(
        nbt_key("frequency"),
        Tag::String(JavaString::from(frequency.text().as_str())),
    );
    frequency
}

/// The chest's frequency, read-only (the default when absent).
fn read_frequency(entity: &ModBlockEntity) -> Frequency {
    entity
        .nbt
        .get(&nbt_key("frequency"))
        .and_then(|tag| match tag {
            Tag::String(text) => Frequency::parse(&text.to_string_lossy()),
            _ => None,
        })
        .unwrap_or(Frequency::DEFAULT)
}

fn set_be_frequency(entity: &mut ModBlockEntity, frequency: Frequency) {
    entity.nbt.insert(
        nbt_key("frequency"),
        Tag::String(JavaString::from(frequency.text().as_str())),
    );
}

/// The chest's uid, made and kept on first use.
fn be_uid(entity: &mut ModBlockEntity) -> u64 {
    if let Some(Tag::Long(uid)) = entity.nbt.get(&nbt_key("uid")) {
        return *uid as u64;
    }
    let uid = fresh_uid();
    entity.nbt.insert(nbt_key("uid"), Tag::Long(uid as i64));
    uid
}

/// The band the next dye click sets (left, middle, right, left...).
fn be_band(entity: &mut ModBlockEntity) -> Band {
    match entity.nbt.get(&nbt_key("band")) {
        Some(Tag::Byte(index)) => Band::from_index(*index as u8),
        _ => Band::Left,
    }
}

fn set_be_band(entity: &mut ModBlockEntity, band: Band) {
    entity
        .nbt
        .insert(nbt_key("band"), Tag::Byte(band.index() as i8));
}

fn state_name(game: &crate::game::Game<'_>, state: BlockStateId) -> String {
    game.res.facts().get(state).name.to_string()
}

fn block_at(game: &crate::game::Game<'_>, pos: [i32; 3]) -> String {
    state_name(game, game.world.block(pos[0], pos[1], pos[2]))
}

/// `BlockEntity.setChanged`: the chunk is written at the next save.
fn mark_modified(game: &mut crate::game::Game<'_>, pos: [i32; 3]) {
    game.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
}

/// The player a player hook acts for: the guest swapped in, or, in single
/// player, the owner (`native_acting_player` is none there).
fn acting_uuid(game: &crate::game::Game<'_>) -> Option<u128> {
    game.native_acting_player()
        .map(|player| player.uuid)
        .or_else(|| {
            game.native_server_players()
                .first()
                .map(|player| player.uuid)
        })
}

fn queue_sync(game: &crate::game::Game<'_>) {
    let Some(uuid) = acting_uuid(game) else {
        return;
    };
    let mut guard = core();
    guard
        .get_or_insert_with(Core::default)
        .sync_players
        .insert(uuid);
}

/// Swap one held `from` for one `to` through the pin's inventory helpers: a
/// direct held-stack write does not reach this pin's integrated client, while
/// `native_take_items` + `native_give_stack` send the change to the screen.
fn swap_one(game: &mut crate::game::Game<'_>, from: &str, to: &str) {
    let Some(uuid) = acting_uuid(game) else {
        return;
    };
    let taken = game.native_take_items(from, 1);
    if taken == 0 {
        return;
    }
    let _ = game.native_give_stack(uuid, ItemStack::new(to, 1));
    // Record the resulting held slot for the delayed resend, then queue it.
    let slot = game.selected;
    let held = game.held().cloned();
    {
        let mut guard = core();
        guard
            .get_or_insert_with(Core::default)
            .pending_held
            .insert(uuid, (slot, held));
    }
    queue_sync(game);
}

/// Replace the acting player's held stack (their state is swapped in) and
/// queue the inventory resend the hub's per-tick diff misses.
fn replace_held(game: &mut crate::game::Game<'_>, stack: ItemStack) {
    let slot = game.held_slot();
    *game.held_mut() = Some(stack);
    if game.lan_acting.is_none() {
        game.hand_stack_replaced(slot);
    }
    queue_sync(game);
}

/// Take one item from the acting player's held stack.
fn consume_held(game: &mut crate::game::Game<'_>, mut stack: ItemStack) {
    if game.creative {
        return;
    }
    stack.count = stack.count.saturating_sub(1);
    let slot = game.held_slot();
    *game.held_mut() = (stack.count > 0).then_some(stack);
    if game.lan_acting.is_none() {
        game.hand_stack_replaced(slot);
    }
    queue_sync(game);
}

fn stack_frequency(game: &crate::game::Game<'_>, stack: &ItemStack) -> Option<Frequency> {
    match game.native_component(stack, COMPONENT).ok().flatten()? {
        Tag::String(text) => Frequency::parse(&text.to_string_lossy()),
        _ => None,
    }
}

fn store_path(world: &Path) -> PathBuf {
    world.join(STORE_DIR).join(STORE_FILE)
}

fn load_store(world: &Path) -> Store<ItemStack> {
    let path = store_path(world);
    let Ok(text) = fs::read_to_string(&path) else {
        return Store::new();
    };
    let (store, warnings) = Store::<ItemStack>::decode(&text);
    for warning in warnings {
        log::warn!("[enderstorage] {}: {warning}", path.display());
    }
    store
}

/// Write the store atomically: a `.tmp` beside it, then a rename.
fn save_store(world: &Path) {
    let text = {
        let guard = core();
        let Some(state) = guard.as_ref() else {
            return;
        };
        if !state.dirty {
            return;
        }
        state.store.encode()
    };
    let dir = world.join(STORE_DIR);
    if let Err(error) = fs::create_dir_all(&dir) {
        log::warn!("[enderstorage] could not create {}: {error}", dir.display());
        return;
    }
    let path = store_path(world);
    let temporary = dir.join(format!("{STORE_FILE}.tmp"));
    if let Err(error) = fs::write(&temporary, text.as_bytes()) {
        log::warn!(
            "[enderstorage] could not write {}: {error}",
            temporary.display()
        );
        return;
    }
    if let Err(error) = fs::rename(&temporary, &path) {
        log::warn!(
            "[enderstorage] could not replace {}: {error}",
            path.display()
        );
        return;
    }
    let mut guard = core();
    if let Some(state) = guard.as_mut() {
        state.dirty = false;
        state.last_save = state.ticks;
    }
}

fn tank_path(world: &Path) -> PathBuf {
    world.join(STORE_DIR).join(TANK_FILE)
}

fn load_tanks(world: &Path) -> TankStore {
    let path = tank_path(world);
    let Ok(text) = fs::read_to_string(&path) else {
        return TankStore::new();
    };
    let (tanks, warnings) = TankStore::decode(&text);
    for warning in warnings {
        log::warn!("[enderstorage] {}: {warning}", path.display());
    }
    tanks
}

/// Write the tank store atomically, like the item store.
fn save_tanks(world: &Path) {
    let text = {
        let guard = core();
        let Some(state) = guard.as_ref() else {
            return;
        };
        if !state.tanks_dirty {
            return;
        }
        state.tanks.encode()
    };
    let dir = world.join(STORE_DIR);
    if let Err(error) = fs::create_dir_all(&dir) {
        log::warn!("[enderstorage] could not create {}: {error}", dir.display());
        return;
    }
    let path = tank_path(world);
    let temporary = dir.join(format!("{TANK_FILE}.tmp"));
    if let Err(error) = fs::write(&temporary, text.as_bytes()) {
        log::warn!(
            "[enderstorage] could not write {}: {error}",
            temporary.display()
        );
        return;
    }
    if let Err(error) = fs::rename(&temporary, &path) {
        log::warn!(
            "[enderstorage] could not replace {}: {error}",
            path.display()
        );
        return;
    }
    let mut guard = core();
    if let Some(state) = guard.as_mut() {
        state.tanks_dirty = false;
    }
}

/// The entity's first tank, as the pin keeps it under `fluids`.
fn read_be_tank(nbt: &Compound) -> Tank {
    let Some(Tag::List(list)) = nbt.get(&nbt_key("fluids")) else {
        return Tank::EMPTY;
    };
    let Some(Tag::Compound(entry)) = list.first() else {
        return Tank::EMPTY;
    };
    let fluid = match entry.get(&nbt_key("Fluid")) {
        Some(Tag::String(fluid)) if fluid.to_string_lossy() != "minecraft:empty" => {
            fluid.to_string_lossy()
        }
        _ => return Tank::EMPTY,
    };
    let amount = match entry.get(&nbt_key("Amount")) {
        Some(Tag::Int(amount)) => *amount,
        _ => 0,
    };
    Tank::new(&fluid, amount)
}

/// Write the first tank back in the pin's `fluids` shape, keeping the tank
/// numbers in place.
fn write_be_tank(nbt: &mut Compound, tank: &Tank) {
    let mut entry = Compound::new();
    entry.insert(
        nbt_key("Fluid"),
        Tag::String(JavaString::from(
            tank.fluid.as_deref().unwrap_or("minecraft:empty"),
        )),
    );
    entry.insert(nbt_key("Amount"), Tag::Int(tank.amount));
    nbt.insert(nbt_key("fluids"), Tag::List(vec![Tag::Compound(entry)]));
}

/// The tank block entity at `pos`, if it is ours.
fn tank_at(game: &crate::game::Game<'_>, pos: [i32; 3]) -> Option<Tank> {
    match game.world.block_entities.get(&(pos[0], pos[1], pos[2])) {
        Some(BlockEntity::Mod(entity)) if entity.id == TANK => Some(read_be_tank(&entity.nbt)),
        _ => None,
    }
}

/// The clicked face as the fluid helpers name it.
fn face_direction(face: [i32; 3]) -> Direction {
    match face {
        [0, -1, 0] => Direction::Down,
        [0, 1, 0] => Direction::Up,
        [0, 0, -1] => Direction::North,
        [0, 0, 1] => Direction::South,
        [-1, 0, 0] => Direction::West,
        _ => Direction::East,
    }
}

/// A filled bucket's fluid, for the bucket-to-tank direction.
fn bucket_fluid(item: &str) -> Option<&'static str> {
    match item {
        "minecraft:water_bucket" => Some("minecraft:water"),
        "minecraft:lava_bucket" => Some("minecraft:lava"),
        _ => None,
    }
}

/// The bucket a drained fluid fills.
fn fluid_bucket(fluid: &str) -> Option<&'static str> {
    match fluid {
        "minecraft:water" => Some("minecraft:water_bucket"),
        "minecraft:lava" => Some("minecraft:lava_bucket"),
        _ => None,
    }
}

/// One bucket from the held stack into the tank: answers whether the click is
/// the tank's (the caller then claims it).
fn tank_bucket_click(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    face: Direction,
    held: &ItemStack,
) -> bool {
    let dimension = game.native_dimension().to_owned();
    if &*held.item == "minecraft:bucket" {
        // Only a whole bucket fills a bucket; a partly filled tank refuses.
        let Some(tank) = tank_at(game, pos) else {
            return false;
        };
        if tank.amount < BUCKET {
            return true;
        }
        let Some((fluid, taken)) = game.native_fluid_extract(&dimension, pos, face, BUCKET) else {
            return true;
        };
        if taken < BUCKET {
            // Put a partial draw back; an empty bucket holds no fraction.
            game.native_fluid_insert(pos, face, &fluid, taken);
            return true;
        }
        let Some(bucket) = fluid_bucket(&fluid) else {
            game.native_fluid_insert(pos, face, &fluid, taken);
            return true;
        };
        swap_one(game, "minecraft:bucket", bucket);
        return true;
    }
    let Some(fluid) = bucket_fluid(&held.item) else {
        return false;
    };
    let Some(tank) = tank_at(game, pos) else {
        return false;
    };
    if tank.room_for(fluid) < BUCKET {
        return true;
    }
    if game.native_fluid_insert(pos, face, fluid, BUCKET) == 0 {
        swap_one(game, &held.item, "minecraft:bucket");
    }
    true
}

/// A server player empties a water bucket at the tank: one bucket goes into
/// the shared tank and nothing is poured (`native_bucket_empty`, API 179).
/// The hook carries the fluid target, not the clicked face, so the insert
/// uses the `up` side (the tank's table reaches every side).
#[allow(private_interfaces)]
pub fn on_bucket_empty(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    fluid: &str,
    cancel: &mut bool,
) {
    if *cancel {
        return;
    }
    if block_at(game, pos) == TANK {
        // The tank owns the click, full or not: no fluid is poured beside it.
        *cancel = true;
    } else {
        return;
    }
    if fluid != "minecraft:water" {
        return;
    }
    let Some(held) = game.held().cloned() else {
        return;
    };
    if &*held.item != "minecraft:water_bucket" {
        return;
    }
    let Some(tank) = tank_at(game, pos) else {
        return;
    };
    if tank.room_for(fluid) < BUCKET {
        return;
    }
    if game.native_fluid_insert(pos, Direction::Up, fluid, BUCKET) == 0 {
        let mut stack = held;
        stack.item = "minecraft:bucket".into();
        stack.count = 1;
        replace_held(game, stack);
    }
}

/// The tank block declares one 16-bucket tank, reachable from every side
/// (`native_block_entity_fluids`, API 54).
#[allow(private_interfaces)]
pub fn on_block_entity_fluids(block: &str, fluids: &mut Option<ModFluidTanks>) {
    if block == TANK {
        let all = vec![0usize];
        *fluids = Some(ModFluidTanks {
            tanks: vec![TANK_CAPACITY],
            faces: [
                all.clone(),
                all.clone(),
                all.clone(),
                all.clone(),
                all.clone(),
                all,
            ],
            inputs: vec![0],
            outputs: vec![0],
        });
    }
}

/// Give the chest block a block entity (`native_block_entity`, API 15).
#[allow(private_interfaces)]
pub fn on_block_entity(block: &str, entity: &mut Option<(String, Compound)>) {
    if block == BLOCK || block == TANK {
        *entity = Some((block.to_owned(), Compound::new()));
    }
}

/// The chest block entity ticks (`native_block_entity_ticks`, API 15).
#[allow(private_interfaces)]
pub fn on_block_entity_ticks(block: &str, ticks: &mut bool) {
    if block == BLOCK || block == TANK {
        *ticks = true;
    }
}

/// 27 slots reachable from every side, shown as the vanilla 9x3 menu
/// (`native_block_entity_container`, API 16/17).
#[allow(private_interfaces)]
pub fn on_block_entity_container(block: &str, container: &mut Option<ModContainer>) {
    if block != BLOCK {
        return;
    }
    let all: Vec<usize> = (0..SLOTS).collect();
    *container = Some(ModContainer {
        slots: SLOTS,
        faces: [
            all.clone(),
            all.clone(),
            all.clone(),
            all.clone(),
            all.clone(),
            all,
        ],
        inputs: (0..SLOTS).collect(),
        outputs: (0..SLOTS).collect(),
        menu: Some("minecraft:generic_9x3".to_owned()),
    });
}

/// One server tick of a chest: apply a queued frequency, then merge its slots
/// with the store (`native_block_entity_tick`, API 17).
#[allow(private_interfaces)]
pub fn on_block_entity_tick(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    entity: &mut ModBlockEntity,
) {
    if entity.id == TANK {
        on_tank_tick(game, pos, entity);
        return;
    }
    if entity.id != BLOCK {
        return;
    }
    let dimension = game.native_dimension().to_owned();
    let mut frequency = be_frequency(entity);
    let uid = be_uid(entity);
    let mut guard = core();
    let state = guard.get_or_insert_with(Core::default);
    if let Some(index) = state
        .pending
        .iter()
        .position(|(dim, at, _)| dim == &dimension && *at == pos)
    {
        let (_, _, wanted) = state.pending.remove(index);
        if wanted != frequency {
            let snapshot = state.snapshots.get(&uid).cloned();
            let (items, changed) = switch(
                &entity.items,
                snapshot.as_deref(),
                &mut state.store,
                frequency,
                wanted,
            );
            if changed {
                state.dirty = true;
            }
            entity.items = items.clone();
            state.snapshots.insert(uid, items);
            set_be_frequency(entity, wanted);
            frequency = wanted;
            mark_modified(game, pos);
        }
    }
    let snapshot = state.snapshots.get(&uid).cloned();
    let synced = sync(
        &entity.items,
        snapshot.as_deref(),
        &mut state.store,
        frequency,
    );
    if synced.store_changed {
        state.dirty = true;
    }
    entity.items = synced.items.clone();
    state.snapshots.insert(uid, synced.items);
}

/// One tick of a tank: a queued frequency first, then the fluid merge with
/// the shared tank store (`native_block_entity_fluids` keeps it in the
/// entity's `fluids`).
fn on_tank_tick(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    entity: &mut ModBlockEntity,
) {
    let dimension = game.native_dimension().to_owned();
    let mut frequency = be_frequency(entity);
    let uid = be_uid(entity);
    let mut current = read_be_tank(&entity.nbt);
    let mut guard = core();
    let state = guard.get_or_insert_with(Core::default);
    let index = state
        .pending
        .iter()
        .position(|(dim, at, _)| dim == &dimension && *at == pos);
    if let Some(index) = index {
        let (_, _, wanted) = state.pending.remove(index);
        if wanted != frequency {
            let snapshot = state.tank_snapshots.get(&uid).cloned();
            let (tank, changed) = switch_tank(
                &current,
                snapshot.as_ref(),
                &mut state.tanks,
                frequency,
                wanted,
            );
            if changed {
                state.tanks_dirty = true;
            }
            current = tank;
            state.tank_snapshots.insert(uid, current.clone());
            set_be_frequency(entity, wanted);
            frequency = wanted;
            mark_modified(game, pos);
        }
    }
    let snapshot = state.tank_snapshots.get(&uid).cloned();
    let synced = sync_tank(&current, snapshot.as_ref(), &mut state.tanks, frequency);
    if synced.store_changed {
        state.tanks_dirty = true;
    }
    if synced.tank != current {
        write_be_tank(&mut entity.nbt, &synced.tank);
    }
    state.tank_snapshots.insert(uid, synced.tank);
}

/// A right click on a tank with a bucket swaps a bucket between the tank and
/// the held stack; the click is claimed so a full tank never pours into the
/// world. A chest with a dye sets the next colour band and consumes one dye
/// (`native_right_click_block`, API 21).
#[allow(private_interfaces)]
pub fn on_right_click_block(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    face: [i32; 3],
    canceled: &mut bool,
) {
    if *canceled {
        return;
    }
    if block_at(game, pos) == TANK {
        let Some(held) = game.held().cloned() else {
            return;
        };
        // `native_bucket_empty` only reaches a LAN guest ("a game's own player
        // is not asked"), so claim water here too: the tank owns the click
        // whether the actor is the host or a guest. A guest that reaches
        // `bucket_empty` never gets there because this cancels the use first.
        if &*held.item == "minecraft:bucket" || bucket_fluid(&held.item).is_some() {
            if tank_bucket_click(game, pos, face_direction(face), &held) {
                *canceled = true;
            }
        }
        return;
    }
    if block_at(game, pos) != BLOCK {
        return;
    }
    let Some(held) = game.held().cloned() else {
        return;
    };
    let Some(colour) = Colour::from_dye(&held.item) else {
        return;
    };
    let key = (pos[0], pos[1], pos[2]);
    let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&key) else {
        return;
    };
    let frequency = be_frequency(entity);
    let band = be_band(entity);
    let previous = frequency.band(band);
    let updated = frequency.with(band, colour);
    let uid = be_uid(entity);
    {
        let mut guard = core();
        let state = guard.get_or_insert_with(Core::default);
        let snapshot = state.snapshots.get(&uid).cloned();
        let (items, changed) = switch(
            &entity.items,
            snapshot.as_deref(),
            &mut state.store,
            frequency,
            updated,
        );
        if changed {
            state.dirty = true;
        }
        entity.items = items.clone();
        state.snapshots.insert(uid, items);
    }
    set_be_frequency(entity, updated);
    set_be_band(entity, band.next());
    mark_modified(game, pos);
    consume_held(game, held);
    if let Some(uuid) = acting_uuid(game) {
        let line = format!(
            "EnderStorage: {} ({} {} -> {}; next click sets {})",
            updated.label(),
            band.name(),
            previous.name(),
            colour.name(),
            band.next().name()
        );
        game.native_action_bar(uuid, Text::literal(line));
    }
    *canceled = true;
}

/// A chest placed from an item takes that item's frequency
/// (`native_place_block`, API 14); the block entity's first tick applies it.
#[allow(private_interfaces)]
pub fn on_place_block(
    game: &mut crate::game::Game<'_>,
    stack: &ItemStack,
    pos: [i32; 3],
    state: BlockStateId,
) {
    let name = state_name(game, state);
    if name != BLOCK && name != TANK {
        return;
    }
    let Some(frequency) = stack_frequency(game, stack) else {
        return;
    };
    let dimension = game.native_dimension().to_owned();
    let mut guard = core();
    let state = guard.get_or_insert_with(Core::default);
    state
        .pending
        .retain(|(dim, at, _)| !(dim == &dimension && *at == pos));
    state.pending.push((dimension, pos, frequency));
}

/// A player broke a chest: commit its last changes, empty its slots so the
/// contents do not spill a second copy, and keep the frequency for the drop
/// (`native_destroy_block`, API 14).
#[allow(private_interfaces)]
pub fn on_destroy_block(
    game: &mut crate::game::Game<'_>,
    pos: [i32; 3],
    state: BlockStateId,
    canceled: &mut bool,
) {
    let name = state_name(game, state);
    if *canceled || (name != BLOCK && name != TANK) {
        return;
    }
    let key = (pos[0], pos[1], pos[2]);
    let dimension = game.native_dimension().to_owned();
    let Some(BlockEntity::Mod(entity)) = game.world.block_entities.get_mut(&key) else {
        return;
    };
    let frequency = be_frequency(entity);
    let uid = be_uid(entity);
    let mut guard = core();
    let state = guard.get_or_insert_with(Core::default);
    if name == TANK {
        let tank = read_be_tank(&entity.nbt);
        let snapshot = state.tank_snapshots.get(&uid).cloned();
        if commit_tank(&tank, snapshot.as_ref(), &mut state.tanks, frequency) {
            state.tanks_dirty = true;
        }
        write_be_tank(&mut entity.nbt, &Tank::EMPTY);
        state.tank_snapshots.remove(&uid);
    } else {
        let snapshot = state.snapshots.get(&uid).cloned();
        if commit(
            &entity.items,
            snapshot.as_deref(),
            &mut state.store,
            frequency,
        ) {
            state.dirty = true;
        }
        entity.items = vec![None; SLOTS];
        state.snapshots.remove(&uid);
    }
    state.broken.insert((dimension, pos), frequency);
    if state.broken.len() > 64 {
        state.broken.clear();
    }
}

/// The broken chest's item drop carries its frequency (`native_loot_drops`,
/// API 75).
#[allow(private_interfaces)]
pub fn on_loot_drops(
    game: &mut crate::game::Game<'_>,
    context: &NativeLootDrops<'_>,
    drops: &mut Vec<ItemStack>,
) {
    let Some((pos, state)) = context.block else {
        return;
    };
    let name = state_name(game, state);
    if name != BLOCK && name != TANK {
        return;
    }
    let item = if name == TANK { TANK_ITEM } else { ITEM };
    let key = (context.dimension.to_owned(), pos);
    let frequency = {
        let mut guard = core();
        let state = guard.get_or_insert_with(Core::default);
        state.broken.remove(&key).or_else(|| {
            game.world
                .block_entities
                .get(&(pos[0], pos[1], pos[2]))
                .and_then(|entity| match entity {
                    BlockEntity::Mod(entity) => Some(read_frequency(entity)),
                    _ => None,
                })
        })
    };
    let Some(frequency) = frequency else {
        return;
    };
    // The content mod's loot table does not reach this pin's table registry
    // from a `--dev-mod` data pack, so make the drop here; a table that does
    // load is deduplicated rather than doubled.
    drops.retain(|drop| &*drop.item != item);
    let mut stack = ItemStack {
        item: item.into(),
        count: 1,
        damage: 0,
        extra: None,
    };
    if game
        .native_set_component(
            &mut stack,
            COMPONENT,
            Tag::String(JavaString::from(frequency.text().as_str())),
        )
        .is_ok()
    {
        drops.push(stack);
    }
}

/// An explosion about to break a chest: commit and empty it like a break, so
/// the store keeps the contents and the block drops its frequency
/// (`native_explosion_detonate`, API 22).
#[allow(private_interfaces)]
pub fn on_explosion_detonate(
    game: &mut crate::game::Game<'_>,
    dimension: &str,
    blocks: &mut Vec<[i32; 3]>,
) {
    let mut found = Vec::new();
    for pos in blocks.iter() {
        let Some(BlockEntity::Mod(entity)) =
            game.world.block_entities.get(&(pos[0], pos[1], pos[2]))
        else {
            continue;
        };
        if entity.id == BLOCK || entity.id == TANK {
            found.push((*pos, read_frequency(entity), entity.id == TANK));
        }
    }
    if found.is_empty() {
        return;
    }
    let mut guard = core();
    let state = guard.get_or_insert_with(Core::default);
    for (pos, frequency, is_tank) in found {
        let Some(BlockEntity::Mod(entity)) =
            game.world.block_entities.get_mut(&(pos[0], pos[1], pos[2]))
        else {
            continue;
        };
        let uid = be_uid(entity);
        if is_tank {
            let tank = read_be_tank(&entity.nbt);
            let snapshot = state.tank_snapshots.get(&uid).cloned();
            if commit_tank(&tank, snapshot.as_ref(), &mut state.tanks, frequency) {
                state.tanks_dirty = true;
            }
            write_be_tank(&mut entity.nbt, &Tank::EMPTY);
            state.tank_snapshots.remove(&uid);
        } else {
            let snapshot = state.snapshots.get(&uid).cloned();
            if commit(
                &entity.items,
                snapshot.as_deref(),
                &mut state.store,
                frequency,
            ) {
                state.dirty = true;
            }
            entity.items = vec![None; SLOTS];
            state.snapshots.remove(&uid);
        }
        state.broken.insert((dimension.to_owned(), pos), frequency);
    }
    if state.broken.len() > 64 {
        state.broken.clear();
    }
}

/// The world opened or closed: load or save the store (`world_lifecycle`).
#[allow(private_interfaces)]
pub fn on_world_lifecycle(game: &mut crate::game::Game<'_>, phase: WorldPhase) {
    match phase {
        WorldPhase::WorldOpenReady => {
            let world = game.native_world_dir();
            let store = world.as_deref().map(load_store).unwrap_or_default();
            let tanks = world.as_deref().map(load_tanks).unwrap_or_default();
            let mut guard = core();
            let state = guard.get_or_insert_with(Core::default);
            state.world = world;
            state.store = store;
            state.tanks = tanks;
            state.snapshots.clear();
            state.tank_snapshots.clear();
            state.pending.clear();
            state.broken.clear();
            state.sync_players.clear();
            state.sync_ready.clear();
            state.dirty = false;
            state.tanks_dirty = false;
            state.ticks = 0;
            state.last_save = 0;
        }
        WorldPhase::WorldCloseFinal | WorldPhase::WorldCloseFailed => {
            let world = {
                let guard = core();
                guard.as_ref().and_then(|state| state.world.clone())
            };
            if let Some(world) = world {
                save_store(&world);
                save_tanks(&world);
            }
            let mut guard = core();
            if let Some(state) = guard.as_mut() {
                state.world = None;
                state.store = Store::new();
                state.tanks = TankStore::new();
                state.snapshots.clear();
                state.tank_snapshots.clear();
                state.pending.clear();
                state.broken.clear();
                state.sync_players.clear();
            state.sync_ready.clear();
                state.dirty = false;
                state.tanks_dirty = false;
            }
        }
        WorldPhase::WorldOpenFailed => {
            let mut guard = core();
            if let Some(state) = guard.as_mut() {
                state.world = None;
                state.store = Store::new();
                state.tanks = TankStore::new();
                state.snapshots.clear();
                state.tank_snapshots.clear();
                state.pending.clear();
                state.broken.clear();
                state.dirty = false;
                state.tanks_dirty = false;
            }
        }
    }
}

/// End of a running tick: debounce the store write, re-send changed hands
/// (`server_tick`).
#[allow(private_interfaces)]
pub fn on_server_tick(game: &mut crate::game::Game<'_>, phase: ServerTickPhase) {
    let ServerTickPhase::ServerTickEnd { world_running } = phase else {
        return;
    };
    if !world_running {
        return;
    }
    let (due, world, players) = {
        let mut guard = core();
        let state = guard.get_or_insert_with(Core::default);
        state.ticks += 1;
        let due = (state.dirty || state.tanks_dirty)
            && state.ticks.saturating_sub(state.last_save) >= SAVE_INTERVAL_TICKS;
        (
            due,
            state.world.clone(),
            // One tick behind: a held-stack change is written back into the
            // hub record by `leave_guest` after this tick's action, so an
            // immediate resend would carry the old stack.
            std::mem::replace(
                &mut state.sync_ready,
                std::mem::take(&mut state.sync_players),
            ),
        )
    };
    if due && let Some(world) = world {
        save_store(&world);
        save_tanks(&world);
        let mut guard = core();
        if let Some(state) = guard.as_mut() {
            state.last_save = state.ticks;
        }
    }
    for uuid in players {
        let pending = {
            let mut guard = core();
            guard
                .as_mut()
                .and_then(|state| state.pending_held.remove(&uuid))
        };
        let Some(host) = game.lan.as_mut() else {
            break;
        };
        if let Some(index) = host
            .hub
            .guests
            .iter()
            .position(|guest| guest.placed && !guest.dead && guest.uuid == uuid)
        {
            if let Some((slot, stack)) = pending {
                let guest = &mut host.hub.guests[index];
                if slot >= 36 {
                    guest.offhand = stack;
                } else {
                    if guest.inventory.len() <= slot {
                        guest.inventory.resize(36, None);
                    }
                    guest.inventory[slot] = stack;
                }
            }
            host.hub.send_inventory(index);
        }
    }
}

/// The server is stopping: flush the store (`server_stopping`).
#[allow(private_interfaces)]
pub fn on_server_stopping(_game: &mut crate::game::Game<'_>) {
    let world = {
        let guard = core();
        guard.as_ref().and_then(|state| state.world.clone())
    };
    if let Some(world) = world {
        save_store(&world);
        save_tanks(&world);
    }
}

/// Register the component id before content loads (`component_types`, API 65).
#[allow(private_interfaces)]
pub fn register_component_types(types: &mut Vec<String>) {
    types.push(COMPONENT.to_owned());
}

/// `/enderstorage freq|info` (`register_commands`, API 8).
#[allow(private_interfaces)]
pub fn on_register_commands(commands: &mut Vec<NativeCommand>) {
    commands.push(NativeCommand::new(
        "enderstorage freq held <left:word> <middle:word> <right:word>",
    ));
    commands.push(NativeCommand::op(
        "enderstorage freq <pos:block_pos> <left:word> <middle:word> <right:word>",
        2,
    ));
    commands.push(NativeCommand::new("enderstorage info <pos:block_pos>"));
}

fn parse_colours(left: &str, middle: &str, right: &str) -> Option<Frequency> {
    Some(Frequency::new(
        Colour::from_name(left)?,
        Colour::from_name(middle)?,
        Colour::from_name(right)?,
    ))
}

fn parse_pos(words: &[&str]) -> Option<[i32; 3]> {
    Some([
        words.first()?.parse().ok()?,
        words.get(1)?.parse().ok()?,
        words.get(2)?.parse().ok()?,
    ])
}

/// The `/enderstorage` commands (`perform_command`, API 8). The position form
/// queues the change for the chest's next tick, as the block entity is not in
/// the world here.
#[allow(private_interfaces)]
pub fn on_perform_command(
    game: &mut crate::game::Game<'_>,
    _player: &NativeServerPlayer,
    command: &str,
    ran: &mut bool,
) {
    let words: Vec<&str> = command.split_whitespace().collect();
    match words.as_slice() {
        ["enderstorage", "freq", "held", left, middle, right] => {
            *ran = true;
            let Some(frequency) = parse_colours(left, middle, right) else {
                game.native_feedback(Text::literal(
                    "Unknown colour: use a wool colour name (white, red, light_blue...).",
                ));
                return;
            };
            let Some(held) = game.held().cloned() else {
                game.native_feedback(Text::literal(
                    "Hold an Ender Chest or Ender Tank to set its frequency.",
                ));
                return;
            };
            if &*held.item != ITEM && &*held.item != TANK_ITEM {
                game.native_feedback(Text::literal(
                    "Hold an Ender Chest or Ender Tank to set its frequency.",
                ));
                return;
            }
            let mut stack = held;
            if game
                .native_set_component(
                    &mut stack,
                    COMPONENT,
                    Tag::String(JavaString::from(frequency.text().as_str())),
                )
                .is_err()
            {
                game.native_feedback(Text::literal("Could not set the frequency."));
                return;
            }
            replace_held(game, stack);
            game.native_feedback(Text::literal(format!(
                "Ender Chest frequency: {}",
                frequency.label()
            )));
        }
        ["enderstorage", "freq", x, y, z, left, middle, right] => {
            *ran = true;
            let Some(pos) = parse_pos(&[*x, *y, *z]) else {
                game.native_feedback(Text::literal(
                    "Usage: /enderstorage freq <x> <y> <z> <left> <middle> <right>",
                ));
                return;
            };
            let Some(frequency) = parse_colours(left, middle, right) else {
                game.native_feedback(Text::literal(
                    "Unknown colour: use a wool colour name (white, red, light_blue...).",
                ));
                return;
            };
            let block = block_at(game, pos);
            if block != BLOCK && block != TANK {
                game.native_feedback(Text::literal("No Ender Chest or Ender Tank there."));
                return;
            }
            let dimension = game.native_dimension().to_owned();
            let mut guard = core();
            let state = guard.get_or_insert_with(Core::default);
            state
                .pending
                .retain(|(dim, at, _)| !(dim == &dimension && *at == pos));
            state.pending.push((dimension, pos, frequency));
            drop(guard);
            game.native_feedback(Text::literal(format!(
                "Ender Chest frequency: {}",
                frequency.label()
            )));
        }
        ["enderstorage", "info", x, y, z] => {
            *ran = true;
            let Some(pos) = parse_pos(&[*x, *y, *z]) else {
                game.native_feedback(Text::literal("Usage: /enderstorage info <x> <y> <z>"));
                return;
            };
            let Some(BlockEntity::Mod(entity)) =
                game.world.block_entities.get(&(pos[0], pos[1], pos[2]))
            else {
                game.native_feedback(Text::literal("No Ender Chest or Ender Tank there."));
                return;
            };
            let frequency = read_frequency(entity);
            if entity.id == TANK {
                let tank = read_be_tank(&entity.nbt);
                let stored = {
                    let guard = core();
                    guard
                        .as_ref()
                        .map_or(Tank::EMPTY, |state| state.tanks.tank(frequency))
                };
                game.native_feedback(Text::literal(format!(
                    "Ender Tank: {} ({} {} mB, {} mB in the store)",
                    frequency.label(),
                    tank.fluid.as_deref().unwrap_or("empty"),
                    tank.amount,
                    stored.amount
                )));
                return;
            }
            if entity.id != BLOCK {
                game.native_feedback(Text::literal("No Ender Chest or Ender Tank there."));
                return;
            }
            let items = entity.items.iter().filter(|slot| slot.is_some()).count();
            let stored = {
                let guard = core();
                guard
                    .as_ref()
                    .map_or(0, |state| state.store.count(frequency))
            };
            game.native_feedback(Text::literal(format!(
                "Ender Chest: {} ({items} slots used, {stored} in the store)",
                frequency.label()
            )));
        }
        _ => {}
    }
}

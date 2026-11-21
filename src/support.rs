//! Game-side glue for EnderStorage (frequency-linked ender chests and tanks).
//!
//! [`crate::runtime`] is the pure truth: the frequency, the per-frequency
//! [`Store`](crate::runtime::Store) and [`TankStore`](crate::runtime::TankStore), and the three-way
//! merge with its tests. This module is the only place that reads the game and writes it back: the
//! block entities' saved compounds and item slots, the store files beside the world, the dye click,
//! the bucket swap and the commands.
//!
//! Upstream keeps one inventory object per `Frequency` and every chest points at it. A Horizon mod
//! block entity owns its own slots, so the port keeps the store as the truth and **mirrors** it into
//! each chest once per block-entity tick, merging the two sides per slot (see `notes.md`).
//!
//! The 0.2.0 package replaced the retired `.hnmod` bundle; the mapping from the old hook helpers to
//! this module is in `notes.md` under `## Migration to the SDK format`.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use mod_api::{
    BlockEntityPart, BlockEntityRef, BlockEntityTickEvent, BlockEntityTicksEvent, BlockPart,
    BlockState, Command, CommandArgumentValue, CommandInvocation, CommandSource, Component,
    Compound, FLUIDS, Flow, FluidIdentity, FluidResource, FluidStack, GameMode, Hand, Host,
    HostSide, HostTransaction, Identifier, InteractionResult, ItemKey, ItemStack, LevelRef,
    ModContainer, ModFluidTanks, PlayerRef, Registrar, RunModCommandEvent, ServerHost,
    ServerTickEvent, ServerTickPhase, Tag, Text, Transaction, WorldLifecycleEvent, WorldPhase,
};

use crate::runtime::{
    BUCKET, Band, Colour, Frequency, SLOTS, Slot, Store, TANK_CAPACITY, Tank, TankStore,
    bucket_swap, commit, commit_tank, may_read, switch, switch_tank, sync, sync_tank, tank_write,
};

/// The chest block and its item; the tank block and its item (each pair shares one id).
const BLOCK: &str = "enderstorage:ender_chest";
const ITEM: &str = "enderstorage:ender_chest";
const TANK: &str = "enderstorage:ender_tank";
const TANK_ITEM: &str = "enderstorage:ender_tank";
/// `Inventory`'s off-hand slot (`player_inventory_slots`' numbers).
const OFF_HAND: u32 = 40;

/// The item component that carries a frequency (`component_types`).
pub const COMPONENT: &str = "enderstorage:frequency";
/// The store lives beside the world, so backups and the world travel together.
const STORE_DIR: &str = "enderstorage";
const STORE_FILE: &str = "frequencies.dat";
const TANK_FILE: &str = "tanks.dat";
/// Write at most once a second; a crash loses at most that much.
const SAVE_INTERVAL_TICKS: u64 = 20;
/// The vanilla 9x3 menu the chest's 27 slots open (`generic_9x3`), so hoppers, comparators and
/// shift-clicks work through the vanilla paths.
const MENU: &str = "minecraft:generic_9x3";
/// The water and lava buckets the tank swaps with.
const BUCKET_ITEM: &str = "minecraft:bucket";

// ---------------------------------------------------------------------------
// The store's slot: one item, with the id a stack is and the saved compound it carries
// ---------------------------------------------------------------------------

/// One stored item: the id the game showed, the count, and the stack's saved compound (components
/// and damage included), exactly as `ItemStack.CODEC` writes it.
///
/// [`Slot::encode`] and [`Slot::decode`] are part of the pure store and take no host, so the text is
/// the stack's **network NBT** in hex: the only lossless, host-free form the SDK offers (a saved item
/// compound, read back with `Tag::try_from`). The `i <slot> hex:<bytes>` lines are the port's own
/// format, as the notes say; the `f <frequency>` lines stay readable.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredItem {
    id: String,
    count: u32,
    nbt: Compound,
}

impl StoredItem {
    /// The store's value for a real stack; `None` for an item whose id this game does not know.
    fn of(host: &dyn Host, stack: &ItemStack) -> Option<Self> {
        let nbt = stack.to_nbt(host)?;
        Some(Self {
            id: stack.item().id(host),
            count: stack.count(),
            nbt,
        })
    }

    /// The stack a stored item stands for, including its components and damage.
    fn to_stack(&self, host: &dyn Host) -> Option<ItemStack> {
        if let Some(stack) = ItemStack::from_nbt(host, &self.nbt) {
            return Some(stack);
        }
        // A compound the game refuses falls back to the bare item and count, so a stored item is
        // never lost to a component this build does not know.
        let key: ItemKey = host.item_key(&Identifier::parse(&self.id).ok()?)?;
        ItemStack::new(key, self.count)
    }
}

impl Slot for StoredItem {
    fn encode(&self) -> String {
        let tag = Tag::from(&self.nbt);
        match Vec::<u8>::try_from(&tag) {
            Ok(bytes) => {
                let mut text = String::with_capacity(4 + bytes.len() * 2);
                text.push_str("hex:");
                for byte in bytes {
                    let _ = write!(text, "{byte:02x}");
                }
                text
            }
            // A compound past the network limits cannot be written; an empty slot is better than a
            // half-written file. `Store::encode` still produces a line, which drops the item.
            Err(_) => "hex:".to_owned(),
        }
    }

    fn decode(text: &str) -> Option<Self> {
        let bytes = unhex(text.strip_prefix("hex:")?)?;
        let Tag::Compound(nbt) = Tag::try_from(bytes).ok()? else {
            return None;
        };
        let id = match nbt.get(&"id".into()) {
            Some(Tag::String(id)) => id.to_string_lossy(),
            _ => return None,
        };
        let count = match nbt.get(&"count".into()) {
            Some(Tag::Int(count)) => u32::try_from(*count).ok()?,
            Some(Tag::Long(count)) => u32::try_from(*count).ok()?,
            None => 1,
            _ => return None,
        };
        (count > 0).then_some(Self { id, count, nbt })
    }

    fn key(&self) -> &str {
        &self.id
    }

    fn count(&self) -> u32 {
        self.count
    }

    fn with_count(&self, count: u32) -> Option<Self> {
        if count == 0 {
            return None;
        }
        let mut nbt = self.nbt.clone();
        nbt.insert("count".into(), Tag::Int(i32::try_from(count).ok()?));
        Some(Self {
            id: self.id.clone(),
            count,
            nbt,
        })
    }
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    let digits = text.as_bytes();
    let mut bytes = Vec::with_capacity(text.len() / 2);
    for pair in digits.chunks(2) {
        let high = hex_digit(pair[0])?;
        let low = hex_digit(pair[1])?;
        bytes.push((high << 4) | low);
    }
    Some(bytes)
}

fn hex_digit(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

/// A chest's slots as the store keeps them; a stack the game cannot name is dropped (it is not
/// reachable through any id, so it cannot be shared).
fn stored_slots(host: &dyn Host, items: &[Option<ItemStack>]) -> Vec<Option<StoredItem>> {
    items
        .iter()
        .map(|slot| slot.as_ref().and_then(|stack| StoredItem::of(host, stack)))
        .collect()
}

/// The store's slots as a chest's item slots, `SLOTS` long.
fn stacks(host: &dyn Host, items: &[Option<StoredItem>]) -> Vec<Option<ItemStack>> {
    let mut out: Vec<Option<ItemStack>> = items
        .iter()
        .map(|slot| slot.as_ref().and_then(|item| item.to_stack(host)))
        .collect();
    out.resize(SLOTS, None);
    out.truncate(SLOTS);
    out
}

// ---------------------------------------------------------------------------
// The frequency component
// ---------------------------------------------------------------------------

/// The `enderstorage:frequency` component: the three colours as `left,middle,right`.
struct FrequencyTag;

impl Component for FrequencyTag {
    type Value = Tag;

    fn id(&self) -> &str {
        COMPONENT
    }

    /// A value that is not the frequency string is not this component: the stack reads as a
    /// default-frequency item rather than being guessed at.
    fn decode(&self, value: &Tag) -> Option<Tag> {
        match value {
            Tag::String(_) => Some(value.clone()),
            _ => None,
        }
    }

    fn encode(&self, value: Tag) -> Tag {
        value
    }
}

/// The frequency a stack carries, if any.
fn stack_frequency(stack: &ItemStack) -> Option<Frequency> {
    match stack.get(&FrequencyTag)? {
        Tag::String(text) => Frequency::parse(&text.to_string_lossy()),
        _ => None,
    }
}

/// Write a frequency into a stack's component patch; whether the stack took it.
fn write_stack_frequency(stack: &mut ItemStack, frequency: Frequency) -> bool {
    stack
        .set(&FrequencyTag, Tag::String(frequency.text().as_str().into()))
        .is_ok()
}

// ---------------------------------------------------------------------------
// The port's state
// ---------------------------------------------------------------------------

/// How many sibling chests one access aligns.
const SYNC_MIRRORS_PER_ACCESS: usize = 16;

/// One loaded chest of a frequency: where it is, and the server tick it last ticked in.
#[derive(Clone, Debug)]
struct Mirror {
    tick: u64,
    dimension: String,
    pos: [i32; 3],
}

thread_local! {
    /// Set while one access is being pushed into the other mirrors, so those mirrors' own notices
    /// do not push it back: without it two chests of one frequency would notify each other forever
    /// on the server thread.
    static SYNCING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Commit one chest's access to the store and align every other loaded mirror of the frequency.
fn on_contents_changed(host: &mut dyn ServerHost, pos: mod_api::BlockPos) {
    if SYNCING.with(std::cell::Cell::get) {
        return;
    }
    SYNCING.with(|flag| flag.set(true));
    push_access(host, pos);
    SYNCING.with(|flag| flag.set(false));
}

/// The body of [`on_contents_changed`], under the re-entrancy guard.
fn push_access(host: &mut dyn ServerHost, pos: mod_api::BlockPos) {
    let pos = [pos.x, pos.y, pos.z];
    let Some(level) = host.level(host.dimension()) else {
        return;
    };
    let Some(entity) = our_entity(&*host, level, pos, BLOCK) else {
        return;
    };
    let Some(mut data) = entity.data(&*host) else {
        return;
    };
    let frequency = be_frequency(&mut data);
    let uid = be_uid(&mut data);
    let current = stored_slots(&*host, &read_slots(&*host, entity));
    let seats: Vec<(u64, String, [i32; 3])> = {
        let mut guard = core();
        let Some(state) = guard.as_mut() else {
            return;
        };
        let snapshot = state.snapshots.get(&uid).cloned();
        let committed = commit(&current, snapshot.as_deref(), &mut state.store, frequency);
        if committed {
            state.dirty = true;
        }
        state.snapshots.insert(uid, state.store.slots(frequency));
        let limit = SYNC_MIRRORS_PER_ACCESS;
        state
            .mirrors
            .get(&frequency)
            .map(|mirrors| {
                mirrors
                    .iter()
                    .filter(|(other, _)| **other != uid)
                    .take(limit)
                    .map(|(id, mirror)| (*id, mirror.dimension.clone(), mirror.pos))
                    .collect()
            })
            .unwrap_or_default()
    };
    if seats.is_empty() {
        return;
    }
    let slots = {
        let guard = core();
        guard
            .as_ref()
            .map(|state| state.store.slots(frequency))
            .unwrap_or_default()
    };
    let stacks = stacks(&*host, &slots);
    for (other, dimension, at) in seats {
        if dimension != host.dimension() {
            continue;
        }
        let Some(level) = host.level(&dimension) else {
            continue;
        };
        let Some(entity) = our_entity(&*host, level, at, BLOCK) else {
            continue;
        };
        let _ = host.transaction(|host, tx| -> Result<(), ()> {
            for (slot, stack) in stacks.iter().enumerate() {
                let _ = entity.set_item_slot(
                    host,
                    tx,
                    u32::try_from(slot).unwrap_or(u32::MAX),
                    stack.clone(),
                );
            }
            Ok(())
        });
        let mut guard = core();
        if let Some(state) = guard.as_mut() {
            state.snapshots.insert(other, slots.clone());
        }
    }
}

/// Register this chest as a loaded mirror of `frequency` for the current tick.
///
/// Every loaded chest ticks every tick, so a uid that has not ticked for two ticks is unloaded,
/// broken or retuned. The registry is what lets an access on one chest push the store's state into
/// its siblings at once; Horizon gives each block entity its own inventory, so without that push a
/// sibling keeps a stale copy between ticks and can hand out items the store no longer holds.
fn register_mirror(
    state: &mut Core,
    frequency: Frequency,
    uid: u64,
    dimension: &str,
    pos: [i32; 3],
) {
    let tick = state.ticks;
    let mirrors = state.mirrors.entry(frequency).or_default();
    mirrors.retain(|_, seen| tick.saturating_sub(seen.tick) <= 1);
    mirrors.insert(
        uid,
        Mirror {
            tick,
            dimension: dimension.to_owned(),
            pos,
        },
    );
}

/// The port's state: the store, each chest's last synced slots, and the work a hook defers.
struct Core {
    world: Option<PathBuf>,
    store: Store<StoredItem>,
    /// Each frequency's tank, the world's truth.
    tanks: TankStore,
    /// Each tank block entity's last synced fluid, by its own uid.
    tank_snapshots: HashMap<u64, Tank>,
    /// Each chest's last synced slots, by its own uid (it moves with the block entity, so a piston
    /// move does not confuse the merge).
    snapshots: HashMap<u64, Vec<Option<StoredItem>>>,
    /// Frequencies a chest must take: from its item on placement, or a command.
    pending: Vec<(String, [i32; 3], Frequency)>,
    /// The loaded chests of each frequency, by uid: where each is, and the server tick it last
    /// ticked in. An access on one chest pushes the store's slots into these siblings at once, so
    /// none of them can hand out contents the store no longer holds.
    mirrors: BTreeMap<Frequency, BTreeMap<u64, Mirror>>,
    /// The frequency of a chest that just broke, for its item's drop.
    broken: HashMap<(String, [i32; 3]), Frequency>,
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
            mirrors: BTreeMap::new(),
            broken: HashMap::new(),
            dirty: false,
            tanks_dirty: false,
            ticks: 0,
            last_save: 0,
        }
    }
}

/// The store is world-global (one per frequency, shared by every chest in every dimension), so it is
/// process state rather than a field of one block entity. `Hooks` is `Send + Sync`, and a `static`
/// `Mutex` is the smallest way to keep the one store the game needs.
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

/// The chest's frequency from its saved compound; the default when absent or unreadable. Inserts
/// the default so the chest keeps it.
fn be_frequency(nbt: &mut Compound) -> Frequency {
    if let Some(Tag::String(text)) = nbt.get(&"frequency".into())
        && let Some(frequency) = Frequency::parse(&text.to_string_lossy())
    {
        return frequency;
    }
    let frequency = Frequency::DEFAULT;
    set_be_frequency(nbt, frequency);
    frequency
}

/// The chest's frequency, read-only (the default when absent).
fn read_frequency(nbt: &Compound) -> Frequency {
    nbt.get(&"frequency".into())
        .and_then(|tag| match tag {
            Tag::String(text) => Frequency::parse(&text.to_string_lossy()),
            _ => None,
        })
        .unwrap_or(Frequency::DEFAULT)
}

fn set_be_frequency(nbt: &mut Compound, frequency: Frequency) {
    nbt.insert(
        "frequency".into(),
        Tag::String(frequency.text().as_str().into()),
    );
}

/// The chest's uid, made and kept on first use.
fn be_uid(nbt: &mut Compound) -> u64 {
    if let Some(Tag::Long(uid)) = nbt.get(&"uid".into()) {
        return *uid as u64;
    }
    let uid = fresh_uid();
    nbt.insert("uid".into(), Tag::Long(uid as i64));
    uid
}

/// The band the next dye click sets (left, middle, right, left...).
fn be_band(nbt: &Compound) -> Band {
    match nbt.get(&"band".into()) {
        Some(Tag::Byte(index)) => Band::from_index(*index as u8),
        _ => Band::Left,
    }
}

fn set_be_band(nbt: &mut Compound, band: Band) {
    nbt.insert("band".into(), Tag::Byte(band.index() as i8));
}

/// The entity's first tank, as the fluid declaration keeps it under `fluids`.
fn read_be_tank(nbt: &Compound) -> Tank {
    let Some(Tag::List(list)) = nbt.get(&"fluids".into()) else {
        return Tank::EMPTY;
    };
    let Some(Tag::Compound(entry)) = list.first() else {
        return Tank::EMPTY;
    };
    let fluid = match entry.get(&"Fluid".into()) {
        Some(Tag::String(fluid)) if fluid.to_string_lossy() != "minecraft:empty" => {
            fluid.to_string_lossy()
        }
        _ => return Tank::EMPTY,
    };
    let amount = match entry.get(&"Amount".into()) {
        Some(Tag::Int(amount)) => *amount,
        _ => 0,
    };
    Tank::new(&fluid, amount)
}

/// Write the first tank back in the declaration's `fluids` shape.
fn write_be_tank(nbt: &mut Compound, tank: &Tank) {
    let mut entry = Compound::new();
    entry.insert(
        "Fluid".into(),
        Tag::String(tank.fluid.as_deref().unwrap_or("minecraft:empty").into()),
    );
    entry.insert("Amount".into(), Tag::Int(tank.amount));
    nbt.insert("fluids".into(), Tag::List(vec![Tag::Compound(entry)]));
}

// ---------------------------------------------------------------------------
// The store files
// ---------------------------------------------------------------------------

fn store_path(world: &Path) -> PathBuf {
    world.join(STORE_DIR).join(STORE_FILE)
}

fn tank_path(world: &Path) -> PathBuf {
    world.join(STORE_DIR).join(TANK_FILE)
}

fn load_store(world: &Path) -> Store<StoredItem> {
    let path = store_path(world);
    let Ok(text) = fs::read_to_string(&path) else {
        return Store::new();
    };
    // A corrupt line is skipped by the decoder; its warning has no route to the player at this pin
    // (the SDK exports no logger, and a native mod may not add a crate), and the rest still loads.
    let (store, _warnings) = Store::<StoredItem>::decode(&text);
    store
}

fn load_tanks(world: &Path) -> TankStore {
    let path = tank_path(world);
    let Ok(text) = fs::read_to_string(&path) else {
        return TankStore::new();
    };
    let (tanks, _warnings) = TankStore::decode(&text);
    tanks
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
    if write_file(&world.join(STORE_DIR), STORE_FILE, &text) {
        let mut guard = core();
        if let Some(state) = guard.as_mut() {
            state.dirty = false;
            state.last_save = state.ticks;
        }
    }
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
    if write_file(&world.join(STORE_DIR), TANK_FILE, &text) {
        let mut guard = core();
        if let Some(state) = guard.as_mut() {
            state.tanks_dirty = false;
        }
    }
}

/// `<dir>/<name>` written through a temporary and a rename; whether it landed. A failed write keeps
/// the store dirty, so the next tick tries again; the SDK has no logger to tell the player with.
fn write_file(dir: &Path, name: &str, text: &str) -> bool {
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    let temporary = dir.join(format!("{name}.tmp"));
    if fs::write(&temporary, text.as_bytes()).is_err() {
        return false;
    }
    fs::rename(&temporary, dir.join(name)).is_ok()
}

/// Load the store of the open world, once per world: the open phase is not a hook at this pin, so
/// the first server tick (or the first chest tick) of a world does it, and every close phase clears
/// it so the next world loads its own.
fn ensure_world(host: &mut dyn ServerHost) {
    let loaded = {
        let guard = core();
        guard.as_ref().is_some_and(|state| state.world.is_some())
    };
    if loaded {
        return;
    }
    let dir = host.world_dir();
    let Some(dir) = dir else {
        return;
    };
    let mut guard = core();
    let state = guard.get_or_insert_with(Core::default);
    if state.world.as_deref() == Some(dir.as_path()) {
        return;
    }
    state.store = load_store(&dir);
    state.tanks = load_tanks(&dir);
    state.snapshots.clear();
    state.tank_snapshots.clear();
    state.pending.clear();
    state.broken.clear();
    state.dirty = false;
    state.tanks_dirty = false;
    state.ticks = 0;
    state.last_save = 0;
    state.world = Some(dir);
}

/// Forget the open world, saving it first when asked.
fn close_world(save: bool) {
    let world = {
        let guard = core();
        guard.as_ref().and_then(|state| state.world.clone())
    };
    if save && let Some(world) = world {
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
        state.dirty = false;
        state.tanks_dirty = false;
    }
}

// ---------------------------------------------------------------------------
// Declarations
// ---------------------------------------------------------------------------

/// Declare the chest and the tank: one block entity type each, the chest's 27-slot container and
/// menu, the tank's 16-bucket tank, and the two item Parts that carry a frequency to a placed block.
pub fn declare(r: &mut dyn Registrar) {
    let chest = r.block_key(BLOCK);
    let tank = r.block_key(TANK);
    let chest_kind = r
        .block_entity_type("ender_chest")
        .blocks(&["ender_chest"])
        .build();
    let tank_kind = r
        .block_entity_type("ender_tank")
        .blocks(&["ender_tank"])
        .build();
    // The same Part answers both what the game asks the *block* (whether it ticks) and what it asks
    // the *block entity type* (each server tick).
    r.block_part(chest, Chest);
    r.block_part(tank, TankPart);
    r.block_entity_part(&chest_kind, ChestEntity)
        .expect("the chest's Part attaches to the type this mod just declared");
    r.block_entity_part(&tank_kind, TankEntity)
        .expect("the tank's Part attaches to the type this mod just declared");
    r.block_container(&chest, container());
    r.block_fluids(&tank, tanks());
    let chest_item = r.item_key(ITEM);
    r.item_part(chest_item, Placed);
    let tank_item = r.item_key(TANK_ITEM);
    r.item_part(tank_item, Placed);
}

/// 27 slots reachable from every side, shown as the vanilla 9x3 menu.
fn container() -> ModContainer {
    let all: Vec<usize> = (0..SLOTS).collect();
    ModContainer {
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
        menu: Some(MENU.to_owned()),
    }
}

/// One 16-bucket tank, reachable from every side.
fn tanks() -> ModFluidTanks {
    let mut tanks = ModFluidTanks::default();
    tanks.tanks = vec![TANK_CAPACITY];
    tanks.faces = std::array::from_fn(|_| vec![0]);
    tanks.inputs = vec![0];
    tanks.outputs = vec![0];
    tanks
}

/// Register the component that carries a frequency, before content (and so the item's stack codec)
/// loads.
pub fn declare_component(types: &mut Vec<String>) {
    types.push(COMPONENT.to_owned());
}

/// `/enderstorage freq|info`.
pub fn declare_commands(commands: &mut Vec<Command>) {
    commands.push(Command::new(
        "enderstorage freq held <left:word> <middle:word> <right:word>",
    ));
    commands.push(Command::op(
        "enderstorage freq <pos:block_pos> <left:word> <middle:word> <right:word>",
        2,
    ));
    commands.push(Command::new("enderstorage info <pos:block_pos>"));
}

/// The chest's Part: it ticks, and each server tick merges its slots with the shared store.
pub struct Chest;

#[mod_api::horizon_part]
impl BlockPart for Chest {
    /// A chest's inventory changed outside its tick: a player's menu or a hopper. Commit the change
    /// to the shared store **at once** and push the result into every other loaded chest of the
    /// frequency, so no sibling can hand out contents the store no longer holds between ticks.
    fn contents_changed(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::ContentsChangedEvent,
    ) {
        on_contents_changed(host, event.pos);
    }
}

/// The chest's block-entity Part: a **separate type** from the block Part, because the engine
/// refuses one part type attached to a block id twice and a block-entity part registers against
/// the block it belongs to.
pub struct ChestEntity;

#[mod_api::horizon_part]
impl BlockPart for ChestEntity {}

#[mod_api::horizon_part]
impl BlockEntityPart for ChestEntity {
    /// The chest ticks on the server (`EntityBlock.getTicker`).
    fn block_entity_ticks(&self, _host: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        event.ticks = true;
    }

    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        tick_chest(host, event);
    }
}

/// The tank's Part: the same, for the shared fluid.
pub struct TankPart;

#[mod_api::horizon_part]
impl BlockPart for TankPart {}

/// The tank's block-entity Part, separate from its block Part for the same reason.
pub struct TankEntity;

#[mod_api::horizon_part]
impl BlockPart for TankEntity {}

#[mod_api::horizon_part]
impl BlockEntityPart for TankEntity {
    fn block_entity_ticks(&self, _host: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        event.ticks = true;
    }

    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        tick_tank(host, event);
    }
}

/// The chest's and the tank's item Part: a placed block takes the item's frequency.
pub struct Placed;

#[mod_api::horizon_part]
impl mod_api::ItemPart for Placed {
    fn on_block_placed_by(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::OnBlockPlacedByEvent<'_>,
    ) {
        block_placed(host, event);
    }
}

// ---------------------------------------------------------------------------
// One server tick of a chest or a tank
// ---------------------------------------------------------------------------

/// One server tick of a chest: apply a queued frequency, then merge its slots with the store.
fn tick_chest(host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
    ensure_world(host);
    let pos = event.pos;
    let dimension = host.dimension().to_owned();
    let mut frequency = be_frequency(&mut event.entity.nbt);
    let uid = be_uid(&mut event.entity.nbt);
    let current = stored_slots(&*host, &event.entity.items);
    let mut out = current.clone();
    {
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
                    &current,
                    snapshot.as_deref(),
                    &mut state.store,
                    frequency,
                    wanted,
                );
                if changed {
                    state.dirty = true;
                }
                state.snapshots.insert(uid, items.clone());
                out = items;
                set_be_frequency(&mut event.entity.nbt, wanted);
                frequency = wanted;
            }
        }
        register_mirror(state, frequency, uid, &dimension, pos);
        let snapshot = state.snapshots.get(&uid).cloned();
        let synced = sync(&out, snapshot.as_deref(), &mut state.store, frequency);
        if synced.store_changed {
            state.dirty = true;
        }
        state.snapshots.insert(uid, synced.items.clone());
        out = synced.items;
    }
    if out != current {
        event.entity.items = stacks(&*host, &out);
    }
}

/// One tick of a tank: a queued frequency first, then the fluid merge with the shared tank store
/// (the declaration keeps the tank in the entity's `fluids`).
fn tick_tank(host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
    ensure_world(host);
    let pos = event.pos;
    let dimension = host.dimension().to_owned();
    let mut frequency = be_frequency(&mut event.entity.nbt);
    let uid = be_uid(&mut event.entity.nbt);
    // What the entity holds before this tick. It is the value the next tick reads, so the
    // snapshot kept below is only honest once the entity takes the merged tank.
    let held = read_be_tank(&event.entity.nbt);
    let mut current = held.clone();
    let mut switched: Option<Tank> = None;
    {
        let mut guard = core();
        let state = guard.get_or_insert_with(Core::default);
        if let Some(index) = state
            .pending
            .iter()
            .position(|(dim, at, _)| dim == &dimension && *at == pos)
        {
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
                current = tank.clone();
                switched = Some(tank);
                set_be_frequency(&mut event.entity.nbt, wanted);
                frequency = wanted;
            }
        }
        // The switched tank belongs to the new frequency and is not merged against the old
        // frequency's snapshot; it is shown as it stands.
        let snapshot = switched.as_ref().or_else(|| state.tank_snapshots.get(&uid));
        let synced = sync_tank(&current, snapshot, &mut state.tanks, frequency);
        if synced.store_changed {
            state.tanks_dirty = true;
        }
        let written = tank_write(&held, synced.tank);
        if written.write {
            write_be_tank(&mut event.entity.nbt, &written.tank);
        }
        state.tank_snapshots.insert(uid, written.tank);
    }
}

// ---------------------------------------------------------------------------
// The world's lifecycle and the server tick
// ---------------------------------------------------------------------------

/// The world closed: save the store, then forget it.
pub fn world_lifecycle(host: &mut dyn ServerHost, event: &mut WorldLifecycleEvent<'_>) {
    match event.phase {
        WorldPhase::WorldCloseFinal | WorldPhase::WorldCloseFailed => close_world(true),
        WorldPhase::WorldOpenFailed => close_world(false),
        // `WorldPhase` is non-exhaustive: an open phase this pin adds later needs no handling, as
        // `ensure_world` loads lazily.
        _ => {
            let _ = host;
        }
    }
}

/// End of a running tick: load the world on the first tick, then debounce the store writes.
pub fn server_tick(host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
    let ServerTickPhase::ServerTickEnd { world_running } = event.phase else {
        return;
    };
    if !world_running {
        return;
    }
    ensure_world(host);
    let (due, world) = {
        let mut guard = core();
        let state = guard.get_or_insert_with(Core::default);
        state.ticks += 1;
        let due = (state.dirty || state.tanks_dirty)
            && state.ticks.saturating_sub(state.last_save) >= SAVE_INTERVAL_TICKS;
        (due, state.world.clone())
    };
    if due && let Some(world) = world {
        save_store(&world);
        save_tanks(&world);
        let mut guard = core();
        if let Some(state) = guard.as_mut() {
            state.last_save = state.ticks;
        }
    }
}

/// The server is stopping: flush the store.
pub fn server_stopping(_host: &mut dyn ServerHost, _event: &mut mod_api::ServerStoppingEvent) {
    close_world(true);
}

// ---------------------------------------------------------------------------
// The blocks
// ---------------------------------------------------------------------------

/// Which of the mod's two blocks a state is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Chest,
    Tank,
}

fn block_kind(host: &dyn Host, state: BlockState) -> Option<Kind> {
    let block = state.block();
    if host.block_key(BLOCK) == Some(block) {
        Some(Kind::Chest)
    } else if host.block_key(TANK) == Some(block) {
        Some(Kind::Tank)
    } else {
        None
    }
}

/// The mod block entity at `pos`, when the block there is this mod's block.
fn our_entity(
    host: &dyn Host,
    level: LevelRef,
    pos: [i32; 3],
    block: &str,
) -> Option<BlockEntityRef> {
    let key = host.block_key(block)?;
    if level.block(host, pos) != Some(key) {
        return None;
    }
    host.block_entity(level, pos)
}

/// Commit a chest's or a tank's last changes to the store, empty its slots (so a break never spills
/// a second copy of what the store keeps), and remember its frequency for the item drop.
fn commit_and_empty(host: &mut dyn ServerHost, dimension: &str, pos: [i32; 3], kind: Kind) {
    let Some(level) = host.level(host.dimension()) else {
        return;
    };
    let block = match kind {
        Kind::Chest => BLOCK,
        Kind::Tank => TANK,
    };
    let Some(entity) = our_entity(&*host, level, pos, block) else {
        return;
    };
    let Some(mut nbt) = entity.data(&*host) else {
        return;
    };
    let frequency = read_frequency(&nbt);
    let uid = be_uid(&mut nbt);
    let mut guard = core();
    let state = guard.get_or_insert_with(Core::default);
    match kind {
        Kind::Tank => {
            let tank = read_be_tank(&nbt);
            let snapshot = state.tank_snapshots.get(&uid).cloned();
            if commit_tank(&tank, snapshot.as_ref(), &mut state.tanks, frequency) {
                state.tanks_dirty = true;
            }
            state.tank_snapshots.remove(&uid);
        }
        Kind::Chest => {
            let items = read_slots(&*host, entity);
            let current = stored_slots(&*host, &items);
            let snapshot = state.snapshots.get(&uid).cloned();
            if commit(&current, snapshot.as_deref(), &mut state.store, frequency) {
                state.dirty = true;
            }
            state.snapshots.remove(&uid);
        }
    }
    state.broken.insert((dimension.to_owned(), pos), frequency);
    if state.broken.len() > 64 {
        state.broken.clear();
    }
    drop(guard);
    // Empty the block entity before it goes: the store already keeps everything, so a slot left
    // filled would be a second copy of it. The tank's fluid lives in its saved compound, the
    // chest's items in the entity's slots (the SDK keeps the two apart).
    match kind {
        Kind::Tank => {
            let mut patch = Compound::new();
            write_be_tank(&mut patch, &Tank::EMPTY);
            let _ = entity.merge_data(host, patch);
        }
        Kind::Chest => {
            let _ = host.transaction(|host, tx| {
                for slot in 0..SLOTS {
                    let _ = entity.set_item_slot(host, tx, slot as u32, None);
                }
                Ok::<(), ()>(())
            });
        }
    }
}

/// A chest's slots as the game holds them.
fn read_slots(host: &dyn Host, entity: BlockEntityRef) -> Vec<Option<ItemStack>> {
    (0..SLOTS)
        .map(|slot| entity.item_slot(host, slot as u32).unwrap_or_default())
        .collect()
}

/// A player broke a chest or a tank: commit, empty, and keep the frequency for the drop.
pub fn break_block(host: &mut dyn ServerHost, event: &mut mod_api::BreakBlockEvent<'_>) -> Flow {
    if !event.cancelled
        && let Some(kind) = block_kind(&*host, event.state)
    {
        let dimension = host.dimension().to_owned();
        commit_and_empty(host, &dimension, event.pos, kind);
    }
    Flow::Continue
}

/// An explosion about to break chests or tanks: same commit and empty, so the store keeps the
/// contents and each block drops its frequency.
pub fn explosion_detonate(
    host: &mut dyn ServerHost,
    event: &mut mod_api::ExplosionDetonateEvent<'_>,
) {
    let dimension = event.dimension.to_owned();
    let Some(level) = host.level(host.dimension()) else {
        return;
    };
    let found: Vec<([i32; 3], Kind)> = event
        .blocks
        .iter()
        .filter_map(|pos| {
            let state = level.block_state(&*host, *pos)?;
            block_kind(&*host, state).map(|kind| (*pos, kind))
        })
        .collect();
    for (pos, kind) in found {
        commit_and_empty(host, &dimension, pos, kind);
    }
}

/// The broken chest's or tank's item drop carries its frequency.
pub fn loot_drops(host: &mut dyn ServerHost, event: &mut mod_api::LootDropsEvent<'_>) {
    let Some((pos, state)) = event.context.block else {
        return;
    };
    let Some(kind) = block_kind(&*host, state) else {
        return;
    };
    let dimension = event.context.dimension.to_owned();
    let block_id = match kind {
        Kind::Chest => BLOCK,
        Kind::Tank => TANK,
    };
    let item_id = match kind {
        Kind::Chest => ITEM,
        Kind::Tank => TANK_ITEM,
    };
    let frequency = {
        let mut guard = core();
        guard
            .get_or_insert_with(Core::default)
            .broken
            .remove(&(dimension, pos))
    }
    .or_else(|| {
        let level = host.level(host.dimension())?;
        let entity = our_entity(&*host, level, pos, block_id)?;
        Some(read_frequency(&entity.data(&*host)?))
    });
    let Some(frequency) = frequency else {
        return;
    };
    let Some(key) = Identifier::parse(item_id)
        .ok()
        .and_then(|id| host.item_key(&id))
    else {
        return;
    };
    let mut stack = ItemStack::of(key);
    if !write_stack_frequency(&mut stack, frequency) {
        return;
    }
    // The content's loot table drops the bare item on a build where it loads; replacing that drop
    // rather than adding to it keeps one chest per broken block.
    let name = stack.item().id(&*host);
    event.drops.retain(|drop| drop.item().id(&*host) != name);
    event.drops.push(stack);
}

/// A chest or tank placed from an item takes that item's frequency; the block entity's first tick
/// applies it.
pub fn block_placed(host: &mut dyn ServerHost, event: &mut mod_api::OnBlockPlacedByEvent<'_>) {
    if block_kind(&*host, event.state).is_none() {
        return;
    }
    let Some(frequency) = stack_frequency(event.stack) else {
        return;
    };
    queue_frequency(host, event.pos, frequency);
}

/// Queue a frequency for the block at `pos` (placement, or a command).
fn queue_frequency(host: &mut dyn ServerHost, pos: [i32; 3], frequency: Frequency) {
    let dimension = host.dimension().to_owned();
    let mut guard = core();
    let state = guard.get_or_insert_with(Core::default);
    state
        .pending
        .retain(|(dim, at, _)| !(dim == &dimension && *at == pos));
    state.pending.push((dimension, pos, frequency));
}

// ---------------------------------------------------------------------------
// Clicks: the dye band, and the bucket
// ---------------------------------------------------------------------------

/// A right click with a dye sets the chest's next colour band and consumes one dye; a right click
/// with a bucket belongs to the tank. Both claim the click, so no menu opens and no fluid is poured
/// into the world. The bucket's exchange is not done here: the bucket's own use (`fill_bucket`)
/// runs one phase later, and swapping the hand in both places produced two buckets per click.
pub fn right_click(host: HostSide<'_>, event: &mut mod_api::RightClickBlockEvent<'_>) -> Flow {
    if event.cancelled || event.click.hand != Hand::Main {
        return Flow::Continue;
    }
    let click = event.click;
    let pos = click.pos;
    let Some(held) = click.stack.cloned() else {
        return Flow::Continue;
    };
    let HostSide::Server(server) = host else {
        return Flow::Continue;
    };
    let Some(player) = server.player(click.player) else {
        return Flow::Continue;
    };
    let Some(level) = server.level(server.dimension()) else {
        return Flow::Continue;
    };
    let here = level
        .block_state(&*server, pos)
        .and_then(|state| block_kind(&*server, state));
    let claimed = match here {
        // The bucket's own use (`fill_bucket`) swaps it; this click only claims the tank so no
        // menu opens and nothing is poured.
        Some(Kind::Tank) => bucket_click(&*server, &held),
        Some(Kind::Chest) => dye_click(server, player, level, pos, &held),
        None => false,
    };
    if claimed {
        *event.cancellation_result = InteractionResult::Success;
        Flow::Cancel
    } else {
        Flow::Continue
    }
}

/// A server player uses a bucket at the tank: an empty bucket draws one whole bucket out of the
/// shared tank and a full bucket puts one in, the held stack gives up (or gains) exactly one item,
/// and nothing is poured beside the block. The swap lives here, in the click's item phase, so the
/// engine's own one item use cannot repeat it. The hook carries the fluid target, not the clicked
/// face, so the tank is reached through the `up` side (its table takes every side).
pub fn fill_bucket(host: &mut dyn ServerHost, event: &mut mod_api::FillBucketEvent<'_>) -> Flow {
    if event.cancelled {
        return Flow::Continue;
    }
    let pos = event.pos;
    let Some(level) = host.level(host.dimension()) else {
        return Flow::Continue;
    };
    let ours = level
        .block_state(&*host, pos)
        .and_then(|state| block_kind(&*host, state))
        == Some(Kind::Tank);
    if !ours {
        return Flow::Continue;
    }
    let fluid = bucket_fluid(&*host, event.stack);
    let empty = event.stack.item().id(&*host) == BUCKET_ITEM;
    let player = host.players().acting_player();
    if let Some(player) = player {
        // The fluid and the hand exchange are one change: a failed bucket swap aborts the
        // transaction, so a tank is never mutated without the player's bucket changing with it.
        let _ = host.transaction(|host, tx| {
            let done = if let Some(fluid) = fluid {
                event.stack.item().id(&*host) == fluid_bucket(fluid).unwrap_or_default()
                    && fill_tank(host, tx, level, pos, fluid, player)
            } else if empty {
                drain_tank(host, tx, level, pos, None, player)
            } else {
                false
            };
            if done { Ok::<bool, ()>(true) } else { Err(()) }
        });
    }
    // The tank owns the pour either way, so a full tank never empties the bucket beside the block.
    Flow::Cancel
}

/// The fluid a filled bucket holds.
fn bucket_fluid(host: &dyn Host, stack: &ItemStack) -> Option<&'static str> {
    match stack.item().id(host).as_str() {
        "minecraft:water_bucket" => Some("minecraft:water"),
        "minecraft:lava_bucket" => Some("minecraft:lava"),
        _ => None,
    }
}

/// The bucket a fluid fills.
fn fluid_bucket(fluid: &str) -> Option<&'static str> {
    match fluid {
        "minecraft:water" => Some("minecraft:water_bucket"),
        "minecraft:lava" => Some("minecraft:lava_bucket"),
        _ => None,
    }
}

/// Whether a right click on a tank with `held` is the tank's: a bucket of either kind is.
fn bucket_click(host: &dyn Host, held: &ItemStack) -> bool {
    held.item().id(host) == BUCKET_ITEM || bucket_fluid(host, held).is_some()
}

/// Put one bucket of `fluid` into the tank at `pos` and turn one held bucket into an empty one;
/// whether it went in. At this pin the declared tank keeps no insert, so the caller's transaction
/// rolls the swap back with the missing fluid; the recorded gap is in `notes.md`.
fn fill_tank(
    host: &mut dyn ServerHost,
    tx: &Transaction,
    level: LevelRef,
    pos: [i32; 3],
    fluid: &'static str,
    player: PlayerRef,
) -> bool {
    let Some(key) = host.fluid_key(fluid) else {
        return false;
    };
    let Some(tanks) = level.capability(host, &FLUIDS, pos, Some(mod_api::Direction::Up)) else {
        return false;
    };
    let resource = FluidResource::of(&FluidStack::new(key, 1));
    let want = bucket_volume();
    // The tank holds one fluid at a time, and a bucket needs room for all of it: a part-filled or
    // foreign tank leaves the bucket alone rather than creating or losing fluid.
    if let Some(current) = tanks.resource(host, 0)
        && !current.matches(&resource.to_stack(1))
    {
        return false;
    }
    let held = tanks.amount(host, 0);
    if tanks.capacity(host, 0, &resource).saturating_sub(held) < want {
        return false;
    }
    let inserted = tanks.insert(host, tx, 0, &resource, want);
    if inserted < want {
        if inserted > 0 {
            let _ = tanks.extract(host, tx, 0, &resource, inserted);
        }
        return false;
    }
    // The bucket swap is part of the same change: if the player's hand does not take the empty
    // bucket, the fill is rolled back by the caller instead of leaving fluid behind for free.
    match fluid_bucket(fluid) {
        Some(bucket) => swap_held(host, player, bucket, BUCKET_ITEM),
        None => false,
    }
}

/// Take one whole bucket out of the tank at `pos` into the held empty bucket — the stack keeps its
/// other items (`runtime::bucket_swap`); whether the draw came out. A partial draw is put back,
/// because an empty bucket holds no fraction.
fn drain_tank(
    host: &mut dyn ServerHost,
    tx: &Transaction,
    level: LevelRef,
    pos: [i32; 3],
    face: Option<mod_api::Direction>,
    player: PlayerRef,
) -> bool {
    let Some(tanks) = level.capability(host, &FLUIDS, pos, face) else {
        return false;
    };
    let Some(resource) = tanks.resource(host, 0) else {
        return false;
    };
    let want = bucket_volume();
    if tanks.amount(host, 0) < want {
        return false;
    }
    let taken = tanks.extract(host, tx, 0, &resource, want);
    if taken < want {
        if taken > 0 {
            let _ = tanks.insert(host, tx, 0, &resource, taken);
        }
        return false;
    }
    let name = match resource.fluid() {
        FluidIdentity::Resolved(key) => key.id(&*host),
        FluidIdentity::Unresolved(fluid) => fluid.id(&*host),
        // `FluidIdentity` is non-exhaustive: a fluid this build cannot name is not a bucket.
        _ => String::new(),
    };
    match fluid_bucket(&name) {
        Some(bucket) => swap_held(host, player, BUCKET_ITEM, bucket),
        None => false,
    }
}

/// One bucket, in millibuckets.
fn bucket_volume() -> u32 {
    u32::try_from(BUCKET).unwrap_or(1_000)
}

/// Replace one held `from` with one `to` (a bucket swap) in the hand that holds it, and answer
/// whether the write succeeded.
///
/// The hand is read from the inventory, not assumed: the selected hotbar slot first, then the off
/// hand (`Inventory`'s 40), then any other slot still holding the item. `fill_bucket` has no hand
/// in its event, so an off-hand fill used to edit the main hand — leaving the off-hand bucket
/// unchanged and the main-hand item destroyed. The swap fails (rather than silently doing
/// nothing) when the item is not there or the write is refused, so the caller rolls the fluid
/// change back.
///
/// A stack gives up **one** item: eight empty buckets drained at a tank leave seven empty buckets
/// and one full bucket in the inventory (`runtime::bucket_swap`), as vanilla's
/// `ItemUtils.createFilledResult` fills one. Only the last item lets the produced bucket take the
/// hand's slot; a stack that merely replaced its whole slot lost the other seven buckets.
fn swap_held(host: &mut dyn ServerHost, player: PlayerRef, from: &str, to: &str) -> bool {
    let Some((slot, stack)) = holding_stack(host, player, from) else {
        return false;
    };
    let Some(swap) = bucket_swap(stack.count()) else {
        return false;
    };
    let Some(key) = Identifier::parse(to).ok().and_then(|id| host.item_key(&id)) else {
        return false;
    };
    if swap.into_hand {
        let Some(produced) = ItemStack::new(key, 1) else {
            return false;
        };
        return player
            .set_inventory_slot(host, slot, Some(produced))
            .is_ok();
    }
    let mut rest = stack.clone();
    if rest.set_count(swap.kept).is_err() {
        return false;
    }
    if player.set_inventory_slot(host, slot, Some(rest)).is_err() {
        return false;
    }
    let Some(produced) = ItemStack::new(key, 1) else {
        return false;
    };
    if player.give_stack(host, produced).is_err() {
        // The hand is put back so the caller's transaction leaves it as it was: no fluid for a
        // bucket that was never produced.
        let _ = player.set_inventory_slot(host, slot, Some(stack));
        return false;
    }
    true
}

/// The slot and the stack holding `item`: the selected slot, then the off hand, then any other
/// slot.
fn holding_stack(host: &dyn ServerHost, player: PlayerRef, item: &str) -> Option<(u32, ItemStack)> {
    let selected = u32::try_from(player.selected_slot(host)?).ok();
    let slots = player.inventory_slots(host)?;
    let holds = |slot: u32| {
        slots
            .iter()
            .find(|(at, stack)| *at == slot && stack.item().id(host) == item)
            .map(|(_, stack)| stack.clone())
    };
    if let Some(slot) = selected
        && let Some(stack) = holds(slot)
    {
        return Some((slot, stack));
    }
    if let Some(stack) = holds(OFF_HAND) {
        return Some((OFF_HAND, stack));
    }
    slots
        .iter()
        .find(|(_, stack)| stack.item().id(host) == item)
        .map(|(slot, stack)| (*slot, stack.clone()))
}

/// The player's main-hand stack and the slot it is in.
fn held(host: &dyn ServerHost, player: PlayerRef) -> Option<(u32, ItemStack)> {
    let slot = u32::try_from(player.selected_slot(host)?).ok()?;
    player
        .inventory_slots(host)?
        .into_iter()
        .find(|(at, _)| *at == slot)
        .map(|(_, stack)| (slot, stack))
}

/// A dye click on a chest: commit what it held to the old frequency, take the new one, set the next
/// band, consume one dye and show the colours. Whether the click is the chest's.
fn dye_click(
    host: &mut dyn ServerHost,
    player: PlayerRef,
    level: LevelRef,
    pos: [i32; 3],
    held_stack: &ItemStack,
) -> bool {
    let Some(colour) = Colour::from_dye(&held_stack.item().id(&*host)) else {
        return false;
    };
    let Some(entity) = our_entity(&*host, level, pos, BLOCK) else {
        return false;
    };
    let Some(data) = entity.data(&*host) else {
        return false;
    };
    let mut nbt = data;
    let frequency = be_frequency(&mut nbt);
    let band = be_band(&nbt);
    let previous = frequency.band(band);
    let updated = frequency.with(band, colour);
    let uid = be_uid(&mut nbt);
    let items = read_slots(&*host, entity);
    let current = stored_slots(&*host, &items);
    let next = {
        let mut guard = core();
        let state = guard.get_or_insert_with(Core::default);
        let snapshot = state.snapshots.get(&uid).cloned();
        let (next, changed) = switch(
            &current,
            snapshot.as_deref(),
            &mut state.store,
            frequency,
            updated,
        );
        if changed {
            state.dirty = true;
        }
        next
    };
    let stacks_back = stacks(&*host, &next);
    let mut patch = Compound::new();
    set_be_frequency(&mut patch, updated);
    set_be_band(&mut patch, band.next());
    patch.insert("uid".into(), Tag::Long(uid as i64));
    let written = host
        .transaction(|host, tx| -> Result<bool, ()> {
            for (slot, stack) in stacks_back.iter().enumerate() {
                if stack.is_some() || items.get(slot).is_some_and(Option::is_some) {
                    let _ = entity.set_item_slot(host, tx, slot as u32, stack.clone());
                }
            }
            Ok(entity.merge_data(host, patch.clone()).unwrap_or(false))
        })
        .unwrap_or(false);
    if written {
        // The chest now shows the new frequency's slots, so its snapshot moves with it; the next
        // tick's merge then starts from what the chest really holds.
        {
            let mut guard = core();
            if let Some(state) = guard.as_mut() {
                state.snapshots.insert(uid, next);
            }
        }
        let _ = entity.set_changed(host);
        consume_held(host, player);
        let line = format!(
            "EnderStorage: {} ({} {} -> {}; next click sets {})",
            updated.label(),
            band.name(),
            previous.name(),
            colour.name(),
            band.next().name()
        );
        let _ = player.action_bar(host, Text::literal(line));
    }
    true
}

/// Take one of the held dyes (a creative player keeps them).
fn consume_held(host: &mut dyn ServerHost, player: PlayerRef) {
    if player.game_mode(host) == Some(GameMode::Creative) {
        return;
    }
    let Some((slot, stack)) = held(host, player) else {
        return;
    };
    match stack.shrink(1) {
        Some(rest) => {
            let _ = player.set_inventory_slot(host, slot, Some(rest));
        }
        None => {
            let _ = player.set_inventory_slot(host, slot, None);
        }
    }
}

// ---------------------------------------------------------------------------
// The commands
// ---------------------------------------------------------------------------

/// `/enderstorage freq held <left> <middle> <right>`, `/enderstorage freq <pos> <l> <m> <r>` and
/// `/enderstorage info <pos>`. The position form queues the change for the chest's next tick, as
/// the block entity is not in the world here.
pub fn run_command(host: &mut dyn ServerHost, event: &mut RunModCommandEvent<'_>) -> Flow {
    let invocation = event.command;
    let Some(root) = invocation.literals.first().map(String::as_str) else {
        return Flow::Continue;
    };
    if root != "enderstorage" {
        return Flow::Continue;
    }
    let sub: Vec<&str> = invocation.literals.iter().map(String::as_str).collect();
    match sub.as_slice() {
        ["enderstorage", "freq", "held"] => {
            let (Some(left), Some(middle), Some(right)) = (
                word(invocation, "left"),
                word(invocation, "middle"),
                word(invocation, "right"),
            ) else {
                say(
                    host,
                    "usage: /enderstorage freq held <left> <middle> <right>",
                );
                return Flow::Cancel;
            };
            let Some(frequency) = parse_colours(left, middle, right) else {
                say(
                    host,
                    "Unknown colour: use a wool colour name (white, red, light_blue...).",
                );
                return Flow::Cancel;
            };
            let Some(player) = source_player(event.source) else {
                say(
                    host,
                    "Hold an Ender Chest or Ender Tank to set its frequency.",
                );
                return Flow::Cancel;
            };
            let Some((slot, mut stack)) = held(host, player) else {
                say(
                    host,
                    "Hold an Ender Chest or Ender Tank to set its frequency.",
                );
                return Flow::Cancel;
            };
            let id = stack.item().id(&*host);
            if id != ITEM && id != TANK_ITEM {
                say(
                    host,
                    "Hold an Ender Chest or Ender Tank to set its frequency.",
                );
                return Flow::Cancel;
            }
            if !write_stack_frequency(&mut stack, frequency) {
                say(host, "Could not set the frequency.");
                return Flow::Cancel;
            }
            let _ = player.set_inventory_slot(host, slot, Some(stack));
            say(
                host,
                &format!("Ender Chest frequency: {}", frequency.label()),
            );
        }
        ["enderstorage", "freq"] => {
            let Some(pos) = block_pos(invocation, "pos") else {
                say(
                    host,
                    "Usage: /enderstorage freq <x> <y> <z> <left> <middle> <right>",
                );
                return Flow::Cancel;
            };
            let (Some(left), Some(middle), Some(right)) = (
                word(invocation, "left"),
                word(invocation, "middle"),
                word(invocation, "right"),
            ) else {
                say(
                    host,
                    "Usage: /enderstorage freq <x> <y> <z> <left> <middle> <right>",
                );
                return Flow::Cancel;
            };
            let Some(frequency) = parse_colours(left, middle, right) else {
                say(
                    host,
                    "Unknown colour: use a wool colour name (white, red, light_blue...).",
                );
                return Flow::Cancel;
            };
            let Some(level) = host.level(host.dimension()) else {
                say(host, "No Ender Chest or Ender Tank there.");
                return Flow::Cancel;
            };
            let here = level
                .block_state(&*host, pos)
                .and_then(|state| block_kind(&*host, state));
            if here.is_none() {
                say(host, "No Ender Chest or Ender Tank there.");
                return Flow::Cancel;
            }
            queue_frequency(host, pos, frequency);
            say(
                host,
                &format!("Ender Chest frequency: {}", frequency.label()),
            );
        }
        ["enderstorage", "info"] => {
            let Some(pos) = block_pos(invocation, "pos") else {
                say(host, "Usage: /enderstorage info <x> <y> <z>");
                return Flow::Cancel;
            };
            // The readout names the block's frequency and its occupancy. The frequency is enough
            // to join a public linked store, so it is not a remote read for an ordinary player:
            // an operator reads any block, a player only one within reach, and any other
            // non-operator source nothing.
            let (operator, reader) = reader(&*host, event.source);
            if !may_read(operator, reader, pos) {
                say(host, "That block is out of reach.");
                return Flow::Cancel;
            }
            let readout = readout(host, pos);
            say(host, &readout);
        }
        _ => {
            say(
                host,
                "usage: /enderstorage freq held|freq <pos>|info <pos> ...",
            );
        }
    }
    Flow::Cancel
}

/// The frequency three colour names stand for.
fn parse_colours(left: &str, middle: &str, right: &str) -> Option<Frequency> {
    Some(Frequency::new(
        Colour::from_name(left)?,
        Colour::from_name(middle)?,
        Colour::from_name(right)?,
    ))
}

/// The `left`/`middle`/`right` word argument.
fn word<'a>(invocation: &'a CommandInvocation, name: &str) -> Option<&'a str> {
    match invocation.argument(name)? {
        CommandArgumentValue::String(word) => Some(word.as_str()),
        _ => None,
    }
}

/// The `pos` block-position argument.
fn block_pos(invocation: &CommandInvocation, name: &str) -> Option<[i32; 3]> {
    match invocation.argument(name)? {
        CommandArgumentValue::BlockPos(pos) => Some(*pos),
        _ => None,
    }
}

/// The source's right to read a block: whether its permission reaches the operator level (2, the
/// level the frequency-changing command requires) and the player's position when it is a player.
fn reader(host: &dyn ServerHost, source: &CommandSource) -> (bool, Option<[f64; 3]>) {
    let operator = source.permission().level() >= 2;
    let player = match source {
        CommandSource::Player { player, .. } => player.entity().position(host),
        _ => None,
    };
    (operator, player)
}

/// The player a command came from.
fn source_player(source: &CommandSource) -> Option<PlayerRef> {
    match source {
        CommandSource::Player { player, .. } => Some(*player),
        _ => None,
    }
}

/// One feedback line to the command's source, vanilla's silence and track-output rules included.
fn say(host: &mut dyn ServerHost, line: &str) {
    let _ = host.commands().feedback(Text::literal(line));
}

/// The chest's or the tank's state, as `/enderstorage info` prints it.
fn readout(host: &mut dyn ServerHost, pos: [i32; 3]) -> String {
    let Some(level) = host.level(host.dimension()) else {
        return "No Ender Chest or Ender Tank there.".to_owned();
    };
    let Some(entity) =
        our_entity(&*host, level, pos, BLOCK).or_else(|| our_entity(&*host, level, pos, TANK))
    else {
        return "No Ender Chest or Ender Tank there.".to_owned();
    };
    let Some(data) = entity.data(&*host) else {
        return "No Ender Chest or Ender Tank there.".to_owned();
    };
    let frequency = read_frequency(&data);
    if level.block(&*host, pos) == host.block_key(TANK) {
        let tank = read_be_tank(&data);
        let stored = {
            let guard = core();
            guard
                .as_ref()
                .map_or(Tank::EMPTY, |state| state.tanks.tank(frequency))
        };
        return format!(
            "Ender Tank: {} ({} {} mB, {} mB in the store)",
            frequency.label(),
            tank.fluid.as_deref().unwrap_or("empty"),
            tank.amount,
            stored.amount
        );
    }
    let items = read_slots(&*host, entity);
    let used = items.iter().filter(|slot| slot.is_some()).count();
    let stored = {
        let guard = core();
        guard
            .as_ref()
            .map_or(0, |state| state.store.count(frequency))
    };
    format!(
        "Ender Chest: {} ({used} slots used, {stored} in the store)",
        frequency.label()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A saved item compound in the shape `ItemStack.CODEC` writes.
    fn item(id: &str, count: i32) -> Compound {
        let mut nbt = Compound::new();
        nbt.insert("count".into(), Tag::Int(count));
        nbt.insert("id".into(), Tag::String(id.into()));
        nbt
    }

    fn stored(id: &str, count: u32) -> StoredItem {
        StoredItem {
            id: id.to_owned(),
            count,
            nbt: item(id, i32::try_from(count).unwrap_or(1)),
        }
    }

    #[test]
    fn a_stored_item_survives_the_store_file() {
        let item = stored("minecraft:diamond", 5);
        let text = item.encode();
        assert!(
            text.starts_with("hex:"),
            "the slot text is the port's own: {text}"
        );
        assert_eq!(StoredItem::decode(&text), Some(item));
    }

    #[test]
    fn a_stored_item_keeps_its_components_and_damage() {
        let mut patch = Compound::new();
        patch.insert("minecraft:damage".into(), Tag::Int(17));
        let mut nested = Compound::new();
        nested.insert("levels".into(), Tag::Compound(Compound::new()));
        patch.insert(
            "minecraft:stored_enchantments".into(),
            Tag::Compound(nested),
        );
        let mut nbt = item("minecraft:enchanted_book", 1);
        nbt.insert("components".into(), Tag::Compound(patch));
        let item = StoredItem {
            id: "minecraft:enchanted_book".to_owned(),
            count: 1,
            nbt,
        };
        assert_eq!(StoredItem::decode(&item.encode()), Some(item));
    }

    #[test]
    fn a_slot_text_that_is_not_ours_reads_empty() {
        assert_eq!(StoredItem::decode("hex:"), None);
        assert_eq!(StoredItem::decode("hex:zz"), None);
        assert_eq!(StoredItem::decode("hex:0"), None);
        assert_eq!(StoredItem::decode("{id:\"minecraft:stone\"}"), None);
    }

    #[test]
    fn a_stored_item_is_keyed_by_its_id_and_sized_by_its_count() {
        let item = stored("minecraft:diamond", 5);
        assert_eq!(item.key(), "minecraft:diamond");
        assert_eq!(item.count(), 5);
        let more = item.with_count(12).expect("12 is a stack");
        assert_eq!((more.count(), more.key()), (12, "minecraft:diamond"));
        assert_eq!(more.nbt.get(&"count".into()), Some(&Tag::Int(12)));
        assert_eq!(
            item.with_count(0),
            None,
            "an empty slot is None, not a 0-count item"
        );
    }

    #[test]
    fn the_store_file_round_trips_a_whole_frequency() {
        let mut store: Store<StoredItem> = Store::new();
        let mut slots = vec![None; SLOTS];
        slots[0] = Some(stored("minecraft:diamond", 5));
        slots[26] = Some(stored("minecraft:emerald", 64));
        let frequency = Frequency::new(Colour::Blue, Colour::Blue, Colour::Blue);
        store.set(frequency, slots.clone());
        let text = store.encode();
        assert!(text.contains("f blue,blue,blue"), "{text}");
        let (back, warnings) = Store::<StoredItem>::decode(&text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back.slots(frequency), slots);
    }

    #[test]
    fn the_chest_declares_the_vanilla_nine_by_three_menu() {
        let container = container();
        assert_eq!(container.slots, SLOTS);
        assert_eq!(container.slots, 27);
        for face in &container.faces {
            assert_eq!(
                face,
                &(0..SLOTS).collect::<Vec<_>>(),
                "every side reaches every slot"
            );
        }
        assert_eq!(container.inputs.len(), SLOTS);
        assert_eq!(container.outputs.len(), SLOTS);
        assert_eq!(container.menu.as_deref(), Some("minecraft:generic_9x3"));
    }

    #[test]
    fn the_tank_declares_one_sixteen_bucket_tank_on_every_side() {
        let tanks = tanks();
        assert_eq!(tanks.tanks, vec![TANK_CAPACITY]);
        assert_eq!(TANK_CAPACITY, 16_000);
        for face in &tanks.faces {
            assert_eq!(face, &vec![0]);
        }
        assert_eq!(tanks.inputs, vec![0]);
        assert_eq!(tanks.outputs, vec![0]);
    }

    #[test]
    fn a_chest_keeps_its_frequency_and_the_default_is_written_once() {
        let mut nbt = Compound::new();
        assert_eq!(be_frequency(&mut nbt), Frequency::DEFAULT);
        assert_eq!(
            nbt.get(&"frequency".into()),
            Some(&Tag::String("white,white,white".into())),
            "the chest keeps the default it started from"
        );
        let red = Frequency::new(Colour::Red, Colour::White, Colour::White);
        set_be_frequency(&mut nbt, red);
        assert_eq!(be_frequency(&mut nbt), red);
        assert_eq!(read_frequency(&nbt), red);
        assert_eq!(
            read_frequency(&Compound::new()),
            Frequency::DEFAULT,
            "an unread chest reads the default"
        );
    }

    #[test]
    fn a_chest_keeps_one_uid_across_calls() {
        let mut nbt = Compound::new();
        let first = be_uid(&mut nbt);
        assert_eq!(be_uid(&mut nbt), first, "a chest keeps the uid it made");
        assert_ne!(be_uid(&mut Compound::new()), first, "two new chests differ");
    }

    #[test]
    fn the_dye_click_band_cycles_left_middle_right() {
        let mut nbt = Compound::new();
        assert_eq!(
            be_band(&nbt),
            Band::Left,
            "a new chest starts at the left band"
        );
        for band in [Band::Left, Band::Middle, Band::Right] {
            set_be_band(&mut nbt, band.next());
            assert_eq!(be_band(&nbt), band.next());
        }
        assert_eq!(Band::Right.next(), Band::Left);
    }

    #[test]
    fn the_tank_round_trips_through_the_saved_fluids() {
        let mut nbt = Compound::new();
        assert_eq!(read_be_tank(&nbt), Tank::EMPTY);
        let water = Tank::new("minecraft:water", 4_000);
        write_be_tank(&mut nbt, &water);
        assert_eq!(read_be_tank(&nbt), water);
        write_be_tank(&mut nbt, &Tank::EMPTY);
        assert_eq!(read_be_tank(&nbt), Tank::EMPTY);
        assert_eq!(
            nbt.get(&"fluids".into()),
            Some(&Tag::List(vec![Tag::Compound({
                let mut entry = Compound::new();
                entry.insert("Amount".into(), Tag::Int(0));
                entry.insert("Fluid".into(), Tag::String("minecraft:empty".into()));
                entry
            })])),
            "the declaration's shape is kept, one entry per tank"
        );
    }

    #[test]
    fn a_bucket_is_a_thousand_millibuckets_both_ways() {
        assert_eq!(bucket_volume(), 1_000);
        assert_eq!(
            fluid_bucket("minecraft:water"),
            Some("minecraft:water_bucket")
        );
        assert_eq!(
            fluid_bucket("minecraft:lava"),
            Some("minecraft:lava_bucket")
        );
        assert_eq!(fluid_bucket("minecraft:milk"), None);
    }

    #[test]
    fn the_commands_are_the_three_this_port_documents() {
        let mut commands: Vec<Command> = Vec::new();
        declare_commands(&mut commands);
        let usages: Vec<&str> = commands
            .iter()
            .map(|command| command.usage.as_str())
            .collect();
        assert_eq!(
            usages,
            vec![
                "enderstorage freq held <left:word> <middle:word> <right:word>",
                "enderstorage freq <pos:block_pos> <left:word> <middle:word> <right:word>",
                "enderstorage info <pos:block_pos>",
            ]
        );
        assert_eq!(
            commands[1].level, 2,
            "re-colouring a block a player cannot reach is an op command"
        );
        assert_eq!(commands[0].level, 0);
    }

    #[test]
    fn the_frequency_component_is_the_ports_own_id() {
        let mut types: Vec<String> = Vec::new();
        declare_component(&mut types);
        assert_eq!(types, vec!["enderstorage:frequency".to_owned()]);
        assert_eq!(FrequencyTag.id(), COMPONENT);
    }
}

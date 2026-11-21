// EnderStorage for Horizon — the frequency store and its merge (pure).
//
// Ported from EnderStorage 2.13.0 (TheCBProject/EnderStorage, MIT) for
// Minecraft Java 1.21.1 NeoForge. Upstream source read for behaviour only;
// this is a clean-room Rust rewrite.
//
// Upstream keeps one `EnderItemStorage` per `Frequency` (three wool colours)
// in a world-global manager, and every chest with that frequency holds the
// same `Container`. Horizon gives a mod block entity its own item slots, so
// this port keeps the frequency store here (the truth) and mirrors it into
// each chest's slots once per block-entity tick. A per-chest snapshot makes
// the mirror a three-way merge: local edits (a player, a hopper) are written
// to the store, remote edits (another chest, another dimension) are adopted,
// and a slot changed on both sides keeps the local value.
//
// Slice 1 is the ender chest. The tank, the pouch, personal (owned)
// frequencies, the lid animation and the custom renderer are deferred; see
// ../notes.md.

use std::collections::BTreeMap;

/// The chest's slots: 3x9, upstream's default `item_storage_size = 1`.
pub const SLOTS: usize = 27;

/// Upstream's 16 wool colours, in vanilla's dye order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Colour {
    White,
    Orange,
    Magenta,
    LightBlue,
    Yellow,
    Lime,
    Pink,
    Gray,
    LightGray,
    Cyan,
    Purple,
    Blue,
    Brown,
    Green,
    Red,
    Black,
}

impl Colour {
    pub const ALL: [Colour; 16] = [
        Colour::White,
        Colour::Orange,
        Colour::Magenta,
        Colour::LightBlue,
        Colour::Yellow,
        Colour::Lime,
        Colour::Pink,
        Colour::Gray,
        Colour::LightGray,
        Colour::Cyan,
        Colour::Purple,
        Colour::Blue,
        Colour::Brown,
        Colour::Green,
        Colour::Red,
        Colour::Black,
    ];

    /// The name upstream's `EnumColour` serialises as (`light_blue`).
    pub fn name(self) -> &'static str {
        match self {
            Colour::White => "white",
            Colour::Orange => "orange",
            Colour::Magenta => "magenta",
            Colour::LightBlue => "light_blue",
            Colour::Yellow => "yellow",
            Colour::Lime => "lime",
            Colour::Pink => "pink",
            Colour::Gray => "gray",
            Colour::LightGray => "light_gray",
            Colour::Cyan => "cyan",
            Colour::Purple => "purple",
            Colour::Blue => "blue",
            Colour::Brown => "brown",
            Colour::Green => "green",
            Colour::Red => "red",
            Colour::Black => "black",
        }
    }

    /// A colour by its name, case-insensitively; none for another word.
    pub fn from_name(name: &str) -> Option<Colour> {
        let normal = name.trim().to_ascii_lowercase().replace(' ', "_");
        Colour::ALL
            .into_iter()
            .find(|colour| colour.name() == normal)
    }

    /// The colour of a dye item (`minecraft:red_dye`); none for another item.
    pub fn from_dye(item: &str) -> Option<Colour> {
        let path = item.strip_prefix("minecraft:")?;
        Colour::from_name(path.strip_suffix("_dye")?)
    }
}

/// A frequency: the three wool colours, left to right.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Frequency {
    pub left: Colour,
    pub middle: Colour,
    pub right: Colour,
}

impl Frequency {
    /// `new Frequency()` upstream: white on white on white.
    pub const DEFAULT: Frequency = Frequency {
        left: Colour::White,
        middle: Colour::White,
        right: Colour::White,
    };

    pub fn new(left: Colour, middle: Colour, right: Colour) -> Self {
        Frequency {
            left,
            middle,
            right,
        }
    }

    /// Parse the store's/component's `left,middle,right` text.
    pub fn parse(text: &str) -> Option<Frequency> {
        let mut parts = text.split(',');
        let left = Colour::from_name(parts.next()?)?;
        let middle = Colour::from_name(parts.next()?)?;
        let right = Colour::from_name(parts.next()?)?;
        if parts.next().is_some() {
            return None;
        }
        Some(Frequency {
            left,
            middle,
            right,
        })
    }

    /// The `left,middle,right` text the store file and the item component use.
    pub fn text(self) -> String {
        format!(
            "{},{},{}",
            self.left.name(),
            self.middle.name(),
            self.right.name()
        )
    }

    /// A player-facing label (`red / white / blue`).
    pub fn label(self) -> String {
        format!(
            "{} / {} / {}",
            self.left.name(),
            self.middle.name(),
            self.right.name()
        )
    }

    pub fn band(self, band: Band) -> Colour {
        match band {
            Band::Left => self.left,
            Band::Middle => self.middle,
            Band::Right => self.right,
        }
    }

    pub fn with(self, band: Band, colour: Colour) -> Frequency {
        match band {
            Band::Left => Frequency {
                left: colour,
                ..self
            },
            Band::Middle => Frequency {
                middle: colour,
                ..self
            },
            Band::Right => Frequency {
                right: colour,
                ..self
            },
        }
    }
}

/// One of the three colour bands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Left,
    Middle,
    Right,
}

impl Band {
    /// The band a dye click sets next (left, middle, right, left...).
    pub fn next(self) -> Band {
        match self {
            Band::Left => Band::Middle,
            Band::Middle => Band::Right,
            Band::Right => Band::Left,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Band::Left => "left",
            Band::Middle => "middle",
            Band::Right => "right",
        }
    }

    pub fn index(self) -> u8 {
        match self {
            Band::Left => 0,
            Band::Middle => 1,
            Band::Right => 2,
        }
    }

    pub fn from_index(index: u8) -> Band {
        match index % 3 {
            1 => Band::Middle,
            2 => Band::Right,
            _ => Band::Left,
        }
    }
}

/// One item slot as the store persists it. The glue implements this for the
/// game's item stack (SNBT text); the tests use a string.
pub trait Slot: Clone + PartialEq {
    fn encode(&self) -> String;
    fn decode(text: &str) -> Option<Self>;
    /// The item this stack is, ignoring its count: two stacks with the same key
    /// are the same item, so their counts can be added and subtracted.
    fn key(&self) -> &str;
    /// The stack's size.
    fn count(&self) -> u32;
    /// The same item and components with `count`; `None` for 0.
    fn with_count(&self, count: u32) -> Option<Self>
    where
        Self: Sized;
}

/// The most one slot holds (a stack). A merge never writes more than this into
/// the store; what does not fit stays in the chest that inserted it.
pub const MAX_PER_SLOT: u32 = 64;

/// Every frequency's inventory, the world's truth.
#[derive(Clone, Debug, PartialEq)]
pub struct Store<T> {
    frequencies: BTreeMap<Frequency, Vec<Option<T>>>,
}

impl<T: Slot> Default for Store<T> {
    fn default() -> Self {
        Store {
            frequencies: BTreeMap::new(),
        }
    }
}

impl<T: Slot> Store<T> {
    pub fn new() -> Self {
        Store::default()
    }

    /// Whether this frequency has an entry (an empty entry still counts).
    pub fn contains(&self, frequency: Frequency) -> bool {
        self.frequencies.contains_key(&frequency)
    }

    /// The frequency's slots, `SLOTS` long; empty when it has no entry.
    pub fn slots(&self, frequency: Frequency) -> Vec<Option<T>> {
        self.frequencies
            .get(&frequency)
            .cloned()
            .unwrap_or_else(|| vec![None; SLOTS])
    }

    pub fn set(&mut self, frequency: Frequency, slots: Vec<Option<T>>) {
        self.frequencies.insert(frequency, pad(&slots));
    }

    /// The non-empty slots of a frequency.
    pub fn count(&self, frequency: Frequency) -> usize {
        self.frequencies.get(&frequency).map_or(0, |slots| {
            slots.iter().filter(|slot| slot.is_some()).count()
        })
    }

    /// The store file's text: a header, then one `f` line per frequency and
    /// one `i <slot> <snbt>` line per item.
    pub fn encode(&self) -> String {
        let mut out = String::from("# enderstorage 1\n");
        for (frequency, slots) in &self.frequencies {
            out.push_str("f ");
            out.push_str(&frequency.text());
            out.push('\n');
            for (index, slot) in slots.iter().enumerate() {
                if let Some(item) = slot {
                    out.push_str(&format!("i {index} {}\n", item.encode()));
                }
            }
        }
        out
    }

    /// Parse the store file; bad lines are skipped and reported.
    pub fn decode(text: &str) -> (Self, Vec<String>) {
        let mut store = Store::new();
        let mut warnings = Vec::new();
        let mut current = None;
        for (number, raw) in text.lines().enumerate() {
            let line = raw.trim_end_matches('\r').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("f ") {
                match Frequency::parse(rest.trim()) {
                    Some(frequency) => {
                        store
                            .frequencies
                            .entry(frequency)
                            .or_insert_with(|| vec![None; SLOTS]);
                        current = Some(frequency);
                    }
                    None => {
                        warnings.push(format!(
                            "line {}: bad frequency {:?}",
                            number + 1,
                            rest.trim()
                        ));
                        current = None;
                    }
                }
                continue;
            }
            if let Some(rest) = line.strip_prefix("i ") {
                let Some(frequency) = current else {
                    warnings.push(format!("line {}: item before a frequency", number + 1));
                    continue;
                };
                let Some((index_text, item_text)) = rest.split_once(' ') else {
                    warnings.push(format!("line {}: item line without a slot", number + 1));
                    continue;
                };
                let Ok(index) = index_text.parse::<usize>() else {
                    warnings.push(format!("line {}: bad slot {:?}", number + 1, index_text));
                    continue;
                };
                if index >= SLOTS {
                    warnings.push(format!("line {}: slot {index} is past {SLOTS}", number + 1));
                    continue;
                }
                match T::decode(item_text) {
                    Some(item) => {
                        if let Some(slots) = store.frequencies.get_mut(&frequency) {
                            slots[index] = Some(item);
                        }
                    }
                    None => warnings.push(format!("line {}: bad item {:?}", number + 1, item_text)),
                }
                continue;
            }
            warnings.push(format!("line {}: unknown line {line:?}", number + 1));
        }
        (store, warnings)
    }
}

/// How a tick's merge went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tick {
    /// The store was the truth; the chest took its slots.
    Adopted,
    /// The chest was the only copy of an unknown frequency; it filled the store.
    Bootstrapped,
    /// Slot-wise changes went both ways.
    Merged,
}

/// One chest's tick result.
#[derive(Clone, Debug, PartialEq)]
pub struct Synced<T> {
    pub items: Vec<Option<T>>,
    pub store_changed: bool,
    pub how: Tick,
}

/// Merge one chest's slots with the store. `snapshot` is what the chest held
/// after its previous tick; none on the first tick after it loads, when the
/// store (or, for an unknown frequency that has items, the chest) wins.
pub fn sync<T: Slot>(
    items: &[Option<T>],
    snapshot: Option<&[Option<T>]>,
    store: &mut Store<T>,
    frequency: Frequency,
) -> Synced<T> {
    let current = pad(items);
    let Some(snapshot) = snapshot else {
        let known = store.contains(frequency);
        if !known && current.iter().any(Option::is_some) {
            store.set(frequency, current.clone());
            return Synced {
                items: current,
                store_changed: true,
                how: Tick::Bootstrapped,
            };
        }
        let stored = store.slots(frequency);
        let store_changed = !known;
        if store_changed {
            store.set(frequency, stored.clone());
        }
        return Synced {
            items: stored,
            store_changed,
            how: Tick::Adopted,
        };
    };
    let snapshot = pad(snapshot);
    let mut stored = store.slots(frequency);
    let known = store.contains(frequency);
    let mut merged = current.clone();
    let mut changed = false;
    for index in 0..SLOTS {
        let (authoritative, shown) = merge_slot(
            snapshot[index].as_ref(),
            current[index].as_ref(),
            stored[index].as_ref(),
        );
        if authoritative != stored[index] {
            stored[index] = authoritative;
            changed = true;
        }
        merged[index] = shown;
    }
    if changed || !known {
        store.set(frequency, stored);
        changed = true;
    }
    Synced {
        items: merged,
        store_changed: changed,
        how: Tick::Merged,
    }
}

/// How far from a block a player without operator rights may still read it: the vanilla
/// block-interaction reach (4.5 blocks) plus slack, from the player's position to the block
/// centre. `/enderstorage info` names a frequency — the key that joins a public linked store — so
/// a remote read has to be an operator's.
pub const READ_REACH: f64 = 6.0;

/// Whether a command source may read the block at `pos`.
///
/// `operator` is the source's permission level reaching 2; a player below it may read only a
/// block within [`READ_REACH`], and a non-player source without it reads nothing.
#[must_use]
pub fn may_read(operator: bool, player: Option<[f64; 3]>, pos: [i32; 3]) -> bool {
    if operator {
        return true;
    }
    player.is_some_and(|player| {
        let centre = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        let distance_squared: f64 = (0..3)
            .map(|axis| (player[axis] - centre[axis]).powi(2))
            .sum();
        distance_squared <= READ_REACH * READ_REACH
    })
}

/// One slot's merge: the value the store should hold and the value the chest
/// should show.
///
/// `snap` is what this chest last saw, `local` what it holds now and `remote`
/// what the store holds now. A chest that did not touch the slot adopts the
/// store's value. A chest that did touch it is applied as a **count delta** on
/// top of the store's value, not as a new absolute value: two chests that each
/// withdraw 32 from one stack of 64 must subtract twice, and two that each
/// insert must add twice. Writing the local absolute value instead let the
/// second withdrawal land on the first's 32 and resurrect items.
///
/// A slot whose *item* changed (a hopper swapping a stack) is a replacement, not
/// a delta: one slot holds one item, so the local edit is the newer value. That
/// is the one case a mirrored slot cannot reconcile; the other chest's stack
/// stays in that chest and is offered again on its next tick.
fn merge_slot<T: Slot>(
    snap: Option<&T>,
    local: Option<&T>,
    remote: Option<&T>,
) -> (Option<T>, Option<T>) {
    if local == snap {
        // Untouched here: the store is the truth.
        return (remote.cloned(), remote.cloned());
    }
    let same_item = snap.is_some_and(|snap| local.is_none_or(|local| local.key() == snap.key()));
    if !same_item {
        return (local.cloned(), local.cloned());
    }
    let snap = snap.expect("same_item needs a snapshot");
    if remote.is_some_and(|remote| remote.key() != snap.key()) {
        // Another chest replaced the slot with a different item at the same
        // time: a tie. The local edit is the newer value.
        return (local.cloned(), local.cloned());
    }
    let base = remote.map_or(0, |remote| i64::from(remote.count()));
    let delta = local.map_or(0, |local| i64::from(local.count())) - i64::from(snap.count());
    let want = base + delta;
    let kept = want.clamp(0, i64::from(MAX_PER_SLOT));
    let item = local.unwrap_or(snap);
    let stored = item.with_count(u32::try_from(kept).unwrap_or(0));
    // What the chest shows: the store's value, or — when the edit does not fit
    // — the whole local stack, so an item is never destroyed by the merge.
    let shown = if want > kept {
        item.with_count(u32::try_from(want).unwrap_or(u32::MAX))
    } else {
        stored.clone()
    };
    (stored, shown)
}

/// Write a chest's locally changed slots to the store; whether it changed.
/// Nothing happens without a snapshot: the chest is then not authoritative.
pub fn commit<T: Slot>(
    items: &[Option<T>],
    snapshot: Option<&[Option<T>]>,
    store: &mut Store<T>,
    frequency: Frequency,
) -> bool {
    let Some(snapshot) = snapshot else {
        return false;
    };
    let current = pad(items);
    let snapshot = pad(snapshot);
    let mut stored = store.slots(frequency);
    let mut changed = false;
    for index in 0..SLOTS {
        let (authoritative, _shown) = merge_slot(
            snapshot[index].as_ref(),
            current[index].as_ref(),
            stored[index].as_ref(),
        );
        if authoritative != stored[index] {
            stored[index] = authoritative;
            changed = true;
        }
    }
    if changed {
        store.set(frequency, stored);
    }
    changed
}

/// Re-colour a chest: commit what it held to the old frequency, then show
/// the new frequency's inventory. Returns the new slots and whether the
/// store changed.
pub fn switch<T: Slot>(
    items: &[Option<T>],
    snapshot: Option<&[Option<T>]>,
    store: &mut Store<T>,
    from: Frequency,
    to: Frequency,
) -> (Vec<Option<T>>, bool) {
    let changed = commit(items, snapshot, store, from);
    (store.slots(to), changed)
}

/// `items`/`snapshot` padded or cut to `SLOTS`.
fn pad<T: Clone>(items: &[Option<T>]) -> Vec<Option<T>> {
    let mut out = items.to_vec();
    out.resize(SLOTS, None);
    out.truncate(SLOTS);
    out
}

// ---------------------------------------------------------------------------
// The ender tank (slice 2): one 16-bucket tank per frequency.
// ---------------------------------------------------------------------------

/// Upstream's `EnderLiquidStorage` capacity: 16 buckets, in millibuckets.
pub const TANK_CAPACITY: i32 = 16_000;
/// A bucket, in millibuckets.
pub const BUCKET: i32 = 1_000;

/// One fluid tank: a fluid id (`minecraft:water`, `minecraft:lava`, or a
/// mod's own) and its amount in millibuckets. Empty is no fluid and 0.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tank {
    pub fluid: Option<String>,
    pub amount: i32,
}

impl Tank {
    pub const EMPTY: Tank = Tank {
        fluid: None,
        amount: 0,
    };

    /// A tank holding `amount` millibuckets of `fluid`, clamped to the
    /// capacity; an amount of 0 is empty.
    pub fn new(fluid: &str, amount: i32) -> Self {
        let mut tank = Tank {
            fluid: Some(fluid.to_owned()),
            amount,
        };
        tank.normalise();
        tank
    }

    pub fn is_empty(&self) -> bool {
        self.amount <= 0 || self.fluid.is_none()
    }

    /// How much of `fluid` this tank can still take: none for another fluid.
    pub fn room_for(&self, fluid: &str) -> i32 {
        if self.is_empty() {
            return TANK_CAPACITY;
        }
        if self.fluid.as_deref() == Some(fluid) {
            return (TANK_CAPACITY - self.amount).max(0);
        }
        0
    }

    /// Put up to `amount` of `fluid` in; answers what did not fit.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn push(&mut self, fluid: &str, amount: i32) -> i32 {
        let amount = amount.max(0);
        let room = self.room_for(fluid);
        let add = room.min(amount);
        if add > 0 {
            if self.is_empty() {
                self.fluid = Some(fluid.to_owned());
            }
            self.amount += add;
        }
        self.normalise();
        amount - add
    }

    /// Take up to `amount` out; answers what left.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn take(&mut self, amount: i32) -> i32 {
        let take = amount.max(0).min(self.amount.max(0));
        self.amount -= take;
        self.normalise();
        take
    }

    /// Keep the invariant: 0..=TANK_CAPACITY and no fluid without an amount.
    fn normalise(&mut self) {
        self.amount = self.amount.clamp(0, TANK_CAPACITY);
        if self.amount == 0 {
            self.fluid = None;
        }
    }
}

/// Every frequency's tank, the world's truth (upstream's liquid storage).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TankStore {
    tanks: BTreeMap<Frequency, Tank>,
}

impl TankStore {
    pub fn new() -> Self {
        TankStore::default()
    }

    /// Whether this frequency has an entry (an empty tank still counts).
    pub fn contains(&self, frequency: Frequency) -> bool {
        self.tanks.contains_key(&frequency)
    }

    /// The frequency's tank; empty when it has no entry.
    pub fn tank(&self, frequency: Frequency) -> Tank {
        self.tanks.get(&frequency).cloned().unwrap_or(Tank::EMPTY)
    }

    pub fn set(&mut self, frequency: Frequency, tank: Tank) {
        let mut tank = tank;
        tank.normalise();
        self.tanks.insert(frequency, tank);
    }

    /// The store file's text: a header, then one `t <frequency> <fluid|-> <mb>`
    /// line per frequency.
    pub fn encode(&self) -> String {
        let mut out = String::from("# enderstorage-tanks 1\n");
        for (frequency, tank) in &self.tanks {
            out.push_str(&format!(
                "t {} {} {}\n",
                frequency.text(),
                tank.fluid.as_deref().unwrap_or("-"),
                tank.amount
            ));
        }
        out
    }

    /// Parse the tank file; bad lines are skipped and reported.
    pub fn decode(text: &str) -> (Self, Vec<String>) {
        let mut store = TankStore::new();
        let mut warnings = Vec::new();
        for (number, raw) in text.lines().enumerate() {
            let line = raw.trim_end_matches('\r').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some(rest) = line.strip_prefix("t ") else {
                warnings.push(format!("line {}: unknown line {line:?}", number + 1));
                continue;
            };
            let mut words = rest.split_whitespace();
            let (Some(frequency), Some(fluid), Some(amount), None) =
                (words.next(), words.next(), words.next(), words.next())
            else {
                warnings.push(format!("line {}: bad tank line {line:?}", number + 1));
                continue;
            };
            let Some(frequency) = Frequency::parse(frequency) else {
                warnings.push(format!("line {}: bad frequency {frequency:?}", number + 1));
                continue;
            };
            let Ok(amount) = amount.parse::<i32>() else {
                warnings.push(format!("line {}: bad amount {amount:?}", number + 1));
                continue;
            };
            let tank = if fluid == "-" {
                Tank::EMPTY
            } else {
                Tank::new(fluid, amount)
            };
            store.set(frequency, tank);
        }
        (store, warnings)
    }
}

/// One tank's tick result.
#[derive(Clone, Debug, PartialEq)]
pub struct SyncedTank {
    pub tank: Tank,
    pub store_changed: bool,
    pub how: Tick,
}

/// Merge one tank's fluid with the store, exactly as `sync` merges slots: on
/// the first tick the store wins (or an unknown frequency's non-empty tank
/// bootstraps it), afterwards a local change is written and a remote one is
/// adopted, with the local tank winning a tie.
pub fn sync_tank(
    tank: &Tank,
    snapshot: Option<&Tank>,
    store: &mut TankStore,
    frequency: Frequency,
) -> SyncedTank {
    let Some(snapshot) = snapshot else {
        let known = store.contains(frequency);
        if !known && !tank.is_empty() {
            store.set(frequency, tank.clone());
            return SyncedTank {
                tank: tank.clone(),
                store_changed: true,
                how: Tick::Bootstrapped,
            };
        }
        let stored = store.tank(frequency);
        let store_changed = !known;
        if store_changed {
            store.set(frequency, stored.clone());
        }
        return SyncedTank {
            tank: stored,
            store_changed,
            how: Tick::Adopted,
        };
    };
    let stored = store.tank(frequency);
    let known = store.contains(frequency);
    let (authoritative, shown) = merge_tank(snapshot, tank, &stored);
    let store_changed = authoritative != stored || !known;
    if store_changed {
        store.set(frequency, authoritative);
    }
    SyncedTank {
        tank: shown,
        store_changed,
        how: Tick::Merged,
    }
}

/// One tank's merge, as [`merge_slot`] does a slot: an untouched tank adopts
/// the store, an edited one is applied as an **amount delta** on top of it, so
/// two chests each bucketing out of one tank subtract twice. A different fluid
/// is a replacement (one tank holds one fluid), where the local value is newer.
fn merge_tank(snapshot: &Tank, local: &Tank, remote: &Tank) -> (Tank, Tank) {
    if local == snapshot {
        return (remote.clone(), remote.clone());
    }
    let same_fluid = local.is_empty() || local.fluid == snapshot.fluid;
    if !same_fluid || (!remote.is_empty() && remote.fluid != snapshot.fluid) {
        return (local.clone(), local.clone());
    }
    let base = i64::from(if remote.fluid == snapshot.fluid {
        remote.amount
    } else {
        0
    });
    let delta = i64::from(local.amount) - i64::from(snapshot.amount);
    let want = base + delta;
    let kept = want.clamp(0, i64::from(TANK_CAPACITY));
    let fluid = local.fluid.as_deref().or(snapshot.fluid.as_deref());
    let tank = |amount: i64| match fluid {
        Some(fluid) if amount > 0 => Tank::new(fluid, i32::try_from(amount).unwrap_or(i32::MAX)),
        _ => Tank::EMPTY,
    };
    let stored = tank(kept);
    let shown = if want > kept {
        tank(want)
    } else {
        stored.clone()
    };
    (stored, shown)
}

/// Write a tank's local change to the store; whether it changed. Nothing
/// happens without a snapshot: the tank is then not authoritative.
pub fn commit_tank(
    tank: &Tank,
    snapshot: Option<&Tank>,
    store: &mut TankStore,
    frequency: Frequency,
) -> bool {
    let Some(snapshot) = snapshot else {
        return false;
    };
    let stored = store.tank(frequency);
    let (authoritative, _shown) = merge_tank(snapshot, tank, &stored);
    if authoritative != stored {
        store.set(frequency, authoritative);
        return true;
    }
    false
}

/// Re-colour a tank: commit what it held to the old frequency, then show the
/// new frequency's tank.
pub fn switch_tank(
    tank: &Tank,
    snapshot: Option<&Tank>,
    store: &mut TankStore,
    from: Frequency,
    to: Frequency,
) -> (Tank, bool) {
    let changed = commit_tank(tank, snapshot, store, from);
    (store.tank(to), changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for the game's item stack.
    #[derive(Clone, Debug, PartialEq, Eq)]
    struct It(&'static str);

    impl Slot for It {
        fn encode(&self) -> String {
            self.0.to_owned()
        }
        fn decode(text: &str) -> Option<Self> {
            let name: &'static str = Box::leak(text.to_owned().into_boxed_str());
            (!name.is_empty()).then_some(It(name))
        }
        fn key(&self) -> &str {
            self.0
        }
        fn count(&self) -> u32 {
            1
        }
        fn with_count(&self, count: u32) -> Option<Self> {
            (count > 0).then_some(It(self.0))
        }
    }

    /// A stack with a count, as a real item is: the merge works on counts.
    #[derive(Clone, Debug, PartialEq)]
    struct Stack {
        item: &'static str,
        count: u32,
    }

    impl Slot for Stack {
        fn encode(&self) -> String {
            format!("{} x{}", self.item, self.count)
        }
        fn decode(text: &str) -> Option<Self> {
            let (item, count) = text.rsplit_once(" x")?;
            Some(Stack {
                item: Box::leak(item.to_owned().into_boxed_str()),
                count: count.parse().ok()?,
            })
        }
        fn key(&self) -> &str {
            self.item
        }
        fn count(&self) -> u32 {
            self.count
        }
        fn with_count(&self, count: u32) -> Option<Self> {
            (count > 0).then_some(Stack {
                item: self.item,
                count,
            })
        }
    }

    fn stack(count: u32) -> Option<Stack> {
        (count > 0).then_some(Stack {
            item: "minecraft:diamond",
            count,
        })
    }

    fn stacks(entries: &[(usize, u32)]) -> Vec<Option<Stack>> {
        let mut slots = vec![None; SLOTS];
        for &(index, count) in entries {
            slots[index] = stack(count);
        }
        slots
    }

    #[test]
    fn only_an_operator_or_a_nearby_player_reads_a_frequency() {
        // The frequency is the key to a public linked store, so `/enderstorage info` answers a
        // player only when they can touch the block, and a non-player source only at level 2.
        let pos = [10, 64, 10];
        assert!(may_read(true, Some([500.0, 64.0, 500.0]), pos));
        assert!(may_read(false, Some([13.0, 64.5, 10.5]), pos));
        assert!(!may_read(false, Some([10.5, 64.5, 17.0]), pos));
        assert!(!may_read(false, None, pos));
        assert!(may_read(false, Some([10.5 + READ_REACH, 64.5, 10.5]), pos));
    }

    #[test]
    fn a_sibling_that_has_not_changed_adopts_the_committed_withdrawal() {
        // A chest's inventory is a copy of the frequency; a sibling keeps its copy between ticks.
        // The fix commits an access to the store at once and pushes the store's slots into every
        // loaded sibling, so this is what a sibling's next merge must see: the committed value,
        // never the stale stack it still held.
        let mut store = Store::new();
        store.set(RED, stacks(&[(0, 64)]));
        let before = stacks(&[(0, 64)]);
        assert!(commit(&vec![None; SLOTS], Some(&before), &mut store, RED));
        assert_eq!(store.slots(RED)[0], None);
        let sibling = sync(&before, Some(&before), &mut store, RED);
        assert_eq!(sibling.items[0], None, "the sibling hands out nothing");
        assert_eq!(store.slots(RED)[0], None);
    }

    #[test]
    fn two_chests_each_withdraw_from_one_shared_stack() {
        // The reviewer's case: both chests mirror a stack of 64 and each loses
        // 32 before its next sync. Both withdrawals are real (the players hold
        // them), so the store must go 64 -> 32 -> 0, not stay at 32.
        let mut store = Store::new();
        store.set(RED, stacks(&[(0, 64)]));
        let snapshot = stacks(&[(0, 64)]);

        let first = sync(&stacks(&[(0, 32)]), Some(&snapshot), &mut store, RED);
        assert_eq!(first.items[0], stack(32));
        assert_eq!(store.slots(RED)[0], stack(32));

        // The second chest still shows its own 64 and has just withdrawn 32.
        let second = sync(&stacks(&[(0, 32)]), Some(&snapshot), &mut store, RED);
        assert_eq!(
            store.slots(RED)[0],
            None,
            "the second withdrawal subtracts what the first left"
        );
        assert_eq!(second.items[0], None, "and both chests now show nothing");
        assert_eq!(
            first.items[0],
            stack(32),
            "the first chest's view is unchanged"
        );
    }

    #[test]
    fn two_chests_each_insert_into_one_shared_stack() {
        let mut store = Store::new();
        store.set(RED, stacks(&[(0, 10)]));
        let snapshot = stacks(&[(0, 10)]);
        let first = sync(&stacks(&[(0, 30)]), Some(&snapshot), &mut store, RED);
        assert_eq!(first.items[0], stack(30));
        // The other chest inserts its own 20 on top of the same stack.
        let second = sync(&stacks(&[(0, 30)]), Some(&snapshot), &mut store, RED);
        assert_eq!(
            store.slots(RED)[0],
            stack(50),
            "both insertions add: 10 + 20 + 20"
        );
        assert_eq!(second.items[0], stack(50), "the chest shows the sum");
    }

    #[test]
    fn an_insert_that_does_not_fit_is_kept_in_the_chest_not_destroyed() {
        // Two chests each put four more items on a stack of 60. The store fills
        // to its 64; the second four cannot fit, so that chest keeps them
        // visible instead of the merge destroying them.
        let mut store = Store::new();
        store.set(RED, stacks(&[(0, 60)]));
        let snapshot = stacks(&[(0, 60)]);
        let first = sync(&stacks(&[(0, 64)]), Some(&snapshot), &mut store, RED);
        assert_eq!(first.items[0], stack(64));
        assert_eq!(store.slots(RED)[0], stack(64));
        let second = sync(&stacks(&[(0, 64)]), Some(&snapshot), &mut store, RED);
        assert_eq!(store.slots(RED)[0], stack(64), "the store holds one stack");
        assert_eq!(
            second.items[0],
            stack(68),
            "the four that do not fit stay in the chest that inserted them"
        );
    }

    fn slot(name: &'static str) -> Option<It> {
        Some(It(name))
    }

    fn empty() -> Vec<Option<It>> {
        vec![None; SLOTS]
    }

    fn with(entries: &[(usize, &'static str)]) -> Vec<Option<It>> {
        let mut slots = empty();
        for &(index, name) in entries {
            slots[index] = slot(name);
        }
        slots
    }

    const RED: Frequency = Frequency {
        left: Colour::Red,
        middle: Colour::White,
        right: Colour::White,
    };
    const BLUE: Frequency = Frequency {
        left: Colour::Blue,
        middle: Colour::Blue,
        right: Colour::Blue,
    };

    #[test]
    fn colour_names_round_trip() {
        for colour in Colour::ALL {
            assert_eq!(Colour::from_name(colour.name()), Some(colour));
        }
        assert_eq!(Colour::from_name("Light_Blue"), Some(Colour::LightBlue));
        assert_eq!(Colour::from_name("light blue"), Some(Colour::LightBlue));
        assert_eq!(Colour::from_name("chartreuse"), None);
        assert_eq!(Colour::ALL.len(), 16);
    }

    #[test]
    fn dye_items_map_to_colours() {
        assert_eq!(Colour::from_dye("minecraft:red_dye"), Some(Colour::Red));
        assert_eq!(
            Colour::from_dye("minecraft:light_blue_dye"),
            Some(Colour::LightBlue)
        );
        assert_eq!(Colour::from_dye("minecraft:red_wool"), None);
        assert_eq!(Colour::from_dye("minecraft:stick"), None);
    }

    #[test]
    fn frequency_parse_and_text() {
        assert_eq!(
            Frequency::parse("white,white,white"),
            Some(Frequency::DEFAULT)
        );
        assert_eq!(
            Frequency::parse("red,light_blue,black"),
            Some(Frequency::new(
                Colour::Red,
                Colour::LightBlue,
                Colour::Black
            ))
        );
        assert_eq!(Frequency::parse("red,white"), None);
        assert_eq!(Frequency::parse("red,white,white,black"), None);
        assert_eq!(Frequency::parse("red,white,plaid"), None);
        assert_eq!(Frequency::DEFAULT.text(), "white,white,white");
        assert_eq!(RED.text(), "red,white,white");
        assert_eq!(RED.label(), "red / white / white");
    }

    #[test]
    fn frequency_bands_and_cycle() {
        assert_eq!(Frequency::DEFAULT.band(Band::Left), Colour::White);
        let red_left = Frequency::DEFAULT.with(Band::Left, Colour::Red);
        assert_eq!(red_left.left, Colour::Red);
        assert_eq!(red_left.middle, Colour::White);
        let all = red_left
            .with(Band::Middle, Colour::Blue)
            .with(Band::Right, Colour::Black);
        assert_eq!(
            all,
            Frequency::new(Colour::Red, Colour::Blue, Colour::Black)
        );
        assert_eq!(Band::Left.next(), Band::Middle);
        assert_eq!(Band::Middle.next(), Band::Right);
        assert_eq!(Band::Right.next(), Band::Left);
        assert_eq!(Band::from_index(2), Band::Right);
        assert_eq!(Band::from_index(9), Band::Left);
        assert_eq!(Band::Left.name(), "left");
    }

    #[test]
    fn store_encode_decode_round_trip() {
        let mut store = Store::new();
        store.set(
            RED,
            with(&[(0, "minecraft:diamond"), (4, "minecraft:stone")]),
        );
        store.set(BLUE, empty());
        let text = store.encode();
        let (decoded, warnings) = Store::<It>::decode(&text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(decoded, store);
        assert!(decoded.contains(RED));
        assert!(decoded.contains(BLUE));
        assert!(!decoded.contains(Frequency::DEFAULT));
        assert_eq!(decoded.count(RED), 2);
        assert_eq!(decoded.count(BLUE), 0);
    }

    #[test]
    fn store_decode_keeps_good_lines_and_warns() {
        let text = "\
# enderstorage 1
f red,white,white
i 0 minecraft:diamond
i 1 minecraft:stone
i 99 minecraft:gold_ingot
not a line
f plaid,white,white
f blue,blue,blue
";
        let (store, warnings) = Store::<It>::decode(text);
        assert_eq!(store.count(RED), 2);
        assert!(store.contains(BLUE));
        assert_eq!(store.count(BLUE), 0);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("plaid")));
        assert!(warnings.iter().any(|w| w.contains("99")));
        assert!(warnings.iter().any(|w| w.contains("not a line")));
    }

    #[test]
    fn first_tick_adopts_the_store() {
        let mut store = Store::new();
        store.set(RED, with(&[(2, "minecraft:emerald")]));
        let synced = sync(&empty(), None, &mut store, RED);
        assert_eq!(synced.how, Tick::Adopted);
        assert!(!synced.store_changed);
        assert_eq!(synced.items, with(&[(2, "minecraft:emerald")]));
    }

    #[test]
    fn first_tick_bootstraps_an_unknown_frequency() {
        let mut store = Store::new();
        let items = with(&[(0, "minecraft:diamond")]);
        let synced = sync(&items, None, &mut store, RED);
        assert_eq!(synced.how, Tick::Bootstrapped);
        assert!(synced.store_changed);
        assert_eq!(synced.items, items);
        assert_eq!(store.slots(RED), items);
    }

    #[test]
    fn first_tick_adopts_a_known_empty_frequency() {
        let mut store = Store::new();
        store.set(RED, empty());
        let stale = with(&[(0, "minecraft:diamond")]);
        let synced = sync(&stale, None, &mut store, RED);
        assert_eq!(synced.how, Tick::Adopted);
        assert_eq!(synced.items, empty());
        assert_eq!(store.count(RED), 0);
    }

    #[test]
    fn tick_commits_a_local_change() {
        let mut store = Store::new();
        store.set(RED, empty());
        let snapshot = empty();
        let items = with(&[(3, "minecraft:diamond")]);
        let synced = sync(&items, Some(&snapshot), &mut store, RED);
        assert_eq!(synced.how, Tick::Merged);
        assert!(synced.store_changed);
        assert_eq!(synced.items, items);
        assert_eq!(store.count(RED), 1);
    }

    #[test]
    fn tick_adopts_a_remote_change() {
        let mut store = Store::new();
        store.set(RED, with(&[(1, "minecraft:gold_ingot")]));
        let snapshot = empty();
        let synced = sync(&empty(), Some(&snapshot), &mut store, RED);
        assert!(!synced.store_changed);
        assert_eq!(synced.items, with(&[(1, "minecraft:gold_ingot")]));
    }

    #[test]
    fn tick_merges_both_sides_per_slot() {
        let mut store = Store::new();
        store.set(RED, with(&[(1, "minecraft:gold_ingot")]));
        let snapshot = empty();
        let items = with(&[(0, "minecraft:diamond")]);
        let synced = sync(&items, Some(&snapshot), &mut store, RED);
        assert!(synced.store_changed);
        assert_eq!(
            synced.items,
            with(&[(0, "minecraft:diamond"), (1, "minecraft:gold_ingot")])
        );
        assert_eq!(store.count(RED), 2);
    }

    #[test]
    fn tick_local_wins_the_same_slot() {
        let mut store = Store::new();
        store.set(RED, with(&[(0, "minecraft:gold_ingot")]));
        let snapshot = empty();
        let items = with(&[(0, "minecraft:diamond")]);
        let synced = sync(&items, Some(&snapshot), &mut store, RED);
        assert!(synced.store_changed);
        assert_eq!(synced.items, with(&[(0, "minecraft:diamond")]));
        assert_eq!(store.slots(RED), with(&[(0, "minecraft:diamond")]));
    }

    #[test]
    fn switch_commits_the_old_frequency() {
        let mut store = Store::new();
        store.set(BLUE, with(&[(0, "minecraft:emerald")]));
        let snapshot = empty();
        let items = with(&[(0, "minecraft:diamond")]);
        let (new_items, changed) = switch(&items, Some(&snapshot), &mut store, RED, BLUE);
        assert!(changed);
        assert_eq!(store.slots(RED), items);
        assert_eq!(new_items, with(&[(0, "minecraft:emerald")]));
    }

    #[test]
    fn switch_to_an_unknown_frequency_is_empty() {
        let mut store = Store::new();
        store.set(RED, with(&[(0, "minecraft:diamond")]));
        let (new_items, changed) = switch(&empty(), None, &mut store, RED, BLUE);
        assert!(!changed);
        assert_eq!(new_items, empty());
        assert_eq!(store.count(RED), 1);
    }

    #[test]
    fn commit_without_a_snapshot_is_a_no_op() {
        let mut store = Store::new();
        let items = with(&[(0, "minecraft:diamond")]);
        assert!(!commit(&items, None, &mut store, RED));
        assert!(!store.contains(RED));
    }

    #[test]
    fn short_and_long_item_lists_are_padded_and_cut() {
        let mut store = Store::new();
        let synced = sync(&[slot("minecraft:diamond")], None, &mut store, RED);
        assert_eq!(synced.items.len(), SLOTS);
        assert_eq!(synced.items[0], slot("minecraft:diamond"));
        let mut long = empty();
        long.push(slot("minecraft:extra"));
        long.push(slot("minecraft:extra"));
        let synced = sync(&long, Some(&empty()), &mut store, RED);
        assert_eq!(synced.items.len(), SLOTS);
    }
}

#[cfg(test)]
mod tank_tests {
    use super::*;

    const RED: Frequency = Frequency {
        left: Colour::Red,
        middle: Colour::White,
        right: Colour::White,
    };
    const BLUE: Frequency = Frequency {
        left: Colour::Blue,
        middle: Colour::Blue,
        right: Colour::Blue,
    };

    fn water(amount: i32) -> Tank {
        Tank::new("minecraft:water", amount)
    }

    #[test]
    fn two_tanks_each_bucket_out_of_one_shared_tank() {
        // The same conflict `two_chests_each_withdraw_from_one_shared_stack`
        // covers, on the fluid side: each chest's tank lost a bucket before its
        // sync, so the store must drop two.
        let mut store = TankStore::new();
        store.set(RED, water(4_000));
        let snapshot = water(4_000);
        let first = sync_tank(&water(3_000), Some(&snapshot), &mut store, RED);
        assert_eq!(first.tank, water(3_000));
        assert_eq!(store.tank(RED), water(3_000));
        let second = sync_tank(&water(3_000), Some(&snapshot), &mut store, RED);
        assert_eq!(store.tank(RED), water(2_000), "both buckets are subtracted");
        assert_eq!(second.tank, water(2_000));
    }

    #[test]
    fn two_tanks_each_fill_one_shared_tank() {
        let mut store = TankStore::new();
        store.set(RED, water(1_000));
        let snapshot = water(1_000);
        let first = sync_tank(&water(2_000), Some(&snapshot), &mut store, RED);
        assert_eq!(first.tank, water(2_000));
        let second = sync_tank(&water(2_000), Some(&snapshot), &mut store, RED);
        assert_eq!(store.tank(RED), water(3_000), "both buckets are added");
        assert_eq!(second.tank, water(3_000));
    }

    #[test]
    fn a_tank_clamps_to_its_capacity() {
        assert_eq!(Tank::new("minecraft:water", 20_000).amount, TANK_CAPACITY);
        assert_eq!(Tank::new("minecraft:water", -5), Tank::EMPTY);
        assert!(Tank::EMPTY.is_empty());
        assert_eq!(Tank::EMPTY.room_for("minecraft:water"), TANK_CAPACITY);
    }

    #[test]
    fn push_and_take_respect_the_fluid() {
        let mut tank = water(15_500);
        assert_eq!(tank.room_for("minecraft:lava"), 0);
        assert_eq!(tank.push("minecraft:lava", BUCKET), BUCKET);
        assert_eq!(tank.push("minecraft:water", BUCKET), 500);
        assert_eq!(tank.amount, TANK_CAPACITY);
        assert_eq!(tank.take(BUCKET), BUCKET);
        assert_eq!(tank.amount, TANK_CAPACITY - BUCKET);
        assert_eq!(tank.take(TANK_CAPACITY), TANK_CAPACITY - BUCKET);
        assert!(tank.is_empty());
        assert_eq!(tank.fluid, None);
    }

    #[test]
    fn tank_store_round_trips_and_warns() {
        let mut store = TankStore::new();
        store.set(RED, water(3_000));
        store.set(BLUE, Tank::EMPTY);
        let text = store.encode();
        let (decoded, warnings) = TankStore::decode(&text);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(decoded, store);
        assert!(decoded.contains(BLUE));
        assert_eq!(decoded.tank(RED), water(3_000));

        let (partial, warnings) = TankStore::decode(
            "# enderstorage-tanks 1\nt red,white,white minecraft:water 1000\n\
             t plaid,white,white minecraft:water 1\nnot a line\nt blue,blue,blue minecraft:water x\n",
        );
        assert_eq!(partial.tank(RED), water(1_000));
        assert_eq!(warnings.len(), 3, "{warnings:?}");
    }

    #[test]
    fn first_tank_tick_adopts_the_store() {
        let mut store = TankStore::new();
        store.set(RED, water(4_000));
        let synced = sync_tank(&Tank::EMPTY, None, &mut store, RED);
        assert_eq!(synced.how, Tick::Adopted);
        assert!(!synced.store_changed);
        assert_eq!(synced.tank, water(4_000));
    }

    #[test]
    fn first_tank_tick_bootstraps_an_unknown_frequency() {
        let mut store = TankStore::new();
        let synced = sync_tank(&water(2_000), None, &mut store, RED);
        assert_eq!(synced.how, Tick::Bootstrapped);
        assert!(synced.store_changed);
        assert_eq!(store.tank(RED), water(2_000));
    }

    #[test]
    fn tank_tick_commits_a_local_change() {
        let mut store = TankStore::new();
        store.set(RED, Tank::EMPTY);
        let synced = sync_tank(&water(1_000), Some(&Tank::EMPTY), &mut store, RED);
        assert_eq!(synced.how, Tick::Merged);
        assert!(synced.store_changed);
        assert_eq!(store.tank(RED), water(1_000));
    }

    #[test]
    fn tank_tick_adopts_a_remote_change() {
        let mut store = TankStore::new();
        store.set(RED, Tank::EMPTY);
        let synced = sync_tank(&water(1_000), Some(&water(1_000)), &mut store, RED);
        assert!(!synced.store_changed);
        assert_eq!(synced.tank, Tank::EMPTY);
    }

    #[test]
    fn tank_tick_applies_a_local_change_as_a_delta() {
        let mut store = TankStore::new();
        store.set(RED, water(3_000));
        // This chest's tank went 500 -> 1,000 (a +500 insert) while the store
        // moved to 3,000: the insert is added, not written over the store.
        let synced = sync_tank(&water(1_000), Some(&water(500)), &mut store, RED);
        assert!(synced.store_changed);
        assert_eq!(synced.tank, water(3_500));
        assert_eq!(store.tank(RED), water(3_500));
    }

    #[test]
    fn tank_switch_commits_the_old_frequency() {
        let mut store = TankStore::new();
        store.set(BLUE, water(5_000));
        let (tank, changed) = switch_tank(&water(1_000), Some(&Tank::EMPTY), &mut store, RED, BLUE);
        assert!(changed);
        assert_eq!(store.tank(RED), water(1_000));
        assert_eq!(tank, water(5_000));
    }

    #[test]
    fn tank_commit_without_a_snapshot_is_a_no_op() {
        let mut store = TankStore::new();
        assert!(!commit_tank(&water(1_000), None, &mut store, RED));
        assert!(!store.contains(RED));
    }
}

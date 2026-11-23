//! Native weapon-status carriers. Only owned parent rows are applied to the player;
//! the engine sends their owned, native-template payloads to targets on a hit.
//! Source-derived, without empirical game verification. Native task/offline only.
use crate::powers::EffectField;
use eldenring::{
    cs::{
        ChrInsExt, EquipParamWeapon, FieldInsHandle, GameMan, ReinforceParamWeapon,
        SoloParamRepository, SpEffectParam, WorldChrMan,
    },
    param::SP_EFFECT_PARAM_ST,
};
use fromsoftware_shared::FromStatic;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub const PARENT_BASE: u32 = 95_040_000;
pub const CHILD_BASE: u32 = 95_041_000;
const CHANNEL_COUNT: usize = 7;
/// 3.4: at most 300 buildup per hit from cards (Arcane included).
const MAX_BUILDUP: i32 = 300;

/// 0.3.1: card buildup as a real weapon passive. Owned rows 95_050_008 + channel
/// are copies of the game's own passive of that status; they go into a free
/// spEffectBehaviorId slot of every equipped weapon, and the matching
/// WeaponEffect text goes into a free spEffectMsgId slot, so every vanilla
/// weapon menu lists the status with the game's own computed total (the texts
/// read `<?bleedATKpwr?>` etc.). A hand whose weapon has no free slot keeps the
/// grease-style carrier (works, not listed).
const NATIVE_BASE: u32 = 95_050_008;
/// Vanilla passives per channel: blood 6400 (Uchigatana's), frost 6700,
/// poison 6500, rot 6600, sleep 6450, madness 6750, death 6800.
const NATIVE_TEMPLATES: [u32; CHANNEL_COUNT] = [6400, 6700, 6500, 6600, 6450, 6750, 6800];
/// (plain text, first and last text of the same status family) in WeaponEffect.fmg.
const STATUS_TEXT: [(i32, i32, i32); CHANNEL_COUNT] = [
    (6501, 6500, 6510),
    (6601, 6600, 6602),
    (6401, 6400, 6410),
    (6451, 6450, 6452),
    (6651, 6650, 6652),
    (6701, 6700, 6702),
    (6551, 6550, 6552),
];
/// Unarmed has its own weapon row; never give fists to every NPC.
const UNARMED_ROW: u32 = 110_000;
/// Rows for "vanilla passive + card buildup" copies (one per weapon row and
/// status), so a weapon that already has the status shows one combined total.
const COMBINED_BASE: u32 = 95_050_016;
const COMBINED_ROWS: u32 = 32;

/// Original passive/text slots of a weapon row we edited, and what we placed.
#[derive(Clone, Debug)]
struct WeaponPatch {
    behavior: [i32; 3],
    text: [i32; 3],
    placed: Vec<usize>,
}

/// Native passives in effect this tick, for the HUD/collection.
#[derive(Debug, Clone, Default, Serialize)]
pub struct NativePassive {
    pub weapon_row: u32,
    pub channel: String,
    pub buildup: i32,
}

#[derive(Clone, Copy)]
struct Channel {
    name: &'static str,
    field: Option<EffectField>,
    state: u16,
    attribute: u8,
    category: u16,
}

// Stable default preference. Native affinity and ordinary greases are never
// counted against this budget or removed by this module.
const CHANNELS: [Channel; CHANNEL_COUNT] = [
    Channel {
        name: "blood_loss",
        field: Some(EffectField::BloodAttackPower),
        state: 6,
        attribute: 22,
        category: 10003,
    },
    Channel {
        name: "frostbite",
        field: Some(EffectField::FreezeAttackPower),
        state: 260,
        attribute: 23,
        category: 10007,
    },
    Channel {
        name: "poison",
        field: Some(EffectField::PoizonAttackPower),
        state: 2,
        attribute: 20,
        category: 10004,
    },
    Channel {
        name: "scarlet_rot",
        field: Some(EffectField::DiseaseAttackPower),
        state: 5,
        attribute: 21,
        category: 10005,
    },
    Channel {
        name: "sleep",
        field: Some(EffectField::SleepAttackPower),
        state: 436,
        attribute: 24,
        category: 10009,
    },
    Channel {
        name: "madness",
        field: Some(EffectField::MadnessAttackPower),
        state: 437,
        attribute: 25,
        category: 10010,
    },
    // 0.8: Death Blight (Malice 30, Death's Herald).
    Channel {
        name: "death_blight",
        field: Some(EffectField::CurseAttackPower),
        state: 116,
        attribute: 26,
        category: 10008,
    },
];

#[derive(Debug, Clone, Default, Serialize)]
pub struct StatusSummary {
    pub selected_channels: Vec<String>,
    pub applied_rows: Vec<u32>,
    pub buildup: BTreeMap<String, i32>,
    pub warnings: Vec<String>,
    pub native: Vec<NativePassive>,
}

#[derive(Clone, PartialEq, Eq)]
struct OwnerKey {
    handle: FieldInsHandle,
    equipment: [i32; 6],
    selected_left: u32,
    selected_right: u32,
    arm_style: u32,
}

#[derive(Default)]
pub struct StatusImpart {
    ready: bool,
    child_templates: Option<Vec<SP_EFFECT_PARAM_ST>>,
    owner: Option<OwnerKey>,
    selected: Vec<usize>,
    priority: Option<[usize; 6]>,
    written_buildup: BTreeMap<usize, i32>,
    native_templates: Option<Vec<SP_EFFECT_PARAM_ST>>,
    native_written: BTreeMap<usize, i32>,
    /// Weapon rows edited for native passives (kept across `forget`, restored
    /// by `clear` and whenever a weapon is no longer equipped).
    patches: BTreeMap<u32, WeaponPatch>,
    /// (weapon row, channel) -> combined row index.
    combined: BTreeMap<(u32, usize), u32>,
}

impl StatusImpart {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// Set all six outgoing fields exactly once, or reset the default with [].
    /// This only orders positive requests supplied to reconcile; it cannot add
    /// buildup, activate an unowned power, or consume a native affinity channel.
    pub fn set_priority(&mut self, fields: &[EffectField]) -> Result<(), String> {
        if fields.is_empty() {
            self.priority = None;
            self.selected.clear();
            return Ok(());
        }
        if fields.len() != 6 {
            return Err(
                "status priority must contain all six outgoing fields exactly once, or be empty"
                    .into(),
            );
        }
        let mut order = [0; 6];
        let mut seen = BTreeSet::new();
        for (position, field) in fields.iter().enumerate() {
            let index = CHANNELS
                .iter()
                .position(|channel| channel.field == Some(*field))
                .ok_or_else(|| {
                    format!(
                        "{} is not a supported outgoing status priority",
                        field.xml_name()
                    )
                })?;
            if !seen.insert(index) {
                return Err(format!("duplicate status priority: {}", field.xml_name()));
            }
            order[position] = index;
        }
        // Validate before changing current configuration. Selected carriers are
        // reconciled on the next native callback, never from this config setter.
        self.priority = Some(order);
        self.selected.clear();
        Ok(())
    }

    /// Validate the generated graph before advertising weapon_status_impart.
    ///
    /// # Safety
    /// Only on the gated game task with this package loaded, without concurrent
    /// param access. No game pointer or borrowed param survives this call.
    pub unsafe fn ensure_ready(&mut self) -> Result<(), String> {
        self.ready = false;
        let repo = unsafe { SoloParamRepository::instance_mut() }
            .map_err(|e| format!("status repository unavailable: {e}"))?;
        let mut templates = Vec::with_capacity(CHANNEL_COUNT);
        for (index, channel) in CHANNELS.iter().enumerate() {
            let child_id = CHILD_BASE + index as u32;
            let child = repo
                .get::<SpEffectParam>(child_id)
                .ok_or_else(|| format!("owned status child {child_id} is missing"))?;
            if child.state_info() != channel.state
                || child.sp_attribute() != channel.attribute
                || child.sp_category() != channel.category
                || child.wep_param_change() != 3
                || child.effect_target_self_target()
                || !child.effect_target_oppose_target()
                || child.atk_occurrence_sp_effect_id() != -1
            {
                return Err(format!(
                    "owned status child {child_id} has incompatible semantics"
                ));
            }
            for hand in 0..2 {
                let parent_id = PARENT_BASE + index as u32 * 2 + hand;
                let parent = repo
                    .get::<SpEffectParam>(parent_id)
                    .ok_or_else(|| format!("owned status parent {parent_id} is missing"))?;
                if parent.state_info() != 152
                    || parent.sp_category() != 0
                    || parent.wep_param_change() != (hand + 1) as u8
                    || parent.atk_occurrence_sp_effect_id() != child_id as i32
                    || parent.effect_endurance() != -1.0
                    || parent.sp_attribute() != channel.attribute
                    || !parent.effect_target_self_target()
                    || parent.change_hp_point() != 0
                    || parent.change_hp_rate() != 0.0
                    || parent.change_mp_point() != 0
                    || parent.change_mp_rate() != 0.0
                {
                    return Err(format!("owned status carrier {parent_id} is incompatible"));
                }
            }
            templates.push(child.clone());
        }
        // Store owned values only. Repeated readiness checks must not replace
        // the original template with an already adjusted buildup payload.
        if self.child_templates.is_none() {
            self.child_templates = Some(templates);
        }
        if self.native_templates.is_none() {
            // Optional layer: without these rows only the carriers are used.
            let natives: Option<Vec<SP_EFFECT_PARAM_ST>> = (0..CHANNEL_COUNT)
                .map(|index| {
                    repo.get::<SpEffectParam>(NATIVE_BASE + index as u32)?;
                    repo.get::<SpEffectParam>(NATIVE_TEMPLATES[index]).cloned()
                })
                .collect();
            self.native_templates = natives;
        }
        self.ready = true;
        Ok(())
    }

    /// Update finite selected buildup values, then reconcile the owned carriers.
    /// Positive requested fields determine availability; the flat bonus alone
    /// never creates a status. Configured preference chooses first, independently
    /// of buildup magnitude; remaining channels use their stable prior order.
    ///
    /// # Safety
    /// Native callback only, offline, pinned executable and generated params.
    /// No filesystem work, enemy pointer walk, or direct HP/status gauge write.
    /// 0.7.1: every requested status is applied at once (`channels` is only
    /// a ceiling, normally 7); `potency` multiplies the card buildup (Mind,
    /// character level, NG+), before the Arcane flat bonus.
    pub unsafe fn reconcile(
        &mut self,
        requested: &BTreeMap<EffectField, f32>,
        channels: u8,
        flat_bonus: i32,
        potency: f32,
        weapon_boost: f32,
    ) -> Result<StatusSummary, String> {
        let weapon_boost = if weapon_boost.is_finite() { weapon_boost.clamp(0.0, 2.0) } else { 0.0 };
        let potency = if potency.is_finite() { potency.clamp(0.5, 4.0) } else { 1.0 };
        let game = unsafe { GameMan::instance() }
            .map_err(|e| format!("status game state unavailable: {e}"))?;
        if !crate::game::actual_offline(game) {
            unsafe {
                self.clear()?;
            }
            return Err("weapon status impart requires offline mode".into());
        }
        if let Err(error) = unsafe { self.ensure_ready() } {
            unsafe {
                self.clear()?;
            }
            return Err(error);
        }
        let world = unsafe { WorldChrMan::instance_mut() }
            .map_err(|e| format!("status player world unavailable: {e}"))?;
        let Some(player) = world.main_player.as_mut() else {
            self.forget();
            return Ok(StatusSummary::default());
        };
        if player.chr_ins.modules.data.hp <= 0 {
            for id in PARENT_BASE..PARENT_BASE + (CHANNEL_COUNT * 2) as u32 {
                player.chr_ins.remove_speffect(id as i32);
            }
            self.forget();
            return Ok(StatusSummary::default());
        }
        let data = unsafe { player.player_game_data.as_ref() };
        if !unsafe { crate::game::native_offline_host(game, data) } {
            for id in PARENT_BASE..PARENT_BASE + (CHANNEL_COUNT * 2) as u32 {
                player.chr_ins.remove_speffect(id as i32);
            }
            self.forget();
            return Err("weapon status target must be the loaded local world's player".into());
        }
        let assembly = &data.equipment.chr_asm;
        let ids = &assembly.equipment_param_ids;
        let weapons: [i32; 6] = [ids[0], ids[1], ids[2], ids[3], ids[4], ids[5]];
        let held = [
            weapons[(assembly.equipment.selected_slots.right_weapon_slot.min(2) * 2 + 1) as usize],
            weapons[(assembly.equipment.selected_slots.left_weapon_slot.min(2) * 2) as usize],
        ];
        let key = OwnerKey {
            handle: player.chr_ins.field_ins_handle,
            equipment: [ids[0], ids[1], ids[2], ids[3], ids[4], ids[5]],
            selected_left: assembly.equipment.selected_slots.left_weapon_slot,
            selected_right: assembly.equipment.selected_slots.right_weapon_slot,
            arm_style: assembly.equipment.arm_style as u32,
        };
        let changed_owner = self.owner.as_ref() != Some(&key);
        if changed_owner {
            // Rebind after a hand/weapon/affinity/load change. Never touch the
            // vanilla grease IDs or change EquipParamWeapon.isEnhance.
            for id in PARENT_BASE..PARENT_BASE + (CHANNEL_COUNT * 2) as u32 {
                player.chr_ins.remove_speffect(id as i32);
            }
            self.owner = Some(key);
            self.written_buildup.clear();
        }
        let mut available = BTreeMap::new();
        let mut summary = StatusSummary::default();
        for (index, channel) in CHANNELS.iter().enumerate() {
            let Some(field) = channel.field else {
                continue;
            };
            let Some(value) = requested.get(&field).copied() else {
                continue;
            };
            if !value.is_finite() {
                summary
                    .warnings
                    .push(format!("{} ignored: nonfinite buildup", channel.name));
                continue;
            }
            if value <= 0.0 {
                continue;
            }
            let base = (value * potency).clamp(0.0, MAX_BUILDUP as f32).round() as i32;
            let buildup = base
                .saturating_add(flat_bonus.clamp(0, 100))
                .clamp(1, MAX_BUILDUP);
            available.insert(index, buildup);
        }
        let budget = usize::from(channels).min(CHANNEL_COUNT);
        let priority = self.priority.unwrap_or([0, 1, 2, 3, 4, 5]);
        let mut selected = Vec::with_capacity(budget);
        for index in priority {
            if selected.len() >= budget {
                break;
            }
            if available.contains_key(&index) {
                selected.push(index);
            }
        }
        // Stable fallback for additional native channels introduced by a future
        // request API; the six configured channels always receive preference.
        for &index in &self.selected {
            if selected.len() >= budget {
                break;
            }
            if available.contains_key(&index) && !selected.contains(&index) {
                selected.push(index);
            }
        }
        for index in 0..CHANNEL_COUNT {
            if selected.len() >= budget {
                break;
            }
            if available.contains_key(&index) && !selected.contains(&index) {
                selected.push(index);
            }
        }
        self.selected = selected;
        if available.len() > budget {
            summary.warnings.push(format!(
                "{} status channels requested, {budget} applied",
                available.len()
            ));
        }

        let live: BTreeSet<u32> = player
            .chr_ins
            .special_effect
            .entries()
            .filter_map(|entry| u32::try_from(entry.param_id).ok())
            .collect();
        // Native weapon passives first; a hand only keeps its carrier for a
        // channel its held weapon could not take.
        let chosen: Vec<(usize, i32)> = self.selected.iter().map(|i| (*i, available[i])).collect();
        let native = match unsafe { self.place_native(&weapons, &chosen, weapon_boost) } {
            Ok(native) => native,
            Err(error) => {
                summary.warnings.push(format!("weapon passives unavailable: {error}"));
                BTreeMap::new()
            }
        };
        for (row, channels) in &native {
            for index in channels {
                summary.native.push(NativePassive {
                    weapon_row: *row,
                    channel: CHANNELS[*index].name.into(),
                    buildup: available[index],
                });
            }
        }
        let on_held = |hand: usize, index: usize| {
            weapon_row(held[hand]).is_some_and(|row| native.get(&row).is_some_and(|c| c.contains(&index)))
        };
        let desired: BTreeSet<u32> = self
            .selected
            .iter()
            .flat_map(|index| {
                let first = PARENT_BASE + *index as u32 * 2;
                // Parent hand 0 is the right-hand carrier (wepParamChange 1),
                // hand 1 the left (2), like Blood Grease - Right/Left.
                [(first, 0), (first + 1, 1)]
                    .into_iter()
                    .filter(|(_, hand)| !on_held(*hand, *index))
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>()
            })
            .collect();
        for id in PARENT_BASE..PARENT_BASE + (CHANNEL_COUNT * 2) as u32 {
            if !desired.contains(&id) && live.contains(&id) {
                player.chr_ins.remove_speffect(id as i32);
            }
        }
        let repo = unsafe { SoloParamRepository::instance_mut() }
            .map_err(|e| format!("status repository unavailable: {e}"))?;
        let templates = self
            .child_templates
            .as_ref()
            .ok_or_else(|| "owned status templates disappeared".to_owned())?;
        for &index in &self.selected {
            let buildup = available[&index];
            let first = PARENT_BASE + index as u32 * 2;
            let missing = changed_owner || !live.contains(&first) || !live.contains(&(first + 1));
            if missing || self.written_buildup.get(&index) != Some(&buildup) {
                let child_id = CHILD_BASE + index as u32;
                let child = repo
                    .get_mut::<SpEffectParam>(child_id)
                    .ok_or_else(|| format!("owned status payload {child_id} disappeared"))?;
                *child = templates[index].clone();
                set_buildup(child, index, buildup);
                self.written_buildup.insert(index, buildup);
            }
            for parent in [first, first + 1] {
                if !desired.contains(&parent) {
                    continue;
                }
                if changed_owner || !live.contains(&parent) {
                    player.apply_speffect(parent as i32, true);
                }
                summary.applied_rows.push(parent);
            }
            summary.selected_channels.push(CHANNELS[index].name.into());
            summary.buildup.insert(CHANNELS[index].name.into(), buildup);
        }
        self.written_buildup
            .retain(|index, _| self.selected.contains(index));
        Ok(summary)
    }

    /// Remove only the owned carriers from the currently resolved local player.
    /// # Safety
    /// Native callback only. Never call from Drop on a render/worker thread.
    pub unsafe fn clear(&mut self) -> Result<(), String> {
        if let Ok(world) = unsafe { WorldChrMan::instance_mut() } {
            if let Some(player) = world.main_player.as_mut() {
                for id in PARENT_BASE..PARENT_BASE + (CHANNEL_COUNT * 2) as u32 {
                    player.chr_ins.remove_speffect(id as i32);
                }
            }
        }
        let restored = unsafe { self.restore_weapons(&[]) };
        self.forget();
        restored
    }

    /// Put each chosen channel into a free passive slot of every equipped
    /// weapon (only slots whose upgrade offset is 0 at every equipped level, so
    /// the id the engine resolves is exactly ours), add the status text when
    /// the weapon does not already list that status, and restore rows that
    /// are no longer equipped. Returns weapon row -> channels placed.
    unsafe fn place_native(
        &mut self,
        weapons: &[i32; 6],
        chosen: &[(usize, i32)],
        boost: f32,
    ) -> Result<BTreeMap<u32, Vec<usize>>, String> {
        let mut levels: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
        for id in weapons {
            if let Some(row) = weapon_row(*id) {
                levels.entry(row).or_default().push((*id % 100) as u32);
            }
        }
        let equipped: Vec<u32> = levels.keys().copied().collect();
        unsafe { self.restore_weapons(&equipped) }?;
        let Some(templates) = self.native_templates.clone() else {
            return Ok(BTreeMap::new());
        };
        let repo = unsafe { SoloParamRepository::instance_mut() }
            .map_err(|e| format!("weapon repository unavailable: {e}"))?;
        for (index, buildup) in chosen {
            if self.native_written.get(index) != Some(buildup) {
                let row = repo
                    .get_mut::<SpEffectParam>(NATIVE_BASE + *index as u32)
                    .ok_or("native passive row disappeared")?;
                *row = templates[*index].clone();
                set_buildup(row, *index, *buildup);
                self.native_written.insert(*index, *buildup);
            }
        }
        let mut placed_rows = BTreeMap::new();
        // Every equipped row and its combined rows are rewritten in this pass,
        // so pool indices are simply handed out in row order.
        self.combined.clear();
        for (row_id, row_levels) in levels {
            // Originals: from our record if we edited the row before.
            let (original, reinforce) = {
                let Some(weapon) = repo.get::<EquipParamWeapon>(row_id) else {
                    continue;
                };
                let current = WeaponPatch {
                    behavior: [
                        weapon.sp_effect_behavior_id0(),
                        weapon.sp_effect_behavior_id1(),
                        weapon.sp_effect_behavior_id2(),
                    ],
                    text: [
                        weapon.sp_effect_msg_id0(),
                        weapon.sp_effect_msg_id1(),
                        weapon.sp_effect_msg_id2(),
                    ],
                    placed: Vec::new(),
                };
                (
                    self.patches.get(&row_id).cloned().unwrap_or(current),
                    weapon.reinforce_type_id() as i32,
                )
            };
            let offset_free = |slot: usize| {
                row_levels.iter().all(|level| {
                    repo.get::<ReinforceParamWeapon>((reinforce + *level as i32).max(0) as u32)
                        .is_some_and(|r| {
                            [r.sp_effect_id1(), r.sp_effect_id2(), r.sp_effect_id3()][slot] == 0
                        })
                })
            };
            // The id the engine resolves for a slot: value + upgrade offset,
            // if every equipped copy of this row agrees on the offset.
            let resolved = |slot: usize| -> Option<(i32, i32)> {
                let mut offsets = row_levels.iter().map(|level| {
                    repo.get::<ReinforceParamWeapon>((reinforce + *level as i32).max(0) as u32)
                        .map(|r| [r.sp_effect_id1(), r.sp_effect_id2(), r.sp_effect_id3()][slot] as i32)
                });
                let first = offsets.next()??;
                offsets.all(|o| o == Some(first)).then_some((original.behavior[slot] + first, first))
            };
            let mut behavior = original.behavior;
            let mut text = original.text;
            let mut placed = Vec::new();
            let mut taken = [false; 3];
            let mut combined_writes = Vec::new();
            // 1) The weapon already has this status: one combined passive.
            for (index, buildup) in chosen {
                let existing = (0..3).find_map(|slot| {
                    if taken[slot] || original.behavior[slot] <= 0 {
                        return None;
                    }
                    let (id, offset) = resolved(slot)?;
                    let vanilla = repo.get::<SpEffectParam>(u32::try_from(id).ok()?)?;
                    (vanilla.state_info() == CHANNELS[*index].state).then(|| (slot, offset, vanilla.clone()))
                });
                let Some((slot, offset, vanilla)) = existing else {
                    continue;
                };
                let next = self.combined.len() as u32;
                let pool = match self.combined.get(&(row_id, *index)) {
                    Some(pool) => *pool,
                    None if next < COMBINED_ROWS => {
                        self.combined.insert((row_id, *index), next);
                        next
                    }
                    None => continue,
                };
                let target = COMBINED_BASE + pool;
                behavior[slot] = target as i32 - offset;
                taken[slot] = true;
                placed.push(*index);
                combined_writes.push((target, vanilla, *index, *buildup));
            }
            // 0.8 Malice: the weapon's own statuses grow too (no card needed).
            if boost > 0.0 {
                for slot in 0..3 {
                    if taken[slot] || original.behavior[slot] <= 0 {
                        continue;
                    }
                    let Some((id, offset)) = resolved(slot) else {
                        continue;
                    };
                    let Some(vanilla) = u32::try_from(id).ok().and_then(|id| repo.get::<SpEffectParam>(id)) else {
                        continue;
                    };
                    let Some(index) = CHANNELS.iter().position(|c| c.state == vanilla.state_info()) else {
                        continue;
                    };
                    if current_buildup(vanilla, index) <= 0 {
                        continue;
                    }
                    let next = self.combined.len() as u32;
                    if next >= COMBINED_ROWS {
                        break;
                    }
                    self.combined.insert((row_id, index), next);
                    let target = COMBINED_BASE + next;
                    behavior[slot] = target as i32 - offset;
                    taken[slot] = true;
                    combined_writes.push((target, vanilla.clone(), index, 0));
                }
            }
            // 2) A status the weapon lacks: a free slot plus its text line.
            let free: Vec<usize> = (0..3)
                .filter(|slot| original.behavior[*slot] == -1 && offset_free(*slot))
                .collect();
            let mut slots = free.into_iter();
            for (index, _) in chosen {
                if placed.contains(index) {
                    continue;
                }
                let Some(slot) = slots.next() else {
                    break;
                };
                behavior[slot] = (NATIVE_BASE + *index as u32) as i32;
                placed.push(*index);
                let (plain, first, last) = STATUS_TEXT[*index];
                if !text.iter().any(|t| (first..=last).contains(t)) {
                    if let Some(free_text) = (0..3).find(|j| text[*j] == -1) {
                        text[free_text] = plain;
                    }
                }
            }
            for (target, mut vanilla, index, buildup) in combined_writes {
                let own = (current_buildup(&vanilla, index) as f32 * (1.0 + boost)).round() as i32;
                let total = (own + buildup).clamp(1, MAX_BUILDUP);
                set_buildup(&mut vanilla, index, total);
                if let Some(row) = repo.get_mut::<SpEffectParam>(target) {
                    *row = vanilla;
                }
            }
            let weapon = repo
                .get_mut::<EquipParamWeapon>(row_id)
                .ok_or("weapon row disappeared")?;
            write_slots(weapon, behavior, text);
            if behavior == original.behavior && text == original.text {
                self.patches.remove(&row_id);
            } else {
                self.patches.insert(
                    row_id,
                    WeaponPatch {
                        behavior: original.behavior,
                        text: original.text,
                        placed: placed.clone(),
                    },
                );
            }
            if !placed.is_empty() {
                placed_rows.insert(row_id, placed);
            }

        }
        Ok(placed_rows)
    }

    /// Restore every edited weapon row except `keep` (all of them for []).
    unsafe fn restore_weapons(&mut self, keep: &[u32]) -> Result<(), String> {
        if self.patches.is_empty() {
            return Ok(());
        }
        let repo = unsafe { SoloParamRepository::instance_mut() }
            .map_err(|e| format!("weapon repository unavailable: {e}"))?;
        let rows: Vec<u32> = self
            .patches
            .keys()
            .copied()
            .filter(|row| !keep.contains(row))
            .collect();
        for row in rows {
            if let Some(patch) = self.patches.remove(&row) {
                if let Some(weapon) = repo.get_mut::<EquipParamWeapon>(row) {
                    write_slots(weapon, patch.behavior, patch.text);
                }
            }
        }
        Ok(())
    }

    fn forget(&mut self) {
        self.owner = None;
        self.selected.clear();
        self.written_buildup.clear();
    }
}

/// EquipParamWeapon row of an equipped item id (level in the last two digits).
fn weapon_row(id: i32) -> Option<u32> {
    if id <= 0 {
        return None;
    }
    let row = (id - id % 100) as u32;
    (row != UNARMED_ROW).then_some(row)
}

fn write_slots(
    weapon: &mut eldenring::param::EQUIP_PARAM_WEAPON_ST,
    behavior: [i32; 3],
    text: [i32; 3],
) {
    if weapon.sp_effect_behavior_id0() != behavior[0] {
        weapon.set_sp_effect_behavior_id0(behavior[0]);
    }
    if weapon.sp_effect_behavior_id1() != behavior[1] {
        weapon.set_sp_effect_behavior_id1(behavior[1]);
    }
    if weapon.sp_effect_behavior_id2() != behavior[2] {
        weapon.set_sp_effect_behavior_id2(behavior[2]);
    }
    if weapon.sp_effect_msg_id0() != text[0] {
        weapon.set_sp_effect_msg_id0(text[0]);
    }
    if weapon.sp_effect_msg_id1() != text[1] {
        weapon.set_sp_effect_msg_id1(text[1]);
    }
    if weapon.sp_effect_msg_id2() != text[2] {
        weapon.set_sp_effect_msg_id2(text[2]);
    }
}

fn current_buildup(row: &SP_EFFECT_PARAM_ST, channel: usize) -> i32 {
    match channel {
        0 => row.blood_attack_power(),
        1 => row.freeze_attack_power(),
        2 => row.poizon_attack_power(),
        3 => row.disease_attack_power(),
        4 => row.sleep_attack_power(),
        5 => row.madness_attack_power(),
        6 => row.curse_attack_power(),
        _ => 0,
    }
}

fn set_buildup(row: &mut SP_EFFECT_PARAM_ST, channel: usize, value: i32) {
    match channel {
        0 => row.set_blood_attack_power(value),
        1 => row.set_freeze_attack_power(value),
        2 => row.set_poizon_attack_power(value),
        3 => row.set_disease_attack_power(value),
        4 => row.set_sleep_attack_power(value),
        5 => row.set_madness_attack_power(value),
        6 => row.set_curse_attack_power(value),
        _ => unreachable!("status channel is bounded by the static channel table"),
    }
}

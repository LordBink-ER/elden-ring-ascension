//! 0.5 "Fate": NG+ boss affixes (which ones a cycle gets) and the native
//! helpers for Blood Price (HP paid for FP) and runes paid for a skipped
//! reward. The affix choice is pure and stable per character and cycle.
use crate::config::FateSettings;

/// Owned SpEffect rows, one per affix (95_030_000..003 are the boss tiers).
pub const AFFIX_ROW_BASE: u32 = 95_030_004;
/// 0.6 Adaptive: one row per damage kind it resists (physical, magic, fire,
/// lightning, holy); 95_030_009 doubles as Adaptive's own `row()`.
pub const ADAPTIVE_ROW_BASE: u32 = 95_030_009;
pub const ELEMENTS: [&str; 5] = ["physical", "magic", "fire", "lightning", "holy"];
/// Every owned affix row (for clean-up).
pub fn affix_rows() -> impl Iterator<Item = u32> {
    AFFIX_ROW_BASE..ADAPTIVE_ROW_BASE + ELEMENTS.len() as u32
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BossAffix {
    Relentless,
    Ironclad,
    Spellward,
    Frenzied,
    Regenerating,
    /// 0.6: resists the main damage kind of the player's weapon.
    Adaptive,
}
pub const BOSS_AFFIXES: [BossAffix; 6] = [
    BossAffix::Relentless,
    BossAffix::Ironclad,
    BossAffix::Spellward,
    BossAffix::Frenzied,
    BossAffix::Regenerating,
    BossAffix::Adaptive,
];
impl BossAffix {
    pub fn name(self) -> &'static str {
        match self {
            Self::Relentless => "Relentless",
            Self::Ironclad => "Ironclad",
            Self::Spellward => "Spellward",
            Self::Frenzied => "Frenzied",
            Self::Regenerating => "Regenerating",
            Self::Adaptive => "Adaptive",
        }
    }
    /// One line for the HUD and the statistics tab (UI labels are English).
    pub fn summary(self) -> &'static str {
        match self {
            Self::Relentless => "40% harder to stagger",
            Self::Ironclad => "takes 15% less physical damage",
            Self::Spellward => "takes 15% less magic, fire, lightning and holy damage",
            Self::Frenzied => "deals 12% more damage",
            Self::Regenerating => "heals 0.25% HP per second",
            Self::Adaptive => "takes 25% less of your main damage type",
        }
    }
    pub fn row(self) -> u32 {
        AFFIX_ROW_BASE + self as u32
    }
    /// The row this affix applies when the player's weapon deals mostly
    /// `element` (index into ELEMENTS); only Adaptive depends on it.
    pub fn row_for(self, element: usize) -> u32 {
        match self {
            Self::Adaptive => ADAPTIVE_ROW_BASE + element.min(ELEMENTS.len() - 1) as u32,
            _ => self.row(),
        }
    }
}

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}
/// FNV-1a of the character key: a stable seed for this character's affixes.
pub fn seed(character_key: &str) -> u64 {
    character_key
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |h, b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
}

/// Affixes every boss of this NG+ cycle carries: none before
/// `boss_affix_from_cycle`, one per cycle, two from NG+3. Consecutive cycles
/// never repeat the same first affix.
pub fn boss_affixes(cycle: u32, seed: u64, settings: &FateSettings) -> Vec<BossAffix> {
    if !settings.boss_affixes || cycle == 0 || cycle < settings.boss_affix_from_cycle.max(1) {
        return Vec::new();
    }
    let n = BOSS_AFFIXES.len() as u64;
    let start = mix(seed) % n;
    // Walk the five affixes in a per-character order, one step per cycle.
    let step = 1 + mix(seed ^ 0x5eed) % (n - 1);
    let first = (start + step * u64::from(cycle - 1)) % n;
    let mut result = vec![BOSS_AFFIXES[first as usize]];
    if cycle >= 3 {
        let second = (first + 1 + mix(seed ^ u64::from(cycle)) % (n - 1)) % n;
        result.push(BOSS_AFFIXES[second as usize]);
    }
    result
}

/// 3.0 Ascended Bosses: from Tide I, a boss may rise with its own affixes.
/// The chance grows with the Tide (20% at I, +6% per step, +10% for Major
/// and Remembrance bosses, at most 80%); a second affix can come from Tide
/// III, a third from Tide VI. Stable per character, boss and cycle, so a
/// reload never rerolls it. `tier` is the boss scaler's 0..=3.
pub fn ascended_affixes(seed: u64, boss: i32, cycle: u32, tide: u8, tier: u8) -> Vec<BossAffix> {
    if tide == 0 {
        return Vec::new();
    }
    let roll = |salt: u64| (mix(seed ^ mix(boss as u64 ^ (u64::from(cycle) << 40)) ^ salt) % 10_000) as f64 / 10_000.0;
    let chance = (0.20 + 0.06 * f64::from(tide.saturating_sub(1)) + if tier >= 2 { 0.10 } else { 0.0 }).min(0.80);
    if roll(1) >= chance {
        return Vec::new();
    }
    let count = 1 + usize::from(tide >= 3 && roll(2) < 0.45) + usize::from(tide >= 6 && roll(3) < 0.35);
    let n = BOSS_AFFIXES.len() as u64;
    let mut pool: Vec<BossAffix> = BOSS_AFFIXES.to_vec();
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        let pick = (mix(seed ^ boss as u64 ^ (0xa5 + k as u64)) % (n - k as u64)) as usize;
        out.push(pool.remove(pick));
    }
    out.sort();
    out
}

/// Native helpers (main game task only).
#[cfg(windows)]
pub mod native {
    use eldenring::cs::WorldChrMan;
    use fromsoftware_shared::FromStatic;

    /// Blood Price: take `cost` HP from the player, never below 1 HP.
    /// Returns the HP actually taken.
    ///
    /// # Safety
    /// Main game task, verified offline local host.
    pub unsafe fn drain_hp(cost: i32) -> Result<i32, String> {
        let world = unsafe { WorldChrMan::instance_mut() }.map_err(|e| e.to_string())?;
        let Some(player) = world.main_player.as_mut() else {
            return Ok(0);
        };
        let data = &mut player.chr_ins.modules.data;
        if data.hp <= 1 || cost <= 0 {
            return Ok(0);
        }
        let taken = cost.min(data.hp - 1);
        data.hp -= taken;
        Ok(taken)
    }

    /// 0.8: take up to `amount` runes from the purse for bought Ascension
    /// points. Returns the runes actually taken (never more than held).
    ///
    /// # Safety
    /// Main game task, verified offline local host.
    pub unsafe fn take_runes(amount: u32) -> Result<u32, String> {
        let world = unsafe { WorldChrMan::instance_mut() }.map_err(|e| e.to_string())?;
        let Some(player) = world.main_player.as_mut() else {
            return Err("no player to charge".into());
        };
        let data = unsafe { player.player_game_data.as_mut() };
        let taken = amount.min(data.rune_count);
        data.rune_count -= taken;
        Ok(taken)
    }

    /// 0.8 lifesteal: restore up to `amount` HP (never above max, never to a
    /// dead player). Returns the HP restored.
    ///
    /// # Safety
    /// Main game task, verified offline local host.
    pub unsafe fn heal_hp(amount: i32) -> Result<i32, String> {
        let world = unsafe { WorldChrMan::instance_mut() }.map_err(|e| e.to_string())?;
        let Some(player) = world.main_player.as_mut() else {
            return Ok(0);
        };
        let data = &mut player.chr_ins.modules.data;
        if data.hp <= 0 || amount <= 0 || data.hp >= data.max_hp {
            return Ok(0);
        }
        let healed = amount.min(data.max_hp - data.hp);
        data.hp += healed;
        Ok(healed)
    }

    /// Pay owed runes into the player's purse (and rune memory).

    ///
    /// # Safety
    /// Main game task, verified offline local host.
    pub unsafe fn grant_runes(amount: u32) -> Result<u32, String> {
        let world = unsafe { WorldChrMan::instance_mut() }.map_err(|e| e.to_string())?;
        let Some(player) = world.main_player.as_mut() else {
            return Err("no player to pay".into());
        };
        let data = unsafe { player.player_game_data.as_mut() };
        let before = data.rune_count;
        data.rune_count = data.rune_count.saturating_add(amount).min(999_999_999);
        data.rune_memory = data.rune_memory.saturating_add(amount);
        Ok(data.rune_count - before)
    }

    /// 0.6 Adaptive: the damage kind (index into ELEMENTS) that dominates the
    /// player's active right-hand weapon (its infusion's base attack).
    ///
    /// # Safety
    /// Main game task, verified offline local host.
    pub unsafe fn weapon_element() -> Option<usize> {
        use eldenring::cs::{ChrAsmHand, EquipParamWeapon, SoloParamRepository};
        let world = unsafe { WorldChrMan::instance() }.ok()?;
        let player = world.main_player.as_ref()?;
        let data = unsafe { player.player_game_data.as_ref() };
        let slot = data.equipment.chr_asm.equipment.active_weapon_slot(ChrAsmHand::Right);
        let id = *data.equipment.chr_asm.equipment_param_ids.get(slot as usize)?;
        if id <= 0 {
            return None;
        }
        let repo = unsafe { SoloParamRepository::instance() }.ok()?;
        let row = repo.get::<EquipParamWeapon>((id as u32 / 100) * 100)?;
        let attack = [
            row.attack_base_physics(),
            row.attack_base_magic(),
            row.attack_base_fire(),
            row.attack_base_thunder(),
            row.attack_base_dark(),
        ];
        let (index, best) = attack.iter().enumerate().max_by_key(|(_, v)| **v)?;
        (*best > 0).then_some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn affixes_follow_cycles() {
        let settings = FateSettings::default();
        assert!(boss_affixes(0, 7, &settings).is_empty());
        assert_eq!(boss_affixes(1, 7, &settings).len(), 1);
        assert_eq!(boss_affixes(3, 7, &settings).len(), 2);
        for cycle in 1..8 {
            let a = boss_affixes(cycle, 7, &settings);
            let b = boss_affixes(cycle + 1, 7, &settings);
            assert_ne!(a[0], b[0]);
            if a.len() == 2 {
                assert_ne!(a[0], a[1]);
            }
        }
    }
}

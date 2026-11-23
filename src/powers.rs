//! Data-driven abilities. All combat fields map to pinned SP_EFFECT_PARAM_ST
//! definitions; an unsupported event or weapon-impart backend excludes a card.
use crate::core::{Catalog, CoreError, MetaEffect, PowerDef, Rarity, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attribute {
    Vigor,
    Mind,
    Endurance,
    Strength,
    Dexterity,
    Intelligence,
    Faith,
    Arcane,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttributeValues {
    pub vigor: u32,
    pub mind: u32,
    pub endurance: u32,
    pub strength: u32,
    pub dexterity: u32,
    pub intelligence: u32,
    pub faith: u32,
    pub arcane: u32,
}
impl AttributeValues {
    pub fn get(self, a: Attribute) -> u32 {
        match a {
            Attribute::Vigor => self.vigor,
            Attribute::Mind => self.mind,
            Attribute::Endurance => self.endurance,
            Attribute::Strength => self.strength,
            Attribute::Dexterity => self.dexterity,
            Attribute::Intelligence => self.intelligence,
            Attribute::Faith => self.faith,
            Attribute::Arcane => self.arcane,
        }
    }
}

/// Values are absolute param-field values, not percentages added to an
/// existing vanilla row. Each owned ability has dedicated, neutralized rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectField {
    MaxHpRate,
    MaxMpRate,
    MaxStaminaRate,
    SlashDamageCutRate,
    BlowDamageCutRate,
    ThrustDamageCutRate,
    NeutralDamageCutRate,
    MagicDamageCutRate,
    FireDamageCutRate,
    ThunderDamageCutRate,
    DarkDamageCutRate,
    PhysicsAttackRate,
    MagicAttackRate,
    FireAttackRate,
    ThunderAttackRate,
    DarkAttackRate,
    ThrustAttackRate,
    StaminaAttackRate,
    SaAttackPowerRate,
    ToughnessDamageCutRate,
    ChangeSaRecoveryVelocity,
    ConsumeStaminaRate,
    StaminaRecoverChangeSpeed,
    GuardStaminaCutRate,
    GuardStaminaMult,
    SoulRate,
    EquipWeightChangeRate,
    ItemDropRate,
    SightSearchEnemyRate,
    HearingSearchEnemyRate,
    FallDamageRate,
    ArtsConsumptionRate,
    MagicConsumptionRate,
    MiracleConsumptionRate,
    MagicEffectTimeChange,
    HpRecoverRate,
    ChangeHpEstusFlaskCorrectRate,
    ChangeMpEstusFlaskCorrectRate,
    ChangeHpRate,
    ChangeHpPoint,
    ChangeMpRate,
    ChangeMpPoint,
    MotionInterval,
    RegistPoizonChangeRate,
    RegistDiseaseChangeRate,
    RegistBloodChangeRate,
    RegistCurseChangeRate,
    RegistFreezeChangeRate,
    RegistSleepChangeRate,
    RegistMadnessChangeRate,
    PoisonDefDamageRate,
    DiseaseDefDamageRate,
    PoizonAttackPower,
    DiseaseAttackPower,
    BloodAttackPower,
    FreezeAttackPower,
    SleepAttackPower,
    MadnessAttackPower,
    DexterityCancelSystemOnlyAddDexterity,
    /// 0.8: Death Blight buildup on the weapon (Malice 30).
    CurseAttackPower,
    SlashAttackRate,
    BlowAttackRate,
    AddVigor,
    AddMind,
    AddEndurance,
    AddStrength,
    AddDexterity,
    AddIntelligence,
    AddFaith,
    AddArcane,
}
impl EffectField {
    pub fn xml_name(self) -> &'static str {
        use EffectField::*;
        match self {
            MaxHpRate => "maxHpRate",
            MaxMpRate => "maxMpRate",
            MaxStaminaRate => "maxStaminaRate",
            SlashDamageCutRate => "slashDamageCutRate",
            BlowDamageCutRate => "blowDamageCutRate",
            ThrustDamageCutRate => "thrustDamageCutRate",
            NeutralDamageCutRate => "neutralDamageCutRate",
            MagicDamageCutRate => "magicDamageCutRate",
            FireDamageCutRate => "fireDamageCutRate",
            ThunderDamageCutRate => "thunderDamageCutRate",
            DarkDamageCutRate => "darkDamageCutRate",
            PhysicsAttackRate => "physicsAttackRate",
            MagicAttackRate => "magicAttackRate",
            FireAttackRate => "fireAttackRate",
            ThunderAttackRate => "thunderAttackRate",
            DarkAttackRate => "darkAttackRate",
            ThrustAttackRate => "thrustAttackRate",
            StaminaAttackRate => "staminaAttackRate",
            SaAttackPowerRate => "saAttackPowerRate",
            ToughnessDamageCutRate => "toughnessDamageCutRate",
            ChangeSaRecoveryVelocity => "changeSaRecoveryVelocity",
            ConsumeStaminaRate => "consumeStaminaRate",
            StaminaRecoverChangeSpeed => "staminaRecoverChangeSpeed",
            GuardStaminaCutRate => "guardStaminaCutRate",
            GuardStaminaMult => "guardStaminaMult",
            SoulRate => "soulRate",
            EquipWeightChangeRate => "equipWeightChangeRate",
            ItemDropRate => "itemDropRate",
            SightSearchEnemyRate => "sightSearchEnemyRate",
            HearingSearchEnemyRate => "hearingSearchEnemyRate",
            FallDamageRate => "fallDamageRate",
            ArtsConsumptionRate => "artsConsumptionRate",
            MagicConsumptionRate => "magicConsumptionRate",
            MiracleConsumptionRate => "miracleConsumptionRate",
            MagicEffectTimeChange => "magicEffectTimeChange",
            HpRecoverRate => "hpRecoverRate",
            ChangeHpEstusFlaskCorrectRate => "changeHpEstusFlaskCorrectRate",
            ChangeMpEstusFlaskCorrectRate => "changeMpEstusFlaskCorrectRate",
            ChangeHpRate => "changeHpRate",
            ChangeHpPoint => "changeHpPoint",
            ChangeMpRate => "changeMpRate",
            ChangeMpPoint => "changeMpPoint",
            MotionInterval => "motionInterval",
            RegistPoizonChangeRate => "registPoizonChangeRate",
            RegistDiseaseChangeRate => "registDiseaseChangeRate",
            RegistBloodChangeRate => "registBloodChangeRate",
            RegistCurseChangeRate => "registCurseChangeRate",
            RegistFreezeChangeRate => "registFreezeChangeRate",
            RegistSleepChangeRate => "registSleepChangeRate",
            RegistMadnessChangeRate => "registMadnessChangeRate",
            PoisonDefDamageRate => "poisonDefDamageRate",
            DiseaseDefDamageRate => "diseaseDefDamageRate",
            PoizonAttackPower => "poizonAttackPower",
            DiseaseAttackPower => "diseaseAttackPower",
            BloodAttackPower => "bloodAttackPower",
            FreezeAttackPower => "freezeAttackPower",
            SleepAttackPower => "sleepAttackPower",
            MadnessAttackPower => "madnessAttackPower",
            CurseAttackPower => "curseAttackPower",
            DexterityCancelSystemOnlyAddDexterity => "dexterityCancelSystemOnlyAddDexterity",
            SlashAttackRate => "slashAttackRate",
            BlowAttackRate => "blowAttackRate",
            AddVigor => "addLifeForceStatus",
            AddMind => "addWillpowerStatus",
            AddEndurance => "addEndureStatus",
            AddStrength => "addStrengthStatus",
            AddDexterity => "addDexterityStatus",
            AddIntelligence => "addMagicStatus",
            AddFaith => "addFaithStatus",
            AddArcane => "addLuckStatus",
        }
    }
    /// Great-Rune style attribute points (signed byte fields, additive).
    pub fn is_attribute_bonus(self) -> bool {
        use EffectField::*;
        matches!(
            self,
            AddVigor
                | AddMind
                | AddEndurance
                | AddStrength
                | AddDexterity
                | AddIntelligence
                | AddFaith
                | AddArcane
        )
    }
    pub fn is_additive(self) -> bool {
        use EffectField::*;
        matches!(
            self,
            StaminaRecoverChangeSpeed
                | ItemDropRate
                | MagicEffectTimeChange
                | ChangeHpRate
                | ChangeHpPoint
                | ChangeMpRate
                | ChangeMpPoint
                | PoizonAttackPower
                | DiseaseAttackPower
                | BloodAttackPower
                | FreezeAttackPower
                | SleepAttackPower
                | MadnessAttackPower
                | CurseAttackPower
                | DexterityCancelSystemOnlyAddDexterity
        ) || self.is_attribute_bonus()
    }
    pub fn is_integer(self) -> bool {
        use EffectField::*;
        matches!(
            self,
            StaminaRecoverChangeSpeed
                | ChangeHpPoint
                | ChangeMpPoint
                | PoizonAttackPower
                | DiseaseAttackPower
                | BloodAttackPower
                | FreezeAttackPower
                | SleepAttackPower
                | MadnessAttackPower
                | CurseAttackPower
                | DexterityCancelSystemOnlyAddDexterity
        ) || self.is_attribute_bonus()
    }
    pub fn requires_capability(self) -> &'static str {
        use EffectField::*;
        match self {
            PoizonAttackPower | DiseaseAttackPower | BloodAttackPower | FreezeAttackPower
            | SleepAttackPower | MadnessAttackPower | CurseAttackPower => "weapon_status_impart",
            ChangeHpRate | ChangeHpPoint | ChangeMpRate | ChangeMpPoint => "periodic_resources",
            DexterityCancelSystemOnlyAddDexterity => "virtual_dexterity",
            _ => "sp_effect_params",
        }
    }
    pub fn neutral(self) -> f32 {
        if self.is_additive() { 0.0 } else { 1.0 }
    }
    /// Hard global stacking bounds apply after combining all owned sources.
    pub fn bounds(self) -> (f32, f32) {
        use EffectField::*;
        match self {
            ChangeHpRate | ChangeMpRate => (-3.0, 0.0),
            ChangeHpPoint => (-60.0, 0.0),
            ChangeMpPoint => (-15.0, 0.0),
            // 2.2: +50 so a perfect-dodge burst on top of Endurance still counts.
            StaminaRecoverChangeSpeed => (0.0, 50.0),
            ItemDropRate => (0.0, 75.0),
            MagicEffectTimeChange => (0.0, 30.0),
            // 3.4: several status cards stack, up to 180 per hit before
            // Arcane (status.rs caps the result at 300): a status pops every
            // few hits, never on every swing.
            PoizonAttackPower | DiseaseAttackPower | BloodAttackPower | FreezeAttackPower
            | SleepAttackPower | MadnessAttackPower | CurseAttackPower => (0.0, 180.0),
            DexterityCancelSystemOnlyAddDexterity => (0.0, 20.0),
            AddVigor | AddMind | AddEndurance | AddStrength | AddDexterity | AddIntelligence
            | AddFaith | AddArcane => (0.0, 40.0),
            SlashDamageCutRate | BlowDamageCutRate | ThrustDamageCutRate | NeutralDamageCutRate
            | MagicDamageCutRate | FireDamageCutRate | ThunderDamageCutRate | DarkDamageCutRate => {
                (0.45, 2.5)
            }
            ToughnessDamageCutRate => (0.5, 2.0),
            ConsumeStaminaRate => (0.45, 2.0),
            GuardStaminaMult => (0.4, 2.0),
            GuardStaminaCutRate => (0.5, 1.5),
            ArtsConsumptionRate | MagicConsumptionRate | MiracleConsumptionRate => (0.35, 2.5),
            SightSearchEnemyRate | HearingSearchEnemyRate => (0.35, 2.0),
            FallDamageRate => (0.25, 1.0),
            MaxHpRate | MaxMpRate | MaxStaminaRate | EquipWeightChangeRate => (1.0, 2.0),
            // 0.8: the world (x1.5, the Tide) and Fortune pay more runes.
            SoulRate => (1.0, 5.0),
            HpRecoverRate => (1.0, 2.0),
            // 0.5: Withered Flask (curse) lowers flask healing.
            ChangeHpEstusFlaskCorrectRate | ChangeMpEstusFlaskCorrectRate => (0.4, 2.0),
            SaAttackPowerRate | StaminaAttackRate | ChangeSaRecoveryVelocity => (0.5, 2.5),
            MotionInterval => (0.1, 10.0),
            RegistPoizonChangeRate
            | RegistDiseaseChangeRate
            | RegistBloodChangeRate
            | RegistCurseChangeRate
            | RegistFreezeChangeRate
            | RegistSleepChangeRate
            | RegistMadnessChangeRate => (1.0, 2.5),
            PoisonDefDamageRate | DiseaseDefDamageRate => (0.4, 1.0),
            _ => (0.5, 2.5),
        }
    }

    /// The SDK setters are compiled against the pinned game's generated types.
    #[cfg(windows)]
    pub fn write(self, row: &mut eldenring::param::SP_EFFECT_PARAM_ST, value: f32) {
        use EffectField::*;
        match self {
            MaxHpRate => row.set_max_hp_rate(value),
            MaxMpRate => row.set_max_mp_rate(value),
            MaxStaminaRate => row.set_max_stamina_rate(value),
            SlashDamageCutRate => row.set_slash_damage_cut_rate(value),
            BlowDamageCutRate => row.set_blow_damage_cut_rate(value),
            ThrustDamageCutRate => row.set_thrust_damage_cut_rate(value),
            NeutralDamageCutRate => row.set_neutral_damage_cut_rate(value),
            MagicDamageCutRate => row.set_magic_damage_cut_rate(value),
            FireDamageCutRate => row.set_fire_damage_cut_rate(value),
            ThunderDamageCutRate => row.set_thunder_damage_cut_rate(value),
            DarkDamageCutRate => row.set_dark_damage_cut_rate(value),
            PhysicsAttackRate => row.set_physics_attack_rate(value),
            MagicAttackRate => row.set_magic_attack_rate(value),
            FireAttackRate => row.set_fire_attack_rate(value),
            ThunderAttackRate => row.set_thunder_attack_rate(value),
            DarkAttackRate => row.set_dark_attack_rate(value),
            ThrustAttackRate => row.set_thrust_attack_rate(value),
            StaminaAttackRate => row.set_stamina_attack_rate(value),
            SaAttackPowerRate => row.set_sa_attack_power_rate(value),
            ToughnessDamageCutRate => row.set_toughness_damage_cut_rate(value),
            ChangeSaRecoveryVelocity => row.set_change_sa_recovery_velocity(value),
            ConsumeStaminaRate => row.set_consume_stamina_rate(value),
            StaminaRecoverChangeSpeed => row.set_stamina_recover_change_speed(value.round() as i32),
            GuardStaminaCutRate => row.set_guard_stamina_cut_rate(value),
            GuardStaminaMult => row.set_guard_stamina_mult(value),
            SoulRate => row.set_soul_rate(value),
            EquipWeightChangeRate => row.set_equip_weight_change_rate(value),
            ItemDropRate => row.set_item_drop_rate(value),
            SightSearchEnemyRate => row.set_sight_search_enemy_rate(value),
            HearingSearchEnemyRate => row.set_hearing_search_enemy_rate(value),
            FallDamageRate => row.set_fall_damage_rate(value),
            ArtsConsumptionRate => row.set_arts_consumption_rate(value),
            MagicConsumptionRate => row.set_magic_consumption_rate(value),
            MiracleConsumptionRate => row.set_miracle_consumption_rate(value),
            MagicEffectTimeChange => row.set_magic_effect_time_change(value),
            HpRecoverRate => row.set_hp_recover_rate(value),
            ChangeHpEstusFlaskCorrectRate => row.set_change_hp_estus_flask_correct_rate(value),
            ChangeMpEstusFlaskCorrectRate => row.set_change_mp_estus_flask_correct_rate(value),
            ChangeHpRate => row.set_change_hp_rate(value),
            ChangeHpPoint => row.set_change_hp_point(value.round() as i32),
            ChangeMpRate => row.set_change_mp_rate(value),
            ChangeMpPoint => row.set_change_mp_point(value.round() as i32),
            MotionInterval => row.set_motion_interval(value),
            RegistPoizonChangeRate => row.set_regist_poizon_change_rate(value),
            RegistDiseaseChangeRate => row.set_regist_disease_change_rate(value),
            RegistBloodChangeRate => row.set_regist_blood_change_rate(value),
            RegistCurseChangeRate => row.set_regist_curse_change_rate(value),
            RegistFreezeChangeRate => row.set_regist_freeze_change_rate(value),
            RegistSleepChangeRate => row.set_regist_sleep_change_rate(value),
            RegistMadnessChangeRate => row.set_regist_madness_change_rate(value),
            PoisonDefDamageRate => row.set_poison_def_damage_rate(value),
            DiseaseDefDamageRate => row.set_disease_def_damage_rate(value),
            PoizonAttackPower => row.set_poizon_attack_power(value.round() as i32),
            DiseaseAttackPower => row.set_disease_attack_power(value.round() as i32),
            BloodAttackPower => row.set_blood_attack_power(value.round() as i32),
            FreezeAttackPower => row.set_freeze_attack_power(value.round() as i32),
            SleepAttackPower => row.set_sleep_attack_power(value.round() as i32),
            MadnessAttackPower => row.set_madness_attack_power(value.round() as i32),
            CurseAttackPower => row.set_curse_attack_power(value.round() as i32),

            DexterityCancelSystemOnlyAddDexterity => row
                .set_dexterity_cancel_system_only_add_dexterity(
                    value.round().clamp(-128.0, 127.0) as i8
                ),
            SlashAttackRate => row.set_slash_attack_rate(value),
            BlowAttackRate => row.set_blow_attack_rate(value),
            AddVigor => row.set_add_life_force_status(attribute_byte(value)),
            AddMind => row.set_add_willpower_status(attribute_byte(value)),
            AddEndurance => row.set_add_endure_status(attribute_byte(value)),
            AddStrength => row.set_add_strength_status(attribute_byte(value)),
            AddDexterity => row.set_add_dexterity_status(attribute_byte(value)),
            AddIntelligence => row.set_add_magic_status(attribute_byte(value)),
            AddFaith => row.set_add_faith_status(attribute_byte(value)),
            AddArcane => row.set_add_luck_status(attribute_byte(value)),
        }
    }
}

#[cfg(windows)]
fn attribute_byte(value: f32) -> i8 {
    value.round().clamp(0.0, 99.0) as i8
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Modifier {
    pub field: EffectField,
    pub value: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CombatEvent {
    HitLanded,
    Kill,
    PerfectDodge,
    StatusProc,
    CriticalHit,
    GuardCounter,
    PoiseBreak,
    /// 0.4: the player killed an Ascended Elite.
    EliteKill,
    /// 0.6: an enemy hit landed on the player's guard.
    Block,
    /// 0.6: one status of the enemy filled up (per kind below).
    BleedProc,
    FrostbiteProc,
    PoisonProc,
    SleepProc,
    MadnessProc,
    /// 0.7: an enemy attack was parried (its attack broke off in its
    /// parryable window), or a frontal critical without a stance break.
    Parry,
    /// 0.7: a critical from behind the enemy.
    Backstab,
    /// 0.9: an enemy died within 3 s of your critical on it.
    Execution,
    /// 0.9: three kills within 6 s.
    Rampage,
}
/// 0.6: Elden Ring's seven statuses, in the order of the resist module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusKind {
    Poison,
    ScarletRot,
    BloodLoss,
    DeathBlight,
    Frostbite,
    Sleep,
    Madness,
}
pub const STATUS_KINDS: [StatusKind; 7] = [
    StatusKind::Poison,
    StatusKind::ScarletRot,
    StatusKind::BloodLoss,
    StatusKind::DeathBlight,
    StatusKind::Frostbite,
    StatusKind::Sleep,
    StatusKind::Madness,
];
impl StatusKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Poison => "poison",
            Self::ScarletRot => "scarlet rot",
            Self::BloodLoss => "blood loss",
            Self::DeathBlight => "death blight",
            Self::Frostbite => "frostbite",
            Self::Sleep => "sleep",
            Self::Madness => "madness",
        }
    }
    /// The specific event a proc of this status raises (besides StatusProc).
    pub fn event(self) -> Option<CombatEvent> {
        match self {
            Self::Poison | Self::ScarletRot => Some(CombatEvent::PoisonProc),
            Self::BloodLoss => Some(CombatEvent::BleedProc),
            Self::Frostbite => Some(CombatEvent::FrostbiteProc),
            Self::Sleep => Some(CombatEvent::SleepProc),
            Self::Madness => Some(CombatEvent::MadnessProc),
            Self::DeathBlight => None,
        }
    }
}
/// Which enemy status procs a status-triggered card reacts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusFilter {
    Any,
    BloodLoss,
    Frostbite,
    /// Poison or scarlet rot.
    Poison,
    Sleep,
    Madness,
    DeathBlight,
}
impl StatusFilter {
    pub fn matches(self, kind: StatusKind) -> bool {
        match self {
            Self::Any => true,
            Self::BloodLoss => kind == StatusKind::BloodLoss,
            Self::Frostbite => kind == StatusKind::Frostbite,
            Self::Poison => matches!(kind, StatusKind::Poison | StatusKind::ScarletRot),
            Self::Sleep => kind == StatusKind::Sleep,
            Self::Madness => kind == StatusKind::Madness,
            Self::DeathBlight => kind == StatusKind::DeathBlight,
        }
    }
}
impl CombatEvent {
    pub fn capability(self) -> &'static str {
        match self {
            Self::HitLanded => "event.hit_landed",
            Self::Kill => "event.kill",
            Self::PerfectDodge => "event.perfect_dodge",
            Self::StatusProc => "event.status_proc",
            Self::CriticalHit => "event.critical_hit",
            Self::GuardCounter => "event.guard_counter",
            Self::PoiseBreak => "event.poise_break",
            Self::EliteKill => "event.elite_kill",
            Self::Block => "event.block",
            Self::BleedProc
            | Self::FrostbiteProc
            | Self::PoisonProc
            | Self::SleepProc
            | Self::MadnessProc => "event.status_proc",
            Self::Parry => "event.parry",
            Self::Backstab => "event.backstab",
            Self::Execution => "event.execution",
            Self::Rampage => "event.rampage",
        }
    }
}
#[derive(Debug, Clone, Copy, Default)]
pub struct RecentEvent {
    pub age_seconds: f32,
    pub stacks: u16,
}

#[derive(Debug, Clone)]
pub struct CombatSnapshot {
    pub in_game: bool,
    pub hp_ratio: f32,
    pub fp_ratio: f32,
    pub stamina_ratio: f32,
    pub equip_load_ratio: Option<f32>,
    pub seconds_since_hp_loss: Option<f32>,
    pub boss_fight_active: bool,
    pub dodging: bool,
    pub guarding: bool,
    pub airborne: bool,
    pub heavy_load: bool,
    pub seconds_since_dodge_end: Option<f32>,
    pub consecutive_hits: u16,
    pub seconds_in_combat: f32,
    pub attributes: AttributeValues,
    pub events: BTreeMap<CombatEvent, RecentEvent>,
    /// 2.0: Path points (PATHS order) and the Tide.
    pub paths: crate::paths::PathPoints,
    pub tide: u8,
    /// 0.9: allies (not your mount) within 15 m.
    pub allies_near: u8,
}
impl Default for CombatSnapshot {
    fn default() -> Self {
        Self {
            in_game: false,
            hp_ratio: 1.0,
            fp_ratio: 1.0,
            stamina_ratio: 1.0,
            equip_load_ratio: None,
            seconds_since_hp_loss: None,
            boss_fight_active: false,
            dodging: false,
            guarding: false,
            airborne: false,
            heavy_load: false,
            seconds_since_dodge_end: None,
            consecutive_hits: 0,
            seconds_in_combat: 0.0,
            attributes: AttributeValues::default(),
            events: BTreeMap::new(),
            paths: [0; 9],
            tide: 0,
            allies_near: 0,
        }
    }
}


#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Activation {
    Always,
    HpBelow {
        ratio: f32,
    },
    HpAbove {
        ratio: f32,
    },
    FpBelow {
        ratio: f32,
    },
    FpAbove {
        ratio: f32,
    },
    StaminaBelow {
        ratio: f32,
    },
    StaminaAbove {
        ratio: f32,
    },
    LoadBelow {
        ratio: f32,
    },
    AttributeAtLeast {
        attribute: Attribute,
        value: u32,
    },
    NoHpLossFor {
        seconds: f32,
    },
    AfterHpLoss {
        seconds: f32,
    },
    AfterEvent {
        event: CombatEvent,
        seconds: f32,
        minimum_stacks: u16,
    },
    BossCombat,
    Dodging,
    Guarding,
    Airborne,
    HeavyLoad,
    AfterDodge {
        seconds: f32,
    },
    CombatFor {
        seconds: f32,
    },
    All {
        conditions: Vec<Activation>,
    },
    /// 2.0: a Path with at least this many points (card levels in it).
    PathAtLeast {
        path: crate::paths::Path,
        value: u8,
    },
    /// 0.8: this many confirmed hits within the streak window.
    Streak {
        hits: u16,
    },
    /// 0.9: an ally (spirit summon, NPC; not your mount) within 15 m.
    AllyNear,
}
impl Activation {
    pub fn active(&self, s: &CombatSnapshot) -> bool {
        if !s.in_game || !s.hp_ratio.is_finite() || s.hp_ratio <= 0.0 {
            return false;
        }
        match self {
            Self::Always => true,
            Self::HpBelow { ratio } => s.hp_ratio < *ratio,
            Self::HpAbove { ratio } => s.hp_ratio >= *ratio,
            Self::FpBelow { ratio } => s.fp_ratio.is_finite() && s.fp_ratio < *ratio,
            Self::FpAbove { ratio } => s.fp_ratio.is_finite() && s.fp_ratio >= *ratio,
            Self::StaminaBelow { ratio } => s.stamina_ratio.is_finite() && s.stamina_ratio < *ratio,
            Self::StaminaAbove { ratio } => {
                s.stamina_ratio.is_finite() && s.stamina_ratio >= *ratio
            }
            Self::LoadBelow { ratio } => s
                .equip_load_ratio
                .is_some_and(|v| v.is_finite() && v < *ratio),
            Self::AttributeAtLeast { attribute, value } => s.attributes.get(*attribute) >= *value,
            Self::NoHpLossFor { seconds } => s
                .seconds_since_hp_loss
                .is_some_and(|v| v.is_finite() && v >= *seconds),
            Self::AfterHpLoss { seconds } => s
                .seconds_since_hp_loss
                .is_some_and(|v| v.is_finite() && v >= 0.0 && v < *seconds),
            Self::AfterEvent {
                event,
                seconds,
                minimum_stacks,
            } => s.events.get(event).is_some_and(|e| {
                e.age_seconds.is_finite()
                    && e.age_seconds >= 0.0
                    && e.age_seconds < *seconds
                    && e.stacks >= *minimum_stacks
            }),
            Self::BossCombat => s.boss_fight_active,
            Self::Dodging => s.dodging,
            Self::Guarding => s.guarding,
            Self::Airborne => s.airborne,
            Self::HeavyLoad => s.heavy_load,
            Self::AfterDodge { seconds } => s
                .seconds_since_dodge_end
                .is_some_and(|v| v.is_finite() && v >= 0.0 && v < *seconds),
            Self::CombatFor { seconds } => {
                s.boss_fight_active
                    && s.seconds_in_combat.is_finite()
                    && s.seconds_in_combat >= *seconds
            }
            Self::All { conditions } => conditions.iter().all(|c| c.active(s)),
            Self::PathAtLeast { path, value } => s.paths[path.index()] >= *value,
            Self::Streak { hits } => s.consecutive_hits >= *hits,
            Self::AllyNear => s.allies_near > 0,
        }
    }
    fn capabilities(&self, out: &mut BTreeSet<String>) {
        match self {
            Self::LoadBelow { .. } => {
                out.insert("equip_load_snapshot".into());
            }
            Self::NoHpLossFor { .. }
            | Self::AfterHpLoss { .. }
            | Self::AfterDodge { .. }
            | Self::CombatFor { .. } => {
                out.insert("elapsed_time".into());
            }
            Self::BossCombat => {
                out.insert("boss_combat".into());
            }
            Self::Dodging | Self::Guarding | Self::Airborne | Self::HeavyLoad => {
                out.insert("action_flags".into());
            }
            Self::AfterEvent { event, .. } => {
                out.insert(event.capability().into());
            }
            Self::Streak { .. } => {
                out.insert("event.hit_landed".into());
            }
            Self::AllyNear => {
                out.insert("native_allies".into());
            }
            Self::All { conditions } => {
                for c in conditions {
                    c.capabilities(out);
                }
            }
            Self::HpBelow { .. }
            | Self::HpAbove { .. }
            | Self::FpBelow { .. }
            | Self::FpAbove { .. }
            | Self::StaminaBelow { .. }
            | Self::StaminaAbove { .. } => {
                out.insert("sampled_resources".into());
            }
            _ => {}
        }
    }
    fn validate(&self, depth: usize) -> Result<()> {
        if depth > 4 {
            return Err(CoreError::Invalid(
                "power condition nesting exceeds limit".into(),
            ));
        }
        match self {
            Self::HpBelow { ratio }
            | Self::HpAbove { ratio }
            | Self::FpBelow { ratio }
            | Self::FpAbove { ratio }
            | Self::StaminaBelow { ratio }
            | Self::StaminaAbove { ratio }
            | Self::LoadBelow { ratio }
                if !ratio.is_finite() || !(0.0..=1.0).contains(ratio) =>
            {
                Err(CoreError::Invalid("invalid resource threshold".into()))
            }
            Self::NoHpLossFor { seconds }
            | Self::AfterHpLoss { seconds }
            | Self::AfterDodge { seconds }
            | Self::CombatFor { seconds }
            | Self::AfterEvent { seconds, .. }
                if !seconds.is_finite() || !(0.1..=60.0).contains(seconds) =>
            {
                Err(CoreError::Invalid("invalid event window".into()))
            }
            Self::AfterEvent { minimum_stacks, .. }
                if *minimum_stacks == 0 || *minimum_stacks > 100 =>
            {
                Err(CoreError::Invalid("invalid event stack requirement".into()))
            }
            Self::PathAtLeast { value, .. } if *value == 0 || *value > 99 => {
                Err(CoreError::Invalid("invalid path requirement".into()))
            }
            Self::Streak { hits } if *hits < 2 || *hits > 50 => {
                Err(CoreError::Invalid("invalid streak requirement".into()))
            }
            Self::All { conditions } => {
                if conditions.is_empty() || conditions.len() > 8 {
                    return Err(CoreError::Invalid("invalid condition set".into()));
                }
                for c in conditions {
                    c.validate(depth + 1)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// When a proc power fires. Values grow with levels; the variant never changes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "on", rename_all = "snake_case")]
pub enum ProcTrigger {
    /// Automatically while fighting, at the nearest engaged enemy.
    Interval { seconds: f32 },
    /// A chance on each of the player's hits.
    Hit { chance: f32, cooldown: f32 },
    /// When `hits` hits land within `window` seconds (default 6).
    Streak {
        hits: u16,
        cooldown: f32,
        #[serde(default = "default_streak_window")]
        window: f32,
    },
    /// A chance at the body of an enemy the player killed.
    Kill { chance: f32, cooldown: f32 },
    /// When a dodge roll starts.
    Dodge { cooldown: f32 },
    /// When one blow takes at least `min_share` of max HP.
    Hurt { min_share: f32, cooldown: f32 },
    /// 0.6: a dodge through an enemy attack, at the attacker.
    PerfectDodge { cooldown: f32 },
    /// 0.6: a chance when an attack lands on the player's guard, at the attacker.
    Block { chance: f32, cooldown: f32 },
    /// 0.6: the player's hit right after a block, at its target.
    GuardCounter { cooldown: f32 },
    /// 0.6: a riposte or backstab, at its victim.
    Critical { cooldown: f32 },
    /// 0.6: an enemy's stance (posture) breaks, at that enemy.
    StanceBreak { cooldown: f32 },
    /// 0.6: a status fills up on an enemy, at that enemy.
    Status { status: StatusFilter, cooldown: f32 },
    /// 0.7: the player parried an enemy attack, at that enemy.
    Parry { cooldown: f32 },
    /// 0.7: a backstab, at its victim.
    Backstab { cooldown: f32 },
    /// 0.9: a kill with a critical, at the body.
    Execution { cooldown: f32 },
    /// 0.9: three kills within 6 s, at the last body.
    Rampage { cooldown: f32 },
}
impl ProcTrigger {
    /// 0.8 Resonance: the same trigger with its waiting times scaled by
    /// `factor` (0.4..1), kept inside the bounds the validator accepts.
    pub fn scaled(self, factor: f32) -> Self {
        let f = if factor.is_finite() { factor.clamp(0.4, 1.0) } else { 1.0 };
        let c = |v: f32, lo: f32| (v * f).max(lo);
        match self {
            Self::Interval { seconds } => Self::Interval { seconds: c(seconds, 0.8) },
            Self::Hit { chance, cooldown } => Self::Hit { chance, cooldown: c(cooldown, 0.3) },
            Self::Streak { hits, cooldown, window } => Self::Streak { hits, cooldown: c(cooldown, 1.0), window },
            Self::Kill { chance, cooldown } => Self::Kill { chance, cooldown: c(cooldown, 0.3) },
            Self::Dodge { cooldown } => Self::Dodge { cooldown: c(cooldown, 0.5) },
            Self::Hurt { min_share, cooldown } => Self::Hurt { min_share, cooldown: c(cooldown, 1.0) },
            Self::PerfectDodge { cooldown } => Self::PerfectDodge { cooldown: c(cooldown, 0.5) },
            Self::Block { chance, cooldown } => Self::Block { chance, cooldown: c(cooldown, 0.3) },
            Self::GuardCounter { cooldown } => Self::GuardCounter { cooldown: c(cooldown, 0.5) },
            Self::Critical { cooldown } => Self::Critical { cooldown: c(cooldown, 0.5) },
            Self::StanceBreak { cooldown } => Self::StanceBreak { cooldown: c(cooldown, 0.5) },
            Self::Status { status, cooldown } => Self::Status { status, cooldown: c(cooldown, 0.5) },
            Self::Parry { cooldown } => Self::Parry { cooldown: c(cooldown, 0.5) },
            Self::Backstab { cooldown } => Self::Backstab { cooldown: c(cooldown, 0.5) },
            Self::Execution { cooldown } => Self::Execution { cooldown: c(cooldown, 0.5) },
            Self::Rampage { cooldown } => Self::Rampage { cooldown: c(cooldown, 0.5) },
        }
    }
    fn kind(&self) -> u8 {

        match self {
            Self::Interval { .. } => 0,
            Self::Hit { .. } => 1,
            Self::Streak { .. } => 2,
            Self::Kill { .. } => 3,
            Self::Dodge { .. } => 4,
            Self::Hurt { .. } => 5,
            Self::PerfectDodge { .. } => 6,
            Self::Block { .. } => 7,
            Self::GuardCounter { .. } => 8,
            Self::Critical { .. } => 9,
            Self::StanceBreak { .. } => 10,
            Self::Status { .. } => 11,
            Self::Parry { .. } => 12,
            Self::Backstab { .. } => 13,
            Self::Execution { .. } => 14,
            Self::Rampage { .. } => 15,
        }
    }
    fn validate(&self) -> Result<()> {
        let ok = match *self {
            Self::Interval { seconds } => seconds.is_finite() && (0.8..=120.0).contains(&seconds),
            Self::Hit { chance, cooldown } | Self::Kill { chance, cooldown } => {
                chance.is_finite()
                    && (0.01..=1.0).contains(&chance)
                    && cooldown.is_finite()
                    && (0.3..=120.0).contains(&cooldown)
            }
            Self::Streak {
                hits,
                cooldown,
                window,
            } => {
                (2..=30).contains(&hits)
                    && cooldown.is_finite()
                    && (1.0..=120.0).contains(&cooldown)
                    && window.is_finite()
                    && (2.0..=15.0).contains(&window)
            }
            Self::Dodge { cooldown }
            | Self::PerfectDodge { cooldown }
            | Self::GuardCounter { cooldown }
            | Self::Critical { cooldown }
            | Self::StanceBreak { cooldown }
            | Self::Parry { cooldown }
            | Self::Backstab { cooldown }
            | Self::Execution { cooldown }
            | Self::Rampage { cooldown }
            | Self::Status { cooldown, .. } => {
                cooldown.is_finite() && (0.5..=120.0).contains(&cooldown)

            }
            Self::Block { chance, cooldown } => {
                chance.is_finite()
                    && (0.01..=1.0).contains(&chance)
                    && cooldown.is_finite()
                    && (0.3..=120.0).contains(&cooldown)
            }
            Self::Hurt { min_share, cooldown } => {
                min_share.is_finite()
                    && (0.01..=0.9).contains(&min_share)
                    && cooldown.is_finite()
                    && (1.0..=120.0).contains(&cooldown)
            }
        };
        if ok {
            Ok(())
        } else {
            Err(CoreError::Invalid("proc trigger outside supported bounds".into()))
        }
    }
}
fn one() -> u8 {
    1
}
fn default_streak_window() -> f32 {
    6.0
}
/// A real bullet chain from config/procs.json fired by the proc engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcSpec {
    pub proc: String,
    pub trigger: ProcTrigger,
    /// Damage scale; 1.0 is the reference bolt of the engine's damage curve.
    pub power: f32,
    #[serde(default = "one")]
    pub count: u8,
}
/// Survive one lethal blow at 1 HP, then heal and ward; repeats after cooldown.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterventionSpec {
    pub cooldown: f32,
    pub heal_share: f32,
    pub ward_seconds: f32,
    /// Incoming damage multiplier while the ward lasts.
    pub ward_cut: f32,
}
impl InterventionSpec {
    fn validate(&self) -> Result<()> {
        if !(self.cooldown.is_finite()
            && (30.0..=3600.0).contains(&self.cooldown)
            && self.heal_share.is_finite()
            && (0.05..=1.0).contains(&self.heal_share)
            && self.ward_seconds.is_finite()
            && (0.5..=10.0).contains(&self.ward_seconds)
            && self.ward_cut.is_finite()
            && (0.05..=1.0).contains(&self.ward_cut))
        {
            return Err(CoreError::Invalid("intervention outside supported bounds".into()));
        }
        Ok(())
    }
    /// Best of several sources: shortest cooldown, largest heal and ward.
    pub fn combine(self, other: Self) -> Self {
        Self {
            cooldown: self.cooldown.min(other.cooldown),
            heal_share: self.heal_share.max(other.heal_share),
            ward_seconds: self.ward_seconds.max(other.ward_seconds),
            ward_cut: self.ward_cut.min(other.ward_cut),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerLevel {
    pub description: String,
    pub activation: Activation,
    pub modifiers: Vec<Modifier>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proc: Option<ProcSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intervention: Option<InterventionSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elite_lure: Option<EliteLure>,
    /// 0.5 curse Blood Price: HP paid per FP point spent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blood_price: Option<f32>,
    /// 0.6: flask and item use animations play this much faster (0.02..0.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_speed: Option<f32>,
    /// 0.8: attack animations this much faster (0.01..0.3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attack_speed: Option<f32>,
    /// 0.8: share of the damage you deal that heals you (0.005..0.2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifesteal: Option<f32>,
    /// 0.8: charged / jumping attack and critical damage bonuses (0.01..1.0),
    /// through the talisman engine effects (Axe, Claw, Dagger talismans).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charge_damage: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump_damage: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical_damage: Option<f32>,
    /// 0.9: your allies' (spirit summons, Torrent) damage, HP and damage-taken
    /// reduction (0.005..1.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ally_damage: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ally_hp: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ally_defense: Option<f32>,
    /// 3.2 No Quarter: your blows land on the victim of your critical while
    /// it gets back up (normally invincible), for the first `window` s of
    /// that (10+ = all of it), with `bonus` more damage then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub downed_strike: Option<DownedStrike>,
}

/// 3.2: see `PowerLevel::downed_strike`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownedStrike {
    pub window: f32,
    pub bonus: f32,
}
impl PowerLevel {
    /// 0.8 extra mechanics: (attack speed, lifesteal, charge, jump, critical),
    /// 0.9: then ally damage, ally HP, ally defense.
    pub fn arts(&self) -> [Option<f32>; 8] {
        [
            self.attack_speed,
            self.lifesteal,
            self.charge_damage,
            self.jump_damage,
            self.critical_damage,
            self.ally_damage,
            self.ally_hp,
            self.ally_defense,
        ]
    }
}

/// More Ascended Elites appear (chance x (1 + chance_bonus)) and each one is
/// worth `ember_bonus` more embers.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EliteLure {
    pub chance_bonus: f32,
    pub ember_bonus: u8,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectTrigger {
    #[default]
    Passive,
    OnKill,
    OnKillTimed,
    OnMomentum,
    OnNearbyAffliction,
    OnCriticalBlessing,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Power {
    pub id: String,
    pub name: String,
    pub category: String,
    pub base_rarity: Rarity,
    pub level_rarities: [Rarity; 5],
    pub weight: f64,
    pub description: String,
    pub effect_row_base: u32,
    #[serde(default)]
    pub trigger: EffectTrigger,
    #[serde(default)]
    pub duration_seconds: f32,
    #[serde(default)]
    pub required_capabilities: BTreeSet<String>,
    #[serde(default)]
    pub meta: Option<MetaEffect>,
    /// 0.5 curse cards: the permanent drawback, shown in red on the card.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub curse: Option<String>,
    /// 0.5 evolutions: parent powers and the level each needs (V).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<(String, u8)>,
    /// 2.0: the Path this card belongs to (its levels count as Path points).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<crate::paths::Path>,
    /// Path bonuses: never offered, never in the codex.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// 2.0 signature cards: the bosses whose first defeat always offers it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signature: Vec<String>,
    pub levels: [PowerLevel; 5],
}
impl Power {
    pub fn required(&self) -> BTreeSet<String> {
        let mut c = self.required_capabilities.clone();
        if self.meta.is_some() {
            c.insert("meta_draw".into());
        }
        if self.trigger == EffectTrigger::OnKill {
            c.insert("native_on_kill".into());
        }
        if self.trigger == EffectTrigger::OnKillTimed {
            c.insert("native_timed_on_kill".into());
        }
        if self.trigger == EffectTrigger::OnMomentum {
            c.insert("native_weighted_momentum".into());
        }
        if self.trigger == EffectTrigger::OnNearbyAffliction {
            c.insert("native_nearby_affliction".into());
        }
        if self.trigger == EffectTrigger::OnCriticalBlessing {
            c.insert("native_critical_blessing".into());
        }
        for level in &self.levels {
            level.activation.capabilities(&mut c);
            if level.proc.is_some() {
                c.insert("native_procs".into());
            }
            if level.intervention.is_some() {
                c.insert("native_intervention".into());
            }
            if level.blood_price.is_some() {
                c.insert("native_blood_price".into());
            }
            if level.item_speed.is_some() {
                c.insert("native_item_speed".into());
            }
            if level.attack_speed.is_some() {
                c.insert("native_attack_speed".into());
            }
            if level.lifesteal.is_some() {
                c.insert("native_lifesteal".into());
            }
            if level.charge_damage.is_some() || level.jump_damage.is_some() || level.critical_damage.is_some() {
                c.insert("native_arts".into());
            }
            if level.ally_damage.is_some() || level.ally_hp.is_some() || level.ally_defense.is_some() {
                c.insert("native_allies".into());
            }
            if level.downed_strike.is_some() {
                c.insert("native_downed_strike".into());
            }
            if !level.modifiers.is_empty() {
                c.insert("sp_effect_params".into());
            }
            for m in &level.modifiers {
                let capability = m.field.requires_capability();
                if !(self.trigger == EffectTrigger::OnKill && capability == "periodic_resources") {
                    c.insert(capability.into());
                }
            }
        }
        c
    }
    pub fn core_definition(&self) -> PowerDef {
        PowerDef {
            id: self.id.clone(),
            name: self.name.clone(),
            category: self.category.clone(),
            base_rarity: self.base_rarity,
            level_rarities: self.level_rarities,
            weight: self.weight,
            required_capabilities: self.required(),
            meta: self.meta.clone(),
            requires: self.requires.clone(),
            cursed: self.curse.is_some(),
            signature: self.signature.clone(),
        }
    }
    pub fn is_evolution(&self) -> bool {
        !self.requires.is_empty()
    }
    pub fn row_id(&self, level: u8) -> Option<u32> {
        if (1..=5).contains(&level) {
            self.effect_row_base.checked_add(u32::from(level) - 1)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerLibrary {
    pub schema: u32,
    pub powers: Vec<Power>,
}
#[derive(Debug, Clone)]
pub struct OwnedEffect {
    pub source_id: String,
    pub level: u8,
    pub effect_id: u32,
    pub trigger: EffectTrigger,
    pub duration_seconds: f32,
    pub modifiers: Vec<Modifier>,
}
#[derive(Debug, Clone, Serialize)]
pub struct EffectRow {
    pub id: u32,
    pub name: String,
    pub fields: BTreeMap<String, serde_json::Value>,
}
#[derive(Debug, Clone, Serialize)]
pub struct EffectManifest {
    pub schema: u32,
    pub effects: Vec<EffectRow>,
}

impl PowerLibrary {
    pub fn from_json(source: &str) -> Result<Self> {
        let library: Self = serde_json::from_str(source)?;
        library.validate()?;
        Ok(library)
    }
    pub fn shipped() -> Result<Self> {
        Self::from_json(include_str!("../config/powers.json"))
    }
    pub fn get(&self, id: &str) -> Option<&Power> {
        self.powers.iter().find(|p| p.id == id)
    }
    pub fn catalog(&self, capabilities: BTreeSet<String>) -> Result<Catalog> {
        // 0.8: stat perks, bonds and oaths are never drawn.
        Catalog::new(
            self.powers.iter().filter(|p| !p.hidden).map(Power::core_definition).collect(),
            capabilities,
        )
    }
    pub fn active_effects(
        &self,
        levels: &BTreeMap<String, u8>,
        snapshot: &CombatSnapshot,
        capabilities: &BTreeSet<String>,
    ) -> Vec<OwnedEffect> {
        self.powers
            .iter()
            .filter_map(|p| {
                let level = *levels.get(&p.id)?;
                if !(1..=5).contains(&level) || !p.required().is_subset(capabilities) {
                    return None;
                }
                let spec = &p.levels[usize::from(level - 1)];
                if spec.modifiers.is_empty() || !spec.activation.active(snapshot) {
                    return None;
                }
                Some(OwnedEffect {
                    source_id: p.id.clone(),
                    level,
                    effect_id: p.row_id(level)?,
                    trigger: p.trigger,
                    duration_seconds: p.duration_seconds,
                    modifiers: spec.modifiers.clone(),
                })
            })
            .collect()
    }
    /// 2.3: names of owned cards (Path bonuses included) that react to one
    /// of `events` (an "after ..." condition or a spell on that event), for
    /// the combat labels. `status` narrows status-triggered spells.
    pub fn reacting(&self, levels: &BTreeMap<String, u8>, events: &[CombatEvent], status: Option<StatusKind>) -> Vec<String> {
        fn waits_for(activation: &Activation, events: &[CombatEvent]) -> bool {
            match activation {
                Activation::AfterEvent { event, .. } => events.contains(event),
                Activation::All { conditions } => conditions.iter().any(|c| waits_for(c, events)),
                _ => false,
            }
        }
        let on = |trigger: &ProcTrigger| -> bool {
            let event = match trigger {
                ProcTrigger::PerfectDodge { .. } => CombatEvent::PerfectDodge,
                ProcTrigger::Block { .. } => CombatEvent::Block,
                ProcTrigger::GuardCounter { .. } => CombatEvent::GuardCounter,
                ProcTrigger::Critical { .. } => CombatEvent::CriticalHit,
                ProcTrigger::StanceBreak { .. } => CombatEvent::PoiseBreak,
                ProcTrigger::Parry { .. } => CombatEvent::Parry,
                ProcTrigger::Backstab { .. } => CombatEvent::Backstab,
                ProcTrigger::Execution { .. } => CombatEvent::Execution,
                ProcTrigger::Rampage { .. } => CombatEvent::Rampage,
                ProcTrigger::Status { status: filter, .. } => {
                    return events.contains(&CombatEvent::StatusProc) && status.is_some_and(|k| filter.matches(k));
                }
                _ => return false,
            };
            events.contains(&event)
        };
        self.powers
            .iter()
            .filter_map(|p| {
                let level = *levels.get(&p.id)?;
                let spec = p.levels.get(usize::from(level.clamp(1, 5) - 1))?;
                (waits_for(&spec.activation, events) || spec.proc.as_ref().is_some_and(|proc| on(&proc.trigger)))
                    .then(|| p.name.clone())
            })
            .collect()
    }
    /// 3.2: the best No Quarter you own (longest window, biggest bonus).
    pub fn downed_strike(&self, levels: &BTreeMap<String, u8>, capabilities: &BTreeSet<String>) -> Option<DownedStrike> {
        self.powers
            .iter()
            .filter(|p| p.required().is_subset(capabilities))
            .filter_map(|p| {
                let level = *levels.get(&p.id)?;
                p.levels.get(usize::from(level.clamp(1, 5) - 1))?.downed_strike
            })
            .filter(|d| d.window.is_finite() && d.bonus.is_finite())
            .reduce(|a, b| DownedStrike { window: a.window.max(b.window), bonus: a.bonus.max(b.bonus) })
    }
    /// Owned proc powers whose activation condition holds now.
    pub fn active_procs(
        &self,
        levels: &BTreeMap<String, u8>,
        snapshot: &CombatSnapshot,
        capabilities: &BTreeSet<String>,
    ) -> Vec<(&str, u8, &ProcSpec)> {
        self.powers
            .iter()
            .filter_map(|p| {
                let level = *levels.get(&p.id)?;
                if !(1..=5).contains(&level) || !p.required().is_subset(capabilities) {
                    return None;
                }
                let spec = &p.levels[usize::from(level - 1)];
                let proc = spec.proc.as_ref()?;
                spec.activation
                    .active(snapshot)
                    .then_some((p.id.as_str(), level, proc))
            })
            .collect()
    }
    /// The combined intervention of every owned source whose condition holds.
    pub fn active_intervention(
        &self,
        levels: &BTreeMap<String, u8>,
        snapshot: &CombatSnapshot,
        capabilities: &BTreeSet<String>,
    ) -> Option<InterventionSpec> {
        self.powers
            .iter()
            .filter_map(|p| {
                let level = *levels.get(&p.id)?;
                if !(1..=5).contains(&level) || !p.required().is_subset(capabilities) {
                    return None;
                }
                let spec = &p.levels[usize::from(level - 1)];
                let intervention = spec.intervention?;
                spec.activation.active(snapshot).then_some(intervention)
            })
            .reduce(InterventionSpec::combine)
    }
    /// HP per FP spent from owned Blood Price sources (largest wins).
    pub fn blood_price(
        &self,
        levels: &BTreeMap<String, u8>,
        capabilities: &BTreeSet<String>,
    ) -> Option<f32> {
        self.powers
            .iter()
            .filter_map(|p| {
                let level = *levels.get(&p.id)?;
                if !(1..=5).contains(&level) || !p.required().is_subset(capabilities) {
                    return None;
                }
                p.levels[usize::from(level - 1)].blood_price
            })
            .reduce(f32::max)
    }
    /// 0.6: flask/item animation speed bonus from owned cards (summed, <= 0.5).
    pub fn item_speed(
        &self,
        levels: &BTreeMap<String, u8>,
        capabilities: &BTreeSet<String>,
    ) -> f32 {
        self.powers
            .iter()
            .filter_map(|p| {
                let level = *levels.get(&p.id)?;
                if !(1..=5).contains(&level) || !p.required().is_subset(capabilities) {
                    return None;
                }
                p.levels[usize::from(level - 1)].item_speed
            })
            .sum::<f32>()
            .clamp(0.0, 0.5)
    }
    /// 0.8: sums of the extra mechanics of owned cards whose condition holds:
    /// [attack speed, lifesteal, charged, jump, critical, ally damage, ally
    /// HP, ally defense].
    pub fn arts(
        &self,
        levels: &BTreeMap<String, u8>,
        snapshot: &CombatSnapshot,
        capabilities: &BTreeSet<String>,
    ) -> [f32; 8] {
        let mut out = [0.0f32; 8];

        for p in &self.powers {
            let Some(level) = levels.get(&p.id).copied().filter(|l| (1..=5).contains(l)) else {
                continue;
            };
            if !p.required().is_subset(capabilities) {
                continue;
            }
            let spec = &p.levels[usize::from(level - 1)];
            if !spec.activation.active(snapshot) {
                continue;
            }
            for (sum, value) in out.iter_mut().zip(spec.arts()) {
                *sum += value.unwrap_or(0.0);
            }
        }
        out
    }
    /// (elite chance bonus, extra embers per elite) from owned powers.
    pub fn elite_lure(&self, levels: &BTreeMap<String, u8>) -> (f64, u32) {

        let mut chance = 0.0;
        let mut embers = 0;
        for p in &self.powers {
            let Some(level) = levels.get(&p.id).copied().filter(|l| (1..=5).contains(l)) else {
                continue;
            };
            if let Some(lure) = p.levels[usize::from(level - 1)].elite_lure {
                chance += f64::from(lure.chance_bonus);
                embers += u32::from(lure.ember_bonus);
            }
        }
        (chance.min(3.0), embers.min(5))
    }
    /// Editable power payload rows and aggregate reservations. Source-derived
    /// watcher/reset/carrier graphs from the native backends must be composed
    /// separately; this partial manifest must not replace config/effects.json.
    pub fn effect_manifest(&self) -> EffectManifest {
        let mut effects = Vec::new();
        for p in &self.powers {
            for (index, l) in p.levels.iter().enumerate() {
                if l.modifiers.is_empty() {
                    continue;
                }
                let mut fields: BTreeMap<String, serde_json::Value> = l
                    .modifiers
                    .iter()
                    .map(|m| (m.field.xml_name().into(), serde_json::json!(m.value)))
                    .collect();
                fields.insert("effectEndurance".into(), serde_json::json!(-1));
                if p.trigger == EffectTrigger::OnCriticalBlessing {
                    effects.push(EffectRow {
                        id: p.effect_row_base + 5_000 + index as u32,
                        name: format!("Ascension {} {} critical posture payload", p.id, index + 1),
                        fields,
                    });
                    continue;
                }
                if matches!(
                    p.trigger,
                    EffectTrigger::OnMomentum | EffectTrigger::OnNearbyAffliction
                ) {
                    fields.insert(
                        "effectEndurance".into(),
                        serde_json::json!(p.duration_seconds),
                    );
                    // The source-derived watcher/reset graph is composed by
                    // the native trigger backend's manifest. This is its finite
                    // payload, never a watcher or reset effect itself.
                    effects.push(EffectRow {
                        id: p.effect_row_base + 10_000 + index as u32,
                        name: format!("Ascension {} {} finite payload", p.id, index + 1),
                        fields,
                    });
                    continue;
                }
                if fields.contains_key("soulRate") {
                    fields.insert("stateInfo".into(), serde_json::json!(76));
                }
                if matches!(
                    p.trigger,
                    EffectTrigger::OnKill | EffectTrigger::OnKillTimed
                ) {
                    let child_id = 95_005_000 + (p.effect_row_base - 95_000_000) + index as u32;
                    let timed = p.trigger == EffectTrigger::OnKillTimed;
                    fields.insert(
                        "effectEndurance".into(),
                        serde_json::json!(if timed { p.duration_seconds } else { 0.0 }),
                    );
                    fields.insert(
                        "motionInterval".into(),
                        serde_json::json!(if timed { 0 } else { 1 }),
                    );
                    effects.push(EffectRow {
                        id: child_id,
                        name: format!("Ascension {} {} on-kill payload", p.id, index + 1),
                        fields,
                    });
                    fields = BTreeMap::from([
                        ("effectEndurance".into(), serde_json::json!(-1)),
                        ("stateInfo".into(), serde_json::json!(199)),
                        ("applyIdOnGetSoul".into(), serde_json::json!(child_id)),
                    ]);
                }
                effects.push(EffectRow {
                    id: p.effect_row_base + index as u32,
                    name: format!("Ascension {} {}", p.id, index + 1),
                    fields,
                });
            }
        }
        for i in 0..8 {
            effects.push(EffectRow {
                id: 95_010_000 + i,
                name: format!("Ascension attribute {i}"),
                fields: BTreeMap::from([("effectEndurance".into(), serde_json::json!(-1))]),
            });
        }
        for i in 0..16 {
            effects.push(EffectRow {
                id: 95_020_000 + i,
                name: format!("Ascension runtime aggregate {i}"),
                fields: BTreeMap::from([("effectEndurance".into(), serde_json::json!(-1))]),
            });
        }
        EffectManifest { schema: 1, effects }
    }
    pub fn validate(&self) -> Result<()> {
        if self.schema != 1 || self.powers.len() > 1000 {
            return Err(CoreError::Invalid(
                "invalid power library schema/size".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        let mut row_ids = BTreeSet::new();
        for p in &self.powers {
            if !ids.insert(&p.id)
                || p.description.is_empty()
                || p.description.len() > 4096
                || p.effect_row_base < 95_000_000
                || p.effect_row_base > 95_004_000
                || !p.duration_seconds.is_finite()
                || (matches!(
                    p.trigger,
                    EffectTrigger::OnKillTimed
                        | EffectTrigger::OnMomentum
                        | EffectTrigger::OnNearbyAffliction
                ) && !(0.1..=60.0).contains(&p.duration_seconds))
                || (!matches!(
                    p.trigger,
                    EffectTrigger::OnKillTimed
                        | EffectTrigger::OnMomentum
                        | EffectTrigger::OnNearbyAffliction
                ) && p.duration_seconds != 0.0)
            {
                return Err(CoreError::Invalid(format!("invalid power {}", p.id)));
            }
            let mut signature: Option<Vec<EffectField>> = None;
            let proc_identity: Vec<Option<(String, u8)>> = p
                .levels
                .iter()
                .map(|l| l.proc.as_ref().map(|s| (s.proc.clone(), s.trigger.kind())))
                .collect();
            if proc_identity.windows(2).any(|w| w[0] != w[1])
                || p.levels.windows(2).any(|w| w[0].intervention.is_some() != w[1].intervention.is_some())
                || p.levels.windows(2).any(|w| w[0].elite_lure.is_some() != w[1].elite_lure.is_some())
                || p.levels.windows(2).any(|w| w[0].blood_price.is_some() != w[1].blood_price.is_some())
                || p.levels.iter().filter_map(|l| l.blood_price).any(|v| !v.is_finite() || !(0.1..=10.0).contains(&v))
                || p.levels.windows(2).any(|w| w[0].item_speed.is_some() != w[1].item_speed.is_some())
                || p.levels.iter().filter_map(|l| l.item_speed).any(|v| !v.is_finite() || !(0.02..=0.5).contains(&v))
                || p.levels.windows(2).any(|w| {
                    w[0].arts().iter().map(Option::is_some).ne(w[1].arts().iter().map(Option::is_some))
                })
                || p.levels.iter().flat_map(|l| l.arts()).flatten().any(|v| !v.is_finite() || !(0.005..=1.0).contains(&v))
                || p.curse.as_ref().is_some_and(|c| c.is_empty() || c.len() > 2048)
                || p.levels.iter().filter_map(|l| l.elite_lure).any(|lure| {
                    !lure.chance_bonus.is_finite()
                        || !(0.0..=3.0).contains(&lure.chance_bonus)
                        || lure.ember_bonus > 5
                })
            {
                return Err(CoreError::Invalid(format!(
                    "power {} changes proc or intervention identity between levels",
                    p.id
                )));
            }
            for (i, l) in p.levels.iter().enumerate() {
                if let Some(proc) = &l.proc {
                    proc.trigger.validate()?;
                    if !crate::core::valid_proc_id(&proc.proc)
                        || !proc.power.is_finite()
                        || !(0.05..=6.0).contains(&proc.power)
                        || !(1..=6).contains(&proc.count)
                    {
                        return Err(CoreError::Invalid(format!("invalid proc in {}", p.id)));
                    }
                }
                if let Some(intervention) = &l.intervention {
                    intervention.validate()?;
                }
                if !row_ids.insert(p.effect_row_base + i as u32)
                    || l.description.is_empty()
                    || l.description.len() > 4096
                    || l.modifiers.len() > 24
                {
                    return Err(CoreError::Invalid(format!("invalid level for {}", p.id)));
                }
                l.activation.validate(0)?;
                if p.trigger == EffectTrigger::OnCriticalBlessing
                    && (l.modifiers.len() != 1
                        || l.modifiers[0].field != EffectField::SaAttackPowerRate
                        || !(1.0..=2.5).contains(&l.modifiers[0].value))
                {
                    return Err(CoreError::Invalid("native critical blessing requires exactly one posture multiplier within 1.0..2.5".into()));
                }
                if p.trigger == EffectTrigger::OnNearbyAffliction {
                    let value = |field| {
                        l.modifiers
                            .iter()
                            .find(|m| m.field == field)
                            .map(|m| m.value)
                    };
                    if l.modifiers.len() != 3
                        || !value(EffectField::MagicAttackRate)
                            .is_some_and(|v| (1.0..=1.5).contains(&v))
                        || !value(EffectField::MagicConsumptionRate)
                            .is_some_and(|v| (0.35..1.0).contains(&v))
                        || value(EffectField::MagicConsumptionRate)
                            != value(EffectField::MiracleConsumptionRate)
                    {
                        return Err(CoreError::Invalid("native nearby affliction requires magic damage and equal sorcery/incantation FP modifiers within supported bounds".into()));
                    }
                }
                if p.trigger == EffectTrigger::OnMomentum
                    && (l.modifiers.len() != 2
                        || !l.modifiers.iter().any(|m| {
                            m.field == EffectField::StaminaRecoverChangeSpeed
                                && m.value >= 1.0
                                && m.value <= 35.0
                        })
                        || !l.modifiers.iter().any(|m| {
                            m.field == EffectField::ConsumeStaminaRate
                                && m.value >= 0.45
                                && m.value < 1.0
                        }))
                {
                    return Err(CoreError::Invalid("native momentum requires stamina recovery and stamina cost modifiers within supported bounds".into()));
                }
                if p.trigger == EffectTrigger::OnKillTimed
                    && (l.modifiers.len() != 1
                        || l.modifiers[0].field != EffectField::SoulRate
                        || !(1.0..=1.5).contains(&l.modifiers[0].value))
                {
                    return Err(CoreError::Invalid("timed native kill rewards require exactly one soul_rate multiplier within 1.0..1.5".into()));
                }
                let fields: Vec<_> = l.modifiers.iter().map(|m| m.field).collect();
                if fields.iter().copied().collect::<BTreeSet<_>>().len() != fields.len() {
                    return Err(CoreError::Invalid("duplicate effect field".into()));
                }
                if signature.as_ref().is_some_and(|s| s != &fields) {
                    return Err(CoreError::Invalid(format!(
                        "power {} changes mechanic identity between levels",
                        p.id
                    )));
                }
                signature = Some(fields);
                for m in &l.modifiers {
                    if !m.value.is_finite()
                        || m.value.abs() > 1000.0
                        || (m.field.is_integer() && m.value.fract() != 0.0)
                    {
                        return Err(CoreError::Invalid("invalid power field value".into()));
                    }
                    if !m.field.is_additive() && m.value <= 0.0 {
                        return Err(CoreError::Invalid("nonpositive multiplier".into()));
                    }
                }
                // 3.2 No Quarter: a window in seconds (10+ = all of it) and a
                // damage bonus.
                if let Some(d) = l.downed_strike {
                    if !d.window.is_finite() || !(0.1..=100.0).contains(&d.window) || !d.bonus.is_finite() || !(0.0..=1.0).contains(&d.bonus) {
                        return Err(CoreError::Invalid("invalid No Quarter window or bonus".into()));
                    }
                }
                if l.modifiers.is_empty()
                    && p.meta.is_none()
                    && l.proc.is_none()
                    && l.intervention.is_none()
                    && l.elite_lure.is_none()
                    && l.item_speed.is_none()
                    && l.arts().iter().all(Option::is_none)
                    && l.downed_strike.is_none()
                {

                    return Err(CoreError::Invalid(
                        "ability has neither combat nor meta implementation".into(),
                    ));
                }
            }
        }
        self.catalog(BTreeSet::new())?;
        Ok(())
    }
}

/// 3.4 balance: soft caps on a combined total. Damage dealt counts in full
/// up to +60%, then half; damage taken and poise in full down to -35%, then
/// half. Only the good direction is softened (curses' penalties count in
/// full), and the hard bounds still apply after this.
fn soften(field: EffectField, value: f64) -> f64 {
    use EffectField::*;
    match field {
        PhysicsAttackRate | MagicAttackRate | FireAttackRate | ThunderAttackRate | DarkAttackRate
        | SlashAttackRate | BlowAttackRate | ThrustAttackRate | SaAttackPowerRate | StaminaAttackRate => {
            const KNEE: f64 = 1.6;
            if value > KNEE { KNEE + (value - KNEE) * 0.5 } else { value }
        }
        SlashDamageCutRate | BlowDamageCutRate | ThrustDamageCutRate | NeutralDamageCutRate
        | MagicDamageCutRate | FireDamageCutRate | ThunderDamageCutRate | DarkDamageCutRate
        | ToughnessDamageCutRate => {
            const KNEE: f64 = 0.65;
            if value < KNEE { KNEE - (KNEE - value) * 0.5 } else { value }
        }
        _ => value,
    }
}

/// Order-independent combination: add additive fields, multiply factors, then
/// enforce global bounds once. This is used for the dedicated aggregate rows;
/// never combine an OnKill payload into a persistent periodic row.
pub fn aggregate_modifiers<'a>(modifiers: impl IntoIterator<Item = &'a Modifier>) -> Vec<Modifier> {
    let mut combined = BTreeMap::<EffectField, f64>::new();
    for m in modifiers {
        if !m.value.is_finite() {
            continue;
        }
        let value = combined
            .entry(m.field)
            .or_insert(f64::from(m.field.neutral()));
        if m.field.is_additive() {
            *value += f64::from(m.value);
        } else if m.field == EffectField::MotionInterval {
            *value = value.min(f64::from(m.value));
        } else {
            *value *= f64::from(m.value);
        }
    }
    combined
        .into_iter()
        .map(|(field, value)| {
            let (lo, hi) = field.bounds();
            let value = soften(field, value).clamp(f64::from(lo), f64::from(hi)) as f32;
            Modifier {
                field,
                value: if field.is_integer() {
                    value.round()
                } else {
                    value
                },
            }
        })
        .collect()
}

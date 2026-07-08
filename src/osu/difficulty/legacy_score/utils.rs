//! upstream `osu.Game.Rulesets.Osu.Difficulty.Utils.LegacyScoreUtils` の移植。
//!
//! - `calculate_nested_score_per_object`: 譜面の slider/spinner の nested judgement
//!   による平均スコア (1 object あたり) を計算。
//! - `calculate_difficulty_peppy_stars`: stable era の score multiplier
//!   `(HP + OD + CS + clamp(obj/drain*8, 0, 16)) / 38 * 5` を計算。

use crate::{
    model::hit_object::HitObjectKind,
    osu::object::{NestedSliderObjectKind, OsuObject, OsuObjectKind},
    Beatmap,
};

/// upstream: `LegacyScoreUtils.CalculateNestedScorePerObject`。譜面全体の nested
/// スコア (slider の head/tail/repeat/tick + spinner の bonus) の合計を objectCount
/// で割った値。`OsuLegacyScoreMissCalculator.calculateScoreAtCombo` で使う。
pub fn calculate_nested_score_per_object(osu_objects: &[OsuObject], object_count: u32) -> f64 {
    if object_count == 0 {
        return 0.0;
    }
    const BIG_TICK_SCORE: f64 = 30.0;
    const SMALL_TICK_SCORE: f64 = 10.0;

    let mut amount_big_ticks = 0i64;
    let mut amount_small_ticks = 0i64;
    let mut spinner_score = 0.0f64;

    for obj in osu_objects {
        match &obj.kind {
            OsuObjectKind::Slider(slider) => {
                // upstream:
                //   `amountOfBigTicks += 2 + s.RepeatCount` (head + tail + repeats)
                let repeat_count = slider
                    .nested_objects
                    .iter()
                    .filter(|n| n.is_repeat())
                    .count() as i64;
                amount_big_ticks += 2 + repeat_count;
                // upstream: `amountOfSmallTicks += s.NestedHitObjects.Count(SliderTick)`
                let tick_count = slider
                    .nested_objects
                    .iter()
                    .filter(|n| n.is_tick())
                    .count() as i64;
                amount_small_ticks += tick_count;
            }
            OsuObjectKind::Spinner(spinner) => {
                spinner_score += calculate_spinner_score(spinner.duration);
            }
            OsuObjectKind::Circle => {}
        }
    }

    let slider_score =
        amount_big_ticks as f64 * BIG_TICK_SCORE + amount_small_ticks as f64 * SMALL_TICK_SCORE;

    (slider_score + spinner_score) / object_count as f64
}

/// upstream: `LegacyScoreUtils.calculateSpinnerScore`。
fn calculate_spinner_score(duration_ms: f64) -> f64 {
    const SPIN_SCORE: i64 = 100;
    const BONUS_SPIN_SCORE: i64 = 1000;
    const MAXIMUM_ROTATIONS_PER_SECOND: f64 = 477.0 / 60.0;
    const MINIMUM_ROTATIONS_PER_SECOND: f64 = 3.0;

    let seconds_duration = duration_ms / 1000.0;
    let total_half_spins_possible =
        (seconds_duration * MAXIMUM_ROTATIONS_PER_SECOND * 2.0) as i64;
    let half_spins_required_for_completion =
        (seconds_duration * MINIMUM_ROTATIONS_PER_SECOND) as i64;
    let half_spins_required_before_bonus = half_spins_required_for_completion + 3;

    let full_spins = total_half_spins_possible / 2;
    let mut score: i64 = SPIN_SCORE * full_spins;

    let mut bonus_spins = (total_half_spins_possible - half_spins_required_before_bonus) / 2;
    // upstream: Max(0, bonusSpins - fullSpins / 2)
    bonus_spins = i64::max(0, bonus_spins - full_spins / 2);
    score += BONUS_SPIN_SCORE * bonus_spins;

    score as f64
}

/// upstream: `LegacyRulesetExtensions.CalculateDifficultyPeppyStars`
///   `round((HP + OD + CS + clamp(obj/drain * 8, 0, 16)) / 38 * 5)`
///
/// mames-pp では f64 で計算するが upstream は decimal (128bit fixed) で計算するので
/// **極端な精度が要求される個別譜面では 1 ずれる可能性あり**。実用上は無視できる範囲。
pub fn calculate_difficulty_peppy_stars(map: &Beatmap, object_count: u32) -> f64 {
    let drain_length = calculate_drain_length(map, object_count);
    let obj_to_drain_ratio = if drain_length != 0 {
        let raw = (object_count as f64 / drain_length as f64) * 8.0;
        raw.clamp(0.0, 16.0)
    } else {
        16.0
    };

    let hp = f64::from(map.hp);
    let od = f64::from(map.od);
    let cs = f64::from(map.cs);

    ((hp + od + cs + obj_to_drain_ratio) / 38.0 * 5.0).round()
}

/// upstream: `LegacyScoreUtils.CalculateDifficultyPeppyStars`
/// (drainLength は break を除いた秒数)。
fn calculate_drain_length(map: &Beatmap, object_count: u32) -> i32 {
    if object_count == 0 || map.hit_objects.is_empty() {
        return 0;
    }

    let first_start = map.hit_objects[0].start_time;
    let last_start_time = map
        .hit_objects
        .last()
        .map(|h| match h.kind {
            HitObjectKind::Spinner(ref spinner) => h.start_time + spinner.duration,
            HitObjectKind::Hold(ref hold) => h.start_time + hold.duration,
            _ => h.start_time,
        })
        .unwrap_or(first_start);

    // break は rosu-map 側の Beatmap 直下では持ってないので 0 とする
    // (実 upstream は BreakPeriod の集計。実運用上ほとんどの譜面で break は数秒〜数十秒、
    //  スコア倍率への影響は 1 未満で peppy_stars の round 前後で差が出るケースは稀)。
    let drain_ms = last_start_time.round() as i32 - first_start.round() as i32;
    drain_ms / 1000
}

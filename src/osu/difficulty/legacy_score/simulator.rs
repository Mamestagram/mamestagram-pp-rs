//! upstream `OsuLegacyScoreSimulator` の Rust 移植。
//!
//! 譜面全体をシミュレートして classic (stable v1) スコアの各 attribute:
//! - `ComboScore` (max combo 中に取れる combo multiplier 分)
//! - `AccuracyScore` (combo multiplier 抜きの base score)
//! - `MaxCombo`
//! を返す。実際に使うのは `ComboScore` のみで、`MaximumLegacyComboScore` として
//! attribute に格納する。

use crate::osu::object::{NestedSliderObjectKind, OsuObject, OsuObjectKind};

/// upstream `LegacyScoreAttributes` に対応する結果構造体。
pub struct LegacyScoreAttributes {
    pub combo_score: f64,
    #[allow(dead_code)]
    pub accuracy_score: i64,
    pub max_combo: u32,
}

/// upstream `OsuLegacyScoreSimulator.Simulate`。
///
/// - `score_multiplier`: `LegacyRulesetExtensions.CalculateDifficultyPeppyStars` の結果。
///   mod は含めない (mod 倍率は miss calculator 側で別途掛ける)。
pub fn simulate(osu_objects: &[OsuObject], score_multiplier: f64) -> LegacyScoreAttributes {
    let mut state = SimState {
        combo: 0,
        combo_score: 0.0,
        accuracy_score: 0,
        score_multiplier,
    };

    for obj in osu_objects {
        simulate_object(obj, &mut state);
    }

    LegacyScoreAttributes {
        combo_score: state.combo_score,
        accuracy_score: state.accuracy_score,
        max_combo: state.combo,
    }
}

struct SimState {
    combo: u32,
    combo_score: f64,
    accuracy_score: i64,
    score_multiplier: f64,
}

fn simulate_object(obj: &OsuObject, state: &mut SimState) {
    match &obj.kind {
        OsuObjectKind::Circle => {
            // upstream:
            //   scoreIncrease = 300; addScoreComboMultiplier = true; increaseCombo = true;
            simulate_hit_generic(state, 300, /*combo_mult=*/ true, /*inc_combo=*/ true);
        }
        OsuObjectKind::Slider(slider) => {
            // upstream nested 順:
            //   SliderHeadCircle (30, +combo) — mames には無いので合成
            //   SliderTick (10, +combo)
            //   SliderRepeat (30, +combo)
            //   SliderTailCircle (30, +combo)
            // その後 Slider 自体: scoreIncrease = 300, increaseCombo=false, addScoreComboMultiplier=true

            // synthetic head
            simulate_hit_generic(state, 30, false, true);

            for nested in &slider.nested_objects {
                let score = match nested.kind {
                    NestedSliderObjectKind::Tick => 10,
                    NestedSliderObjectKind::Repeat | NestedSliderObjectKind::Tail => 30,
                };
                simulate_hit_generic(state, score, false, true);
            }

            // slider judgment (300)
            simulate_hit_generic(state, 300, /*combo_mult=*/ true, /*inc_combo=*/ false);
        }
        OsuObjectKind::Spinner(spinner) => {
            // upstream: spinner ticks (bonus, 100 / 1100) を全部シミュレート、
            // その後本体 300 + combo multiplier。
            // combo_score には bonus は寄与しないので、bonus 部分は無視して
            // 本体 300 だけで良い。
            let _ = spinner; // 本体だけ
            simulate_hit_generic(state, 300, /*combo_mult=*/ true, /*inc_combo=*/ true);
        }
    }
}

/// upstream `simulateHit` の core 部分 (bonus 部分は combo_score には効かないので省略)。
fn simulate_hit_generic(state: &mut SimState, score_increase: i64, add_combo_multiplier: bool, increase_combo: bool) {
    if add_combo_multiplier {
        // upstream:
        //   `ComboScore += (int)(Math.Max(0, combo - 1) * (scoreIncrease / 25 * scoreMultiplier))`
        // int キャストで下位を切るので、事前に f64 で計算 → int キャストで truncate。
        let combo_minus_1 = i64::max(0, state.combo as i64 - 1);
        let per_hit = (score_increase / 25) as f64 * state.score_multiplier;
        // upstream: PossibleLossOfFraction (intentional to match osu-stable)
        //   `scoreIncrease / 25` は int 割算 → 300/25 = 12, 30/25 = 1, 10/25 = 0
        state.combo_score += (combo_minus_1 as f64 * per_hit).trunc();
    }

    state.accuracy_score += score_increase;
    if increase_combo {
        state.combo += 1;
    }
}

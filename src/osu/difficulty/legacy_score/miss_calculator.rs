//! upstream `OsuLegacyScoreMissCalculator` の Rust 移植。
//!
//! classic (stable) score で `LegacyTotalScore` が既知の場合に、そのスコア値から
//! 「map の何処かで dropped combo (miss / slider break) が何回あるはず」の逆算を行う。
//!
//! 呼び出し前提:
//! - `attributes.max_combo > 0`
//! - `legacy_total_score` が付いている (mames-pp では `Option<u64>` で渡す)
//! - `using_classic_slider_acc == true && !using_score_v2`

use crate::{
    osu::{OsuDifficultyAttributes, OsuScoreState},
    GameMods,
};

pub struct OsuLegacyScoreMissCalculator<'a> {
    state: &'a OsuScoreState,
    mods: &'a GameMods,
    attrs: &'a OsuDifficultyAttributes,
    accuracy: f64,
    legacy_total_score: u64,
}

impl<'a> OsuLegacyScoreMissCalculator<'a> {
    pub fn new(
        state: &'a OsuScoreState,
        mods: &'a GameMods,
        attrs: &'a OsuDifficultyAttributes,
        accuracy: f64,
        legacy_total_score: u64,
    ) -> Self {
        Self {
            state,
            mods,
            attrs,
            accuracy,
            legacy_total_score,
        }
    }

    /// upstream: `Calculate()`。miss count 推定値 (常に >= 1) を返す。
    /// max_combo 0 や legacy_total_score 0 の場合は 0 を返す。
    pub fn calculate(&self) -> f64 {
        if self.attrs.max_combo == 0 {
            return 0.0;
        }
        // upstream の getLegacyScoreMultiplier は mod-dependent の倍率。
        // Relax/Autopilot が付いてたら 0 を返して pp path を無効化する。
        let mod_multiplier = self.get_legacy_mod_multiplier();
        if mod_multiplier == 0.0 || self.attrs.legacy_score_base_multiplier == 0.0 {
            return 0.0;
        }
        let score_v1_multiplier = self.attrs.legacy_score_base_multiplier * mod_multiplier;
        if score_v1_multiplier == 0.0 {
            return 0.0;
        }

        let relevant_combo_per_object = self.calculate_relevant_score_combo_per_object();
        let maximum_miss_count = self.calculate_maximum_combo_based_miss_count();

        let score_obtained_during_max_combo = self.calculate_score_at_combo(
            f64::from(self.state.max_combo),
            relevant_combo_per_object,
            score_v1_multiplier,
        );
        let remaining_score = self.legacy_total_score as f64 - score_obtained_during_max_combo;

        if remaining_score <= 0.0 {
            return maximum_miss_count;
        }

        let remaining_combo = f64::from(self.attrs.max_combo - self.state.max_combo);
        let expected_remaining_score = self.calculate_score_at_combo(
            remaining_combo,
            relevant_combo_per_object,
            score_v1_multiplier,
        );

        let mut score_based_miss_count = expected_remaining_score / remaining_score;
        // upstream: `Max(scoreBasedMissCount, 1)` — combo-based と併用させるため最低 1
        score_based_miss_count = score_based_miss_count.max(1.0);
        // upstream: `Min(scoreBasedMissCount, maximumMissCount)`
        score_based_miss_count.min(maximum_miss_count)
    }

    /// upstream: `calculateScoreAtCombo(combo, relevantComboPerObject, scoreV1Multiplier)`
    fn calculate_score_at_combo(
        &self,
        combo: f64,
        relevant_combo_per_object: f64,
        score_v1_multiplier: f64,
    ) -> f64 {
        let total_hits =
            self.state.n300 as f64 + self.state.n100 as f64 + self.state.n50 as f64 + self.state.misses as f64;

        let estimated_objects = combo / relevant_combo_per_object - 1.0;

        // upstream: 算術級数の和 (2*(r-1) + (n-1)*r) * n / 2, r = combo per object
        let combo_score = if relevant_combo_per_object > 0.0 {
            (2.0 * (relevant_combo_per_object - 1.0)
                + (estimated_objects - 1.0) * relevant_combo_per_object)
                * estimated_objects
                / 2.0
        } else {
            0.0
        };

        // upstream: `comboScore *= accuracy * 300 / 25 * scoreV1Multiplier`
        let combo_score_scaled = combo_score * self.accuracy * 300.0 / 25.0 * score_v1_multiplier;

        // upstream: `objectsHit = (totalHits - countMiss) * combo / maxCombo`
        let objects_hit =
            (total_hits - self.state.misses as f64) * combo / f64::from(self.attrs.max_combo);

        // upstream: `nonComboScore = (300 + NestedScorePerObject) * accuracy * objectsHit`
        let non_combo_score =
            (300.0 + self.attrs.nested_score_per_object) * self.accuracy * objects_hit;

        combo_score_scaled + non_combo_score
    }

    /// upstream: `calculateRelevantScoreComboPerObject`
    fn calculate_relevant_score_combo_per_object(&self) -> f64 {
        let mut combo_score = self.attrs.maximum_legacy_combo_score;

        // upstream: `comboScore /= 300.0 / 25.0 * attributes.LegacyScoreBaseMultiplier`
        let divisor = 300.0 / 25.0 * self.attrs.legacy_score_base_multiplier;
        if divisor != 0.0 {
            combo_score /= divisor;
        }

        // upstream: `result = (MaxCombo - 2) * MaxCombo / max(MaxCombo + 2 * (comboScore - 1), 1)`
        let max_combo = f64::from(self.attrs.max_combo);
        let numerator = (max_combo - 2.0) * max_combo;
        let denominator = f64::max(max_combo + 2.0 * (combo_score - 1.0), 1.0);

        numerator / denominator
    }

    /// upstream: `calculateMaximumComboBasedMissCount` (harsher version of combo-based
    /// miss estimation, capped at score-based value)
    fn calculate_maximum_combo_based_miss_count(&self) -> f64 {
        let miss_count_base = f64::from(self.state.misses);

        if self.attrs.n_sliders == 0 {
            return miss_count_base;
        }

        let total_imperfect_hits =
            f64::from(self.state.n100 + self.state.n50 + self.state.misses);

        let mut miss_count = 0.0;

        // upstream: `likelyMissedSliderendPortion = 0.04 + 0.06 * pow(min(AimTopWeightedSliderFactor, 1), 2)`
        let likely_missed_sliderend_portion =
            0.04 + 0.06 * self.attrs.aim_top_weighted_slider_factor.min(1.0).powi(2);
        let threshold_reduction = f64::min(
            4.0 + likely_missed_sliderend_portion * f64::from(self.attrs.n_sliders),
            f64::from(self.attrs.n_sliders),
        );
        let full_combo_threshold = f64::from(self.attrs.max_combo) - threshold_reduction;

        if f64::from(self.state.max_combo) < full_combo_threshold {
            // upstream: `Pow(fullComboThreshold / max(1.0, scoreMaxCombo), 2.5)`
            miss_count = (full_combo_threshold / f64::from(self.state.max_combo).max(1.0)).powf(2.5);
        }

        miss_count = miss_count.min(total_imperfect_hits);

        // upstream: `maxPossibleSliderBreaks = min(SliderCount, (MaxCombo - scoreMaxCombo) / 2)`
        let combo_diff = if self.attrs.max_combo > self.state.max_combo {
            self.attrs.max_combo - self.state.max_combo
        } else {
            0
        };
        let max_possible_slider_breaks = i32::min(
            self.attrs.n_sliders as i32,
            (combo_diff as i32) / 2,
        );

        let slider_breaks = miss_count - miss_count_base;
        if slider_breaks > f64::from(max_possible_slider_breaks) {
            miss_count = miss_count_base + f64::from(max_possible_slider_breaks);
        }

        miss_count
    }

    /// upstream: `getLegacyScoreMultiplier`
    fn get_legacy_mod_multiplier(&self) -> f64 {
        let score_v2 = self.mods.has_score_v2();
        let mut multiplier = 1.0;

        if self.mods.nf() {
            multiplier *= if score_v2 { 1.0 } else { 0.5 };
        }
        if self.mods.ez() {
            multiplier *= 0.5;
        }
        if self.mods.ht() {
            multiplier *= 0.3;
        }
        if self.mods.hd() {
            multiplier *= 1.06;
        }
        if self.mods.hr() {
            multiplier *= if score_v2 { 1.10 } else { 1.06 };
        }
        if self.mods.dt() {
            multiplier *= if score_v2 { 1.20 } else { 1.12 };
        }
        if self.mods.fl() {
            multiplier *= 1.12;
        }
        if self.mods.so() {
            multiplier *= 0.9;
        }
        // upstream: `case OsuModRelax: case OsuModAutopilot: return 0;`
        if self.mods.rx() || self.mods.ap() {
            return 0.0;
        }
        multiplier
    }
}

use crate::{
    taiko::{TaikoDifficultyAttributes, TaikoPerformanceAttributes, TaikoScoreState},
    util::{
        difficulty::{logistic, reverse_lerp},
        special_functions::{erf, erf_inv},
    },
    GameMods,
};

pub(super) struct TaikoPerformanceCalculator<'mods> {
    attrs: TaikoDifficultyAttributes,
    mods: &'mods GameMods,
    state: TaikoScoreState,
}

impl<'a> TaikoPerformanceCalculator<'a> {
    pub const fn new(
        attrs: TaikoDifficultyAttributes,
        mods: &'a GameMods,
        state: TaikoScoreState,
    ) -> Self {
        Self { attrs, mods, state }
    }
}

impl TaikoPerformanceCalculator<'_> {
    pub fn calculate(self) -> TaikoPerformanceAttributes {
        // upstream `TaikoPerformanceCalculator.CreatePerformanceAttributes`
        let total_hits = self.total_hits();
        let great_hit_window = self.attrs.great_hit_window;

        // upstream: `estimatedUnstableRate = (countGreat == 0 || greatHitWindow <= 0)
        //             ? null : computeDeviationUpperBound(countGreat / totalHits) * 10`
        let estimated_unstable_rate = if self.state.n300 == 0 || great_hit_window <= 0.0 {
            None
        } else {
            let accuracy = f64::from(self.state.n300) / total_hits;
            Some(self.compute_deviation_upper_bound(accuracy) * 10.0)
        };

        // upstream: totalDifficultHits = totalHits * consistency_factor
        let total_difficult_hits = total_hits * self.attrs.consistency_factor;

        // upstream: isConvert / isClassic
        let is_convert = self.attrs.is_convert;
        let is_classic = self.mods.cl(); // Classic mod flag

        // upstream: difficulty * 1.08 + accuracy * 1.1
        let difficulty_value = self
            .compute_difficulty_value(estimated_unstable_rate, total_difficult_hits, is_convert, is_classic)
            * 1.08;
        let accuracy_value = self
            .compute_accuracy_value(estimated_unstable_rate, total_difficult_hits, is_convert)
            * 1.1;

        let pp = difficulty_value + accuracy_value;

        // effective_miss_count は upstream には無いフィールドだが後方互換のため残す。
        // upstream の miss_penalty は effective_miss_count ではなく countMiss を直接使うので、
        // ここでは表示専用値として保持。
        let effective_miss_count = f64::from(self.state.misses);

        TaikoPerformanceAttributes {
            difficulty: self.attrs,
            pp,
            pp_acc: accuracy_value,
            pp_difficulty: difficulty_value,
            effective_miss_count,
            estimated_unstable_rate,
        }
    }

    fn compute_difficulty_value(
        &self,
        estimated_unstable_rate: Option<f64>,
        total_difficult_hits: f64,
        is_convert: bool,
        is_classic: bool,
    ) -> f64 {
        // upstream: if (estimatedUnstableRate == null || totalDifficultHits == 0) return 0
        let Some(estimated_unstable_rate) = estimated_unstable_rate else {
            return 0.0;
        };
        if total_difficult_hits <= 0.0 {
            return 0.0;
        }

        // upstream: rhythm penalty 計算
        // rhythmExpectedUnstableRate = computeDeviationUpperBound(1.0) * 10
        // rhythmMaximumUnstableRate = computeDeviationUpperBound(0.8) * 10
        let rhythm_expected_unstable_rate = self.compute_deviation_upper_bound(1.0) * 10.0;
        let rhythm_maximum_unstable_rate = self.compute_deviation_upper_bound(0.8) * 10.0;

        // upstream: rhythmFactor = ReverseLerp(RhythmDifficulty / StarRating, 0.15, 0.4)
        let rhythm_factor = if self.attrs.stars > 0.0 {
            reverse_lerp(self.attrs.rhythm / self.attrs.stars, 0.15, 0.4)
        } else {
            0.0
        };

        // upstream: rhythmPenalty = 1 - Logistic(EUR, midpoint = (expected+max)/2, mult = 10/(max-expected), maxValue = 0.25 * rhythmFactor^3)
        let mid_ur = (rhythm_expected_unstable_rate + rhythm_maximum_unstable_rate) / 2.0;
        let ur_range = rhythm_maximum_unstable_rate - rhythm_expected_unstable_rate;
        let rhythm_penalty = if ur_range > 0.0 {
            1.0 - logistic(
                estimated_unstable_rate,
                mid_ur,
                10.0 / ur_range,
                Some(0.25 * rhythm_factor.powi(3)),
            )
        } else {
            1.0
        };

        // upstream: baseDifficulty = 5 * max(1.0, StarRating * rhythmPenalty / 0.110) - 4.0
        let base_difficulty = 5.0 * f64::max(1.0, self.attrs.stars * rhythm_penalty / 0.110) - 4.0;

        // upstream: min(pow(base, 3) / 69052.51, pow(base, 2.25) / 1250.0)
        let mut difficulty_value = f64::min(
            base_difficulty.powi(3) / 69052.51,
            base_difficulty.powf(2.25) / 1250.0,
        );

        // upstream: *= 1 + 0.10 * max(0, StarRating - 10)
        difficulty_value *= 1.0 + 0.10 * f64::max(0.0, self.attrs.stars - 10.0);

        // upstream: lengthBonus = 1 + 0.25 * totalDifficultHits / (totalDifficultHits + 4000)
        let length_bonus = 1.0 + 0.25 * total_difficult_hits / (total_difficult_hits + 4000.0);
        difficulty_value *= length_bonus;

        // upstream: missPenalty = 0.97 + 0.03 * totalDifficultHits / (totalDifficultHits + 1500)
        //           difficulty_value *= pow(missPenalty, countMiss)
        let miss_penalty = 0.97 + 0.03 * total_difficult_hits / (total_difficult_hits + 1500.0);
        difficulty_value *= miss_penalty.powf(f64::from(self.state.misses));

        // upstream: Hidden bonus (complex)
        if self.mods.hd() {
            let mut hidden_bonus = if is_convert { 0.025 } else { 0.1 };

            // Hidden+Flashlight は reading penalty 対象外
            if !self.mods.fl() {
                // Non-classic: 20% に縮小
                if !is_classic {
                    hidden_bonus *= 0.2;
                }
                // Classic + Easy: 50% に縮小
                if self.mods.ez() && is_classic {
                    hidden_bonus *= 0.5;
                }
            }

            difficulty_value *= 1.0 + hidden_bonus;
        }

        // upstream: Flashlight bonus
        if self.mods.fl() {
            difficulty_value *= f64::max(
                1.0,
                1.050 - f64::min(self.attrs.mono_stamina_factor / 50.0, 1.0) * length_bonus,
            );
        }

        // upstream: mono accuracy scaling
        // monoAccScalingExponent = 2 + MonoStaminaFactor
        // monoAccScalingShift = 500 - 100 * (MonoStaminaFactor * 3)
        let mono_acc_scaling_exp = 2.0 + self.attrs.mono_stamina_factor;
        let mono_acc_scaling_shift = 500.0 - 100.0 * (self.attrs.mono_stamina_factor * 3.0);

        difficulty_value
            * erf(mono_acc_scaling_shift / (f64::sqrt(2.0) * estimated_unstable_rate))
                .powf(mono_acc_scaling_exp)
    }

    fn compute_accuracy_value(
        &self,
        estimated_unstable_rate: Option<f64>,
        total_difficult_hits: f64,
        is_convert: bool,
    ) -> f64 {
        // upstream: if (greatHitWindow <= 0 || estimatedUnstableRate == null) return 0
        if self.attrs.great_hit_window <= 0.0 {
            return 0.0;
        }
        let Some(estimated_unstable_rate) = estimated_unstable_rate else {
            return 0.0;
        };

        // upstream: 470 * pow(0.9885, EUR)
        let mut accuracy_value = 470.0 * 0.9885_f64.powf(estimated_unstable_rate);

        // upstream: *= 1 + pow(50/EUR, 2) * pow(StarRating, 2.8) / 600
        accuracy_value *= 1.0
            + (50.0 / estimated_unstable_rate).powi(2)
                * self.attrs.stars.powf(2.8)
                / 600.0;

        // upstream: Hidden bonus (only if not convert)
        if self.mods.hd() && !is_convert {
            accuracy_value *= 1.075;
        }

        // upstream: length bonus based on totalDifficultHits
        accuracy_value *= 1.0 + 0.3 * total_difficult_hits / (total_difficult_hits + 4000.0);

        // upstream: HDFL memory bonus
        let memory_length_bonus = f64::min(1.15, (self.total_hits() / 1500.0).powf(0.3));

        if self.mods.fl() && self.mods.hd() && !is_convert {
            accuracy_value *= f64::max(1.0, 1.05 * memory_length_bonus);
        }

        accuracy_value
    }

    // upstream: computeDeviationUpperBound(accuracy)
    fn compute_deviation_upper_bound(&self, accuracy: f64) -> f64 {
        const Z: f64 = 2.32634787404; // 99% critical value (one-tailed)

        let n = self.total_hits();
        let p = accuracy;

        let p_lower_bound = (n * p + Z * Z / 2.0) / (n + Z * Z)
            - Z / (n + Z * Z) * f64::sqrt(n * p * (1.0 - p) + Z * Z / 4.0);

        self.attrs.great_hit_window / (f64::sqrt(2.0) * erf_inv(p_lower_bound))
    }

    const fn total_hits(&self) -> f64 {
        self.state.total_hits() as f64
    }
}

use std::f64::consts::PI;

use crate::{
    osu::{
        difficulty::skills::{aim::Aim, speed::Speed},
        OsuDifficultyAttributes, OsuPerformanceAttributes, OsuScoreState,
    },
    util::{
        difficulty::reverse_lerp,
        float_ext::FloatExt,
        special_functions::{erf, erf_inv},
    },
    GameMods,
};

use super::{n_large_tick_miss, n_slider_ends_dropped, total_imperfect_hits};

// upstream OsuPerformanceCalculator.cs:25 — 1.12 (旧 1.15 は fork-specific 値)
pub const PERFORMANCE_BASE_MULTIPLIER: f64 = 1.12;
// upstream: PERFORMANCE_NORM_EXPONENT = 1.1
pub const PERFORMANCE_NORM_EXPONENT: f64 = 1.1;
// relax は本 fork の独自計算なので既存の値を保持
pub const PERFORMANCE_BASE_MULTIPLIER_RELAX: f64 = 1.15;

// DiffUtils.SQRT2 is deliberately one ULP below std::f64::consts::SQRT_2.
// Retain that exact value because this feeds into lazer's speed deviation.
#[allow(clippy::approx_constant, clippy::excessive_precision)]
const LAZER_SQRT_2: f64 = 1.414_213_562_373_095_0;

pub(super) struct OsuPerformanceCalculator<'mods> {
    attrs: OsuDifficultyAttributes,
    mods: &'mods GameMods,
    acc: f64,
    state: OsuScoreState,
    effective_miss_count: f64,
    combo_based_estimated_miss_count: f64,
    score_based_estimated_miss_count: Option<f64>,
    using_classic_slider_acc: bool,
}

impl<'a> OsuPerformanceCalculator<'a> {
    pub const fn new(
        attrs: OsuDifficultyAttributes,
        mods: &'a GameMods,
        acc: f64,
        state: OsuScoreState,
        effective_miss_count: f64,
        combo_based_estimated_miss_count: f64,
        score_based_estimated_miss_count: Option<f64>,
        using_classic_slider_acc: bool,
    ) -> Self {
        Self {
            attrs,
            mods,
            acc,
            state,
            effective_miss_count,
            combo_based_estimated_miss_count,
            score_based_estimated_miss_count,
            using_classic_slider_acc,
        }
    }
}

impl OsuPerformanceCalculator<'_> {
    pub fn calculate(self) -> OsuPerformanceAttributes {
        let total_hits = self.state.total_hits();

        if total_hits == 0 {
            return OsuPerformanceAttributes {
                difficulty: self.attrs,
                ..Default::default()
            };
        }

        // RX は fork の独自計算に routing
        if self.mods.rx() {
            return self.calculate_relax();
        }

        // vanilla path (mode 0) は osu-master 2 完全一致で計算する
        self.calculate_vanilla()
    }

    // upstream OsuPerformanceCalculator.CreatePerformanceAttributes を完全移植
    fn calculate_vanilla(self) -> OsuPerformanceAttributes {
        let total_hits = f64::from(self.state.total_hits());

        let mut multiplier = PERFORMANCE_BASE_MULTIPLIER;

        if self.mods.nf() {
            multiplier *= (1.0 - 0.02 * self.effective_miss_count).max(0.9);
        }

        if self.mods.so() && total_hits > 0.0 {
            multiplier *= 1.0 - (f64::from(self.attrs.n_spinners) / total_hits).powf(0.85);
        }

        // Slider break estimation
        let (aim_estimated_slider_breaks, speed_estimated_slider_breaks) = if self
            .effective_miss_count
            > 0.0
        {
            (
                self.calculate_estimated_slider_breaks(self.attrs.aim_top_weighted_slider_factor),
                self.calculate_estimated_slider_breaks(self.attrs.speed_top_weighted_slider_factor),
            )
        } else {
            (0.0, 0.0)
        };

        let speed_deviation = self.calculate_speed_deviation();

        let aim_value = self.compute_aim_value_vanilla(aim_estimated_slider_breaks);
        let speed_value =
            self.compute_speed_value_vanilla(speed_deviation, speed_estimated_slider_breaks);
        let acc_value = self.compute_accuracy_value_vanilla();

        let reading_value = self.compute_reading_value_vanilla(aim_estimated_slider_breaks);
        let flashlight_value = self.compute_flashlight_value_vanilla();
        let cognition_value = sum_cognition_difficulty(reading_value, flashlight_value);

        // upstream: totalValue = Norm(1.1, aim, speed, acc, cognition) * multiplier
        let total_norm = norm_pnorm(
            PERFORMANCE_NORM_EXPONENT,
            &[aim_value, speed_value, acc_value, cognition_value],
        );
        let pp = total_norm * multiplier;

        OsuPerformanceAttributes {
            difficulty: self.attrs,
            pp_acc: acc_value,
            pp_aim: aim_value,
            pp_flashlight: flashlight_value,
            pp_speed: speed_value,
            pp_reading: reading_value,
            pp,
            effective_miss_count: self.effective_miss_count,
            combo_based_estimated_miss_count: self.combo_based_estimated_miss_count,
            score_based_estimated_miss_count: self.score_based_estimated_miss_count,
            aim_estimated_slider_breaks,
            speed_estimated_slider_breaks,
            speed_deviation,
        }
    }

    pub fn calculate_relax(self) -> OsuPerformanceAttributes {
        let total_hits = self.state.total_hits();

        if total_hits == 0 {
            return OsuPerformanceAttributes {
                difficulty: self.attrs,
                ..Default::default()
            };
        }

        let total_hits = f64::from(total_hits);
        let mut multiplier = PERFORMANCE_BASE_MULTIPLIER_RELAX;

        // SO penalty
        if self.mods.so() && total_hits > 0.0 {
            multiplier *= 1.0 - (f64::from(self.attrs.n_spinners) / total_hits).powf(0.85);
        }

        let speed_deviation = self.calculate_speed_deviation();

        let mut aim_value = self.compute_aim_value();
        let mut speed_value = self.compute_speed_value(speed_deviation);
        let mut acc_value = self.compute_accuracy_value();

        let aim_speed_ratio = aim_value / speed_value;

        if aim_speed_ratio < 1.0 {
            speed_value = speed_value.powf(self.acc * aim_speed_ratio);
            aim_value *= aim_speed_ratio;
        }

        aim_value = aim_value.powf(1.1);
        acc_value = acc_value.powf(1.1);

        let pp = (aim_value + speed_value + acc_value).powf(1.0 / 1.1) * multiplier;

        OsuPerformanceAttributes {
            difficulty: self.attrs,
            pp_acc: acc_value,
            pp_aim: aim_value,
            pp_flashlight: 0.0,
            pp_speed: speed_value,
            pp_reading: 0.0,
            pp,
            effective_miss_count: self.effective_miss_count,
            combo_based_estimated_miss_count: self.combo_based_estimated_miss_count,
            score_based_estimated_miss_count: self.score_based_estimated_miss_count,
            aim_estimated_slider_breaks: 0.0,
            speed_estimated_slider_breaks: 0.0,
            speed_deviation,
        }
    }

    fn compute_aim_value(&self) -> f64 {
        if self.mods.ap() {
            return 0.0;
        }

        let mut aim_difficulty = self.attrs.aim;

        if self.attrs.n_sliders > 0 && self.attrs.aim_difficult_slider_count > 0.0 {
            let estimate_improperly_followed_difficult_sliders = if self.using_classic_slider_acc {
                // * When the score is considered classic (regardless if it was made on old client or not)
                // * we consider all missing combo to be dropped difficult sliders
                let maximum_possible_dropped_sliders = total_imperfect_hits(&self.state);

                f64::clamp(
                    f64::min(
                        maximum_possible_dropped_sliders,
                        f64::from(self.attrs.max_combo - self.state.max_combo),
                    ),
                    0.0,
                    self.attrs.aim_difficult_slider_count,
                )
            } else {
                // * We add tick misses here since they too mean that the player didn't follow the slider properly
                // * We however aren't adding misses here because missing slider heads has a harsh penalty
                // * by itself and doesn't mean that the rest of the slider wasn't followed properly
                f64::clamp(
                    f64::from(
                        n_slider_ends_dropped(&self.attrs, &self.state)
                            + n_large_tick_miss(&self.attrs, &self.state),
                    ),
                    0.0,
                    self.attrs.aim_difficult_slider_count,
                )
            };

            let slider_nerf_factor = (1.0 - self.attrs.slider_factor)
                * f64::powf(
                    1.0 - estimate_improperly_followed_difficult_sliders
                        / self.attrs.aim_difficult_slider_count,
                    3.0,
                )
                + self.attrs.slider_factor;
            aim_difficulty *= slider_nerf_factor;
        }

        let mut aim_value = Aim::difficulty_to_performance(aim_difficulty);

        let total_hits = self.total_hits();

        let len_bonus = 0.95
            + 0.4 * (total_hits / 2000.0).min(1.0)
            + f64::from(u8::from(total_hits > 2000.0)) * (total_hits / 2000.0).log10() * 0.5;

        aim_value *= len_bonus;

        if self.effective_miss_count > 0.0 {
            aim_value *= Self::calculate_miss_penalty(
                self.effective_miss_count,
                self.attrs.aim_difficult_strain_count,
            );
        }

        if self.effective_miss_count > 0.0 {
            if self.mods.rx() {
                aim_value *= Self::calculate_relax_miss_penalty(
                    self.total_hits(),
                    self.effective_miss_count,
                );
            } else {
                aim_value *= Self::calculate_miss_penalty(
                    self.effective_miss_count,
                    self.attrs.aim_difficult_strain_count,
                );
            }
        }

        let ar_factor = if self.attrs.ar > 10.33 {
            0.3 * (self.attrs.ar - 10.33)
        } else if self.attrs.ar < 8.0 {
            0.05 * (8.0 - self.attrs.ar)
        } else {
            0.0
        };

        // * Buff for longer maps with high AR.
        aim_value *= 1.0 + ar_factor * len_bonus;

        if self.mods.bl() {
            aim_value *= 1.3
                + (total_hits
                    * (0.0016 / (1.0 + 2.0 * self.effective_miss_count))
                    * self.acc.powf(16.0))
                    * (1.0 - 0.003 * self.attrs.hp * self.attrs.hp);
        } else if self.mods.hd() || self.mods.tc() {
            // * We want to give more reward for lower AR when it comes to aim and HD. This nerfs high AR and buffs lower AR.
            aim_value *= 1.0 + 0.04 * (12.0 - self.attrs.ar);
        }

        aim_value *= self.acc;
        // * It is important to consider accuracy difficulty when scaling with accuracy.
        aim_value *= 0.98 + f64::powf(f64::max(0.0, self.attrs.od()), 2.0) / 2500.0;

        aim_value
    }

    fn compute_speed_value(&self, speed_deviation: Option<f64>) -> f64 {
        let Some(speed_deviation) = speed_deviation.filter(|_| !self.mods.rx()) else {
            return 0.0;
        };

        let mut speed_value = Speed::difficulty_to_performance(self.attrs.speed);

        let total_hits = self.total_hits();

        let len_bonus = 0.95
            + 0.4 * (total_hits / 2000.0).min(1.0)
            + f64::from(u8::from(total_hits > 2000.0)) * (total_hits / 2000.0).log10() * 0.5;

        speed_value *= len_bonus;

        if self.effective_miss_count > 0.0 {
            if self.mods.rx() {
                speed_value *= Self::calculate_relax_miss_penalty(
                    self.total_hits(),
                    self.effective_miss_count,
                );
            } else {
                speed_value *= Self::calculate_miss_penalty(
                    self.effective_miss_count,
                    self.attrs.speed_difficult_strain_count,
                );
            }
        }

        let ar_factor = if self.attrs.ar > 10.33 {
            0.3 * (self.attrs.ar - 10.33)
        } else {
            0.0
        };

        // * Buff for longer maps with high AR.
        speed_value *= 1.0 + ar_factor * len_bonus;

        if self.mods.bl() {
            // * Increasing the speed value by object count for Blinds isn't
            // * ideal, so the minimum buff is given.
            speed_value *= 1.12;
        } else if self.mods.hd() || self.mods.tc() {
            // * We want to give more reward for lower AR when it comes to aim and HD.
            // * This nerfs high AR and buffs lower AR.
            speed_value *= 1.0 + 0.04 * (12.0 - self.attrs.ar);
        }

        let speed_high_deviation_mult = self.calculate_speed_high_deviation_nerf(speed_deviation);
        speed_value *= speed_high_deviation_mult;

        // * Calculate accuracy assuming the worst case scenario
        let relevant_total_diff = f64::max(0.0, total_hits - self.attrs.speed_note_count);
        let relevant_n300 = (f64::from(self.state.n300) - relevant_total_diff).max(0.0);
        let relevant_n100 = (f64::from(self.state.n100)
            - (relevant_total_diff - f64::from(self.state.n300)).max(0.0))
        .max(0.0);
        let relevant_n50 = (f64::from(self.state.n50)
            - (relevant_total_diff - f64::from(self.state.n300 + self.state.n100)).max(0.0))
        .max(0.0);

        let relevant_acc = if self.attrs.speed_note_count.eq(0.0) {
            0.0
        } else {
            (relevant_n300 * 6.0 + relevant_n100 * 2.0 + relevant_n50)
                / (self.attrs.speed_note_count * 6.0)
        };

        let od = self.attrs.od();

        // * Scale the speed value with accuracy and OD.
        speed_value *= (0.95 + f64::powf(f64::max(0.0, od), 2.0) / 750.0)
            * f64::powf((self.acc + relevant_acc) / 2.0, (14.5 - od) / 2.0);

        speed_value
    }

    fn compute_accuracy_value(&self) -> f64 {
        // * This percentage only considers HitCircles of any value - in this part
        // * of the calculation we focus on hitting the timing hit window.
        let mut amount_hit_objects_with_acc = self.attrs.n_circles;

        if !self.using_classic_slider_acc {
            amount_hit_objects_with_acc += self.attrs.n_sliders;
        }

        let mut better_acc_percentage = if amount_hit_objects_with_acc > 0 {
            f64::from(
                (self.state.n300 as i32
                    - (i32::max(
                        self.state.total_hits() as i32 - amount_hit_objects_with_acc as i32,
                        0,
                    )))
                    * 6
                    + self.state.n100 as i32 * 2
                    + self.state.n50 as i32,
            ) / f64::from(amount_hit_objects_with_acc * 6)
        } else {
            0.0
        };

        // * It is possible to reach a negative accuracy with this formula. Cap it at zero - zero points.
        if better_acc_percentage < 0.0 {
            better_acc_percentage = 0.0;
        }

        // * Lots of arbitrary values from testing.
        // * Considering to use derivation from perfect accuracy in a probabilistic manner - assume normal distribution.
        let mut acc_value =
            1.52163_f64.powf(self.attrs.od()) * better_acc_percentage.powf(24.0) * 2.83;

        // * Bonus for many hitcircles - it's harder to keep good accuracy up for longer.
        acc_value *= (f64::from(amount_hit_objects_with_acc) / 1000.0)
            .powf(0.3)
            .min(1.15);

        // * Increasing the accuracy value by object count for Blinds isn't
        // * ideal, so the minimum buff is given.
        if self.mods.bl() {
            acc_value *= 1.14;
        } else if self.mods.hd() || self.mods.tc() {
            acc_value *= 1.08;
        }

        if self.mods.fl() {
            acc_value *= 1.02;
        }

        acc_value
    }

    fn calculate_speed_deviation(&self) -> Option<f64> {
        if total_successful_hits(&self.state) == 0 {
            return None;
        }

        // * Calculate accuracy assuming the worst case scenario
        let mut speed_note_count = self.attrs.speed_note_count;
        speed_note_count +=
            (f64::from(self.state.total_hits()) - self.attrs.speed_note_count) * 0.1;

        // * Assume worst case: all mistakes were on speed notes
        let relevant_count_miss = f64::min(f64::from(self.state.misses), speed_note_count);
        let relevant_count_meh = f64::min(
            f64::from(self.state.n50),
            speed_note_count - relevant_count_miss,
        );
        let relevant_count_ok = f64::min(
            f64::from(self.state.n100),
            speed_note_count - relevant_count_miss - relevant_count_meh,
        );
        let relevant_count_great = f64::max(
            0.0,
            speed_note_count - relevant_count_miss - relevant_count_meh - relevant_count_ok,
        );

        self.calculate_deviation(relevant_count_great, relevant_count_ok, relevant_count_meh)
    }

    fn calculate_deviation(
        &self,
        relevant_count_great: f64,
        relevant_count_ok: f64,
        relevant_count_meh: f64,
    ) -> Option<f64> {
        if relevant_count_great + relevant_count_ok + relevant_count_meh <= 0.0 {
            return None;
        }

        // * The sample proportion of successful hits.
        let n = f64::max(1.0, relevant_count_great + relevant_count_ok);
        let p = relevant_count_great / n;

        #[allow(clippy::items_after_statements, clippy::unreadable_literal)]
        const Z: f64 = 2.32634787404; // * 99% critical value for the normal distribution (one-tailed).

        // * We can be 99% confident that the population proportion is at
        // * least this value.
        let p_lower_bound = p.min(
            (n * p + Z * Z / 2.0) / (n + Z * Z)
                - Z / (n + Z * Z) * f64::sqrt(n * p * (1.0 - p) + Z * Z / 4.0),
        );

        let great_hit_window: f64 = self.attrs.great_hit_window;
        let ok_hit_window: f64 = self.attrs.ok_hit_window;
        let meh_hit_window: f64 = self.attrs.meh_hit_window;

        // * Tested max precision for the deviation calculation.
        let deviation = if p_lower_bound > 0.01 {
            // * Compute deviation assuming greats and oks are normally distributed.
            let mut deviation = great_hit_window / (LAZER_SQRT_2 * erf_inv(p_lower_bound));

            // * Subtract the variance provided by tails outside the ok window.
            let ratio = ok_hit_window / deviation;
            let ok_hit_window_tail_amount =
                f64::sqrt(2.0 / PI) * ok_hit_window * f64::exp(-0.5 * ratio * ratio)
                    / (deviation * erf(ok_hit_window / (LAZER_SQRT_2 * deviation)));

            deviation *= f64::sqrt(1.0 - ok_hit_window_tail_amount);
            deviation
        } else {
            // * Tested limit for a score containing only oks.
            ok_hit_window / f64::sqrt(3.0)
        };

        // * Then compute the variance for mehs.
        let meh_variance = (meh_hit_window * meh_hit_window
            + ok_hit_window * meh_hit_window
            + ok_hit_window * ok_hit_window)
            / 3.0;

        // * Find the total deviation.
        let deviation = f64::sqrt(
            ((relevant_count_great + relevant_count_ok) * deviation * deviation
                + relevant_count_meh * meh_variance)
                / (relevant_count_great + relevant_count_ok + relevant_count_meh),
        );

        Some(deviation)
    }

    fn calculate_speed_high_deviation_nerf(&self, speed_deviation: f64) -> f64 {
        let speed_value = Speed::difficulty_to_performance(self.attrs.speed);

        // * Decides a point where the PP value achieved compared to the speed deviation is assumed to be tapped improperly. Any PP above this point is considered "excess" speed difficulty.
        // * This is used to cause PP above the cutoff to scale logarithmically towards the original speed value thus nerfing the value.
        let excess_speed_difficulty_cutoff = 100.0 + 220.0 * f64::powf(22.0 / speed_deviation, 6.5);

        if speed_value <= excess_speed_difficulty_cutoff {
            return 1.0;
        }

        #[allow(clippy::items_after_statements)]
        const SCALE: f64 = 50.0;

        let mut adjusted_speed_value = SCALE
            * (f64::ln((speed_value - excess_speed_difficulty_cutoff) / SCALE + 1.0)
                + excess_speed_difficulty_cutoff / SCALE);

        // * 220 UR and less are considered tapped correctly to ensure that normal scores will be punished as little as possible
        let lerp = 1.0 - reverse_lerp(speed_deviation, 22.0, 27.0);
        adjusted_speed_value = f64::lerp(adjusted_speed_value, speed_value, lerp);

        adjusted_speed_value / speed_value
    }

    // upstream: calculateMissPenalty
    //   `0.93 / (missCount / (4 * Math.Log(difficultStrainCount)) + 1)`
    // 旧 mames は `0.96 / ((miss/(4*ln^0.94)) + 1)` で fork-specific 値、
    // 本家に合わせて `0.93 / ((miss/(4*ln)) + 1)` に戻す。
    fn calculate_miss_penalty(miss_count: f64, diff_strain_count: f64) -> f64 {
        0.93 / (miss_count / (4.0 * diff_strain_count.ln()) + 1.0)
    }

    fn calculate_relax_miss_penalty(total_hits: f64, effective_miss_count: f64) -> f64 {
        (1.0 - (effective_miss_count / total_hits).powf(0.55))
            .powf(1.0 + (effective_miss_count / 2.0))
    }

    fn get_combo_scaling_factor(&self) -> f64 {
        if self.attrs.max_combo == 0 {
            1.0
        } else {
            (f64::from(self.state.max_combo).powf(0.8) / f64::from(self.attrs.max_combo).powf(0.8))
                .min(1.0)
        }
    }

    const fn total_hits(&self) -> f64 {
        self.state.total_hits() as f64
    }

    // ===== upstream OsuPerformanceCalculator.cs 完全一致の vanilla path 実装 =====

    // upstream: computeAimValue
    fn compute_aim_value_vanilla(&self, aim_slider_breaks: f64) -> f64 {
        if self.mods.ap() {
            return 0.0;
        }

        let mut aim_difficulty = self.attrs.aim;

        if self.attrs.n_sliders > 0 && self.attrs.aim_difficult_slider_count > 0.0 {
            let estimate_improperly_followed_difficult_sliders = if self.using_classic_slider_acc {
                let maximum_possible_dropped_sliders = total_imperfect_hits(&self.state);
                f64::clamp(
                    f64::min(
                        maximum_possible_dropped_sliders,
                        f64::from(self.attrs.max_combo - self.state.max_combo),
                    ),
                    0.0,
                    self.attrs.aim_difficult_slider_count,
                )
            } else {
                f64::clamp(
                    f64::from(
                        n_slider_ends_dropped(&self.attrs, &self.state)
                            + n_large_tick_miss(&self.attrs, &self.state),
                    ),
                    0.0,
                    self.attrs.aim_difficult_slider_count,
                )
            };
            let slider_nerf_factor = (1.0 - self.attrs.slider_factor)
                * f64::powf(
                    1.0 - estimate_improperly_followed_difficult_sliders
                        / self.attrs.aim_difficult_slider_count,
                    3.0,
                )
                + self.attrs.slider_factor;
            aim_difficulty *= slider_nerf_factor;
        }

        // upstream: aimValue = DifficultyToPerformance(aimDifficulty) = 4 * pow(aim, 3)
        let mut aim_value = 4.0 * aim_difficulty.powi(3);

        let total_hits = self.total_hits();

        // upstream: 0.95 + 0.35 * min(1, total/2000) + (total>2000 ? log10(...)*0.5 : 0)
        let length_bonus = 0.95
            + 0.35 * (total_hits / 2000.0).min(1.0)
            + if total_hits > 2000.0 {
                (total_hits / 2000.0).log10() * 0.5
            } else {
                0.0
            };
        aim_value *= length_bonus;

        if self.effective_miss_count > 0.0 {
            let relevant_miss_count = f64::min(
                self.effective_miss_count + aim_slider_breaks,
                total_imperfect_hits(&self.state)
                    + f64::from(n_large_tick_miss(&self.attrs, &self.state)),
            );
            aim_value *= Self::calculate_miss_penalty(
                relevant_miss_count,
                self.attrs.aim_difficult_strain_count,
            );
        }

        // Blinds bonus, else Traceable bonus (旧 HD/TC の一律 boost は削除)
        if self.mods.bl() {
            aim_value *= 1.3
                + (total_hits
                    * (0.0016 / (1.0 + 2.0 * self.effective_miss_count))
                    * self.acc.powf(16.0))
                    * (1.0 - 0.003 * self.attrs.hp * self.attrs.hp);
        } else if self.mods.tc() {
            aim_value *= 1.0 + self.calculate_traceable_bonus(self.attrs.slider_factor);
        }

        aim_value *= self.acc;
        aim_value
    }

    // upstream: computeSpeedValue
    fn compute_speed_value_vanilla(
        &self,
        speed_deviation: Option<f64>,
        speed_slider_breaks: f64,
    ) -> f64 {
        let Some(speed_deviation) = speed_deviation else {
            return 0.0;
        };
        // upstream: HarmonicSkill.DifficultyToPerformance(speed) = 4 * pow(speed, 3)
        let mut speed_value = 4.0 * self.attrs.speed.powi(3);

        if self.effective_miss_count > 0.0 {
            let relevant_miss_count = f64::min(
                self.effective_miss_count + speed_slider_breaks,
                total_imperfect_hits(&self.state)
                    + f64::from(n_large_tick_miss(&self.attrs, &self.state)),
            );
            speed_value *= Self::calculate_miss_penalty(
                relevant_miss_count,
                self.attrs.speed_difficult_strain_count,
            );
        }

        if self.mods.bl() {
            speed_value *= 1.12;
        }

        let speed_high_deviation_mult =
            self.calculate_speed_high_deviation_nerf_vanilla(speed_deviation);
        speed_value *= speed_high_deviation_mult;

        // upstream: effectiveHitWindow = 20 * pow(4/speed, 0.35)
        let effective_hit_window = 20.0 * (4.0 / self.attrs.speed).powf(0.35);

        // upstream: effectiveAccuracy = erf(effectiveHitWindow / speedDeviation)
        let effective_accuracy = erf(effective_hit_window / speed_deviation);

        // upstream: speedValue *= pow(effectiveAccuracy, 2)
        speed_value *= effective_accuracy.powi(2);

        speed_value
    }

    // upstream: computeAccuracyValue
    fn compute_accuracy_value_vanilla(&self) -> f64 {
        let mut amount_hit_objects_with_acc = self.attrs.n_circles;
        if !self.using_classic_slider_acc || self.mods.has_score_v2() {
            amount_hit_objects_with_acc += self.attrs.n_sliders;
        }

        let total_hits = self.state.total_hits() as i32;
        let better_acc_percentage = if amount_hit_objects_with_acc > 0 {
            let num = (self.state.n300 as i32
                - i32::max(total_hits - amount_hit_objects_with_acc as i32, 0))
                * 6
                + self.state.n100 as i32 * 2
                + self.state.n50 as i32;
            f64::from(num) / f64::from(amount_hit_objects_with_acc * 6)
        } else {
            0.0
        };
        let better_acc_percentage = better_acc_percentage.max(0.0);

        // upstream: overallDifficulty = (79.5 - greatHitWindow) / 6
        let overall_difficulty = (79.5 - self.attrs.great_hit_window) / 6.0;

        // upstream: pow(1.52163, OD) * pow(betterAccPercentage, 24) * 2.83
        let mut acc_value =
            1.52163_f64.powf(overall_difficulty) * better_acc_percentage.powf(24.0) * 2.83;

        // upstream: length bonus (< 1000 → pow(x, 0.3), ≥ 1000 → pow(x, 0.1))
        let ratio = f64::from(amount_hit_objects_with_acc) / 1000.0;
        acc_value *= if amount_hit_objects_with_acc < 1000 {
            ratio.powf(0.3)
        } else {
            ratio.powf(0.1)
        };

        // upstream: Blinds + Traceable の bonus のみ (HD/FL boost は削除)
        if self.mods.bl() {
            acc_value *= 1.14;
        } else if self.mods.tc() {
            // AR > 10 で bonus を減衰
            let approach_rate = self.attrs.ar;
            acc_value *= 1.0 + 0.08 * reverse_lerp(approach_rate, 11.5, 10.0);
        }

        acc_value
    }

    // upstream: computeFlashlightValue
    fn compute_flashlight_value_vanilla(&self) -> f64 {
        if !self.mods.fl() {
            return 0.0;
        }

        // upstream: Flashlight.DifficultyToPerformance = 25 * pow(difficulty, 2)
        let mut flashlight_value = 25.0 * self.attrs.flashlight.powi(2);

        let total_hits = self.total_hits();

        if self.effective_miss_count > 0.0 {
            flashlight_value *= 0.97
                * (1.0 - (self.effective_miss_count / total_hits).powf(0.775))
                    .powf(self.effective_miss_count.powf(0.875));
        }

        flashlight_value *= self.get_combo_scaling_factor();

        // upstream: 0.5 + accuracy / 2.0 のみ (旧 length bonus と OD 補正は削除)
        flashlight_value *= 0.5 + self.acc / 2.0;

        flashlight_value
    }

    // upstream: computeReadingValue
    fn compute_reading_value_vanilla(&self, aim_slider_breaks: f64) -> f64 {
        // upstream: HarmonicSkill.DifficultyToPerformance(reading)
        let mut reading_value = 4.0 * self.attrs.reading.powi(3);

        if self.effective_miss_count > 0.0 {
            reading_value *= Self::calculate_miss_penalty(
                self.effective_miss_count + aim_slider_breaks,
                self.attrs.reading_difficult_note_count,
            );
        }

        // upstream: reading value scales HARSHLY with accuracy
        reading_value *= self.acc.powi(3);

        reading_value
    }

    // upstream: calculateEstimatedSliderBreaks
    fn calculate_estimated_slider_breaks(&self, top_weighted_slider_factor: f64) -> f64 {
        let non_miss_mistakes = i32::from(self.state.n100 != 0) * self.state.n100 as i32
            + i32::from(self.state.n50 != 0) * self.state.n50 as i32;
        let non_miss_mistakes = f64::from(non_miss_mistakes);

        if !self.using_classic_slider_acc || non_miss_mistakes == 0.0 {
            return 0.0;
        }

        let missed_combo_percent =
            1.0 - f64::from(self.state.max_combo) / f64::from(self.attrs.max_combo);
        let mut estimated = f64::min(
            non_miss_mistakes,
            self.effective_miss_count * top_weighted_slider_factor,
        );

        let non_miss_adjustment = (non_miss_mistakes - estimated + 4.5) / (non_miss_mistakes + 4.0);
        estimated *= crate::util::difficulty::smoothstep(self.effective_miss_count, 1.0, 2.0);

        estimated
            * non_miss_adjustment
            * crate::util::difficulty::logistic(missed_combo_percent, 0.33, 15.0, None)
    }

    // upstream: calculateTraceableBonus
    fn calculate_traceable_bonus(&self, slider_factor: f64) -> f64 {
        let approach_rate = self.attrs.ar;
        let high_ar_slider_visibility = 0.5 + slider_factor.powi(6) / 2.0;
        let low_ar_slider_visibility = slider_factor.powi(6);

        let mut traceable_bonus = 0.0275;
        traceable_bonus +=
            0.025 * (12.0 - f64::max(approach_rate, 7.0)) * high_ar_slider_visibility;

        if approach_rate < 7.0 {
            traceable_bonus +=
                0.025 * (7.0 - f64::max(approach_rate, 0.0)) * low_ar_slider_visibility;
        }
        if approach_rate < 0.0 {
            traceable_bonus +=
                0.025 * (1.0 - 1.5_f64.powf(approach_rate)) * low_ar_slider_visibility;
        }

        traceable_bonus
    }

    // upstream: calculateSpeedHighDeviationNerf (identical to existing method but using
    // HarmonicSkill.DifficultyToPerformance = 4 * pow(speed, 3) instead of Speed::difficulty_to_performance)
    fn calculate_speed_high_deviation_nerf_vanilla(&self, speed_deviation: f64) -> f64 {
        let speed_value = 4.0 * self.attrs.speed.powi(3);
        let excess_cutoff = 100.0 + 220.0 * (22.0 / speed_deviation).powf(6.5);

        if speed_value <= excess_cutoff {
            return 1.0;
        }

        const SCALE: f64 = 50.0;
        let mut adjusted =
            SCALE * ((((speed_value - excess_cutoff) / SCALE) + 1.0).ln() + excess_cutoff / SCALE);

        let lerp = 1.0 - reverse_lerp(speed_deviation, 22.0, 27.0);
        adjusted = f64::lerp(adjusted, speed_value, lerp);

        adjusted / speed_value
    }
}

// upstream: DiffUtils.Norm
fn norm_pnorm(p: f64, values: &[f64]) -> f64 {
    let sum: f64 = values.iter().map(|v| v.powf(p)).sum();
    sum.powf(1.0 / p)
}

// upstream: OsuDifficultyCalculator.SumCognitionDifficulty
fn sum_cognition_difficulty(reading: f64, flashlight: f64) -> f64 {
    if reading <= 0.0 {
        return flashlight;
    }
    if flashlight <= 0.0 {
        return reading;
    }
    // upstream: Norm(1.1, reading, flashlight * clamp(flashlight/reading, 0.25, 1.0))
    let flashlight_scaled = flashlight * (flashlight / reading).clamp(0.25, 1.0);
    norm_pnorm(PERFORMANCE_NORM_EXPONENT, &[reading, flashlight_scaled])
}

const fn total_successful_hits(state: &OsuScoreState) -> u32 {
    state.n300 + state.n100 + state.n50
}

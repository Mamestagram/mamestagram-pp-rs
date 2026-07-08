//! upstream `osu.Game.Rulesets.Osu.Difficulty.Skills.Reading` の Rust 移植。
//!
//! HarmonicSkill を継承した independent な skill (StrainSkill ではないので
//! define_skill! macro は使わず自前構造)。per-object の difficulty を貯め、
//! `difficulty_value()` で harmonic sum を計算する。
//!
//! 依存: `ReadingEvaluator` (同ファイル内)、`OsuDifficultyObject.opacity_at`、
//! `angle`, `lazy_jump_dist`, `strain_time`。
//!
//! `preempt` / `overall_difficulty` は map-wide 定数として skill 側で保持。
//! upstream の `OsuDifficultyHitObject.Preempt/OverallDifficulty` は per-object
//! だが、mames-pp では convert 後の map から一括計算するのでこの近似で OK。

use crate::{
    any::difficulty::object::IDifficultyObject,
    osu::difficulty::object::OsuDifficultyObject,
    util::difficulty::{logistic, reverse_lerp, smootherstep},
    GameMods,
};

pub struct Reading {
    /// per-object 難易度 (upstream: `ObjectDifficulties`)。
    object_difficulties: Vec<f64>,
    /// per-object の start_time (reduced_note_count 計算用)。
    object_start_times: Vec<f64>,
    /// 現在の strain (前 object との decay を反映)。
    current_strain: f64,

    // mod flags
    has_hidden_mod: bool,
    has_touch_device_mod: bool,
    has_relax_mod: bool,
    has_autopilot_mod: bool,
    /// upstream OsuModMagnetised.AttractionStrength (0..1)。未対応の場合 0.0。
    magnetised_strength: f64,

    // evaluator 側で使う map-wide 定数
    preempt: f64,
    overall_difficulty: f64,

    /// evaluator 経由で `opacity_at` に必要
    time_fade_in: f64,

    /// 遅延計算用の `ObjectWeightSum` (harmonic sum で使う)。
    /// `difficulty_value()` を呼んだ時点で更新される。
    object_weight_sum: f64,
}

impl Reading {
    /// upstream: `skill_multiplier`
    const SKILL_MULTIPLIER: f64 = 2.5;
    /// upstream: `strainDecay` の base
    const STRAIN_DECAY_BASE: f64 = 0.8;

    /// HarmonicSkill defaults
    const HARMONIC_SCALE: f64 = 1.0;
    const DECAY_EXPONENT: f64 = 0.9;

    /// upstream: `reduced_difficulty_duration = 60 * 1000` (60 秒間 memorize と仮定)
    const REDUCED_DIFFICULTY_DURATION: f64 = 60_000.0;

    pub fn new(
        mods: &GameMods,
        preempt: f64,
        overall_difficulty: f64,
        time_fade_in: f64,
    ) -> Self {
        Self {
            object_difficulties: Vec::new(),
            object_start_times: Vec::new(),
            current_strain: 0.0,
            has_hidden_mod: mods.hd(),
            has_touch_device_mod: mods.td(),
            has_relax_mod: mods.rx(),
            has_autopilot_mod: mods.ap(),
            magnetised_strength: 0.0, // 未サポート mod、後で拡張可能
            preempt,
            overall_difficulty,
            time_fade_in,
            object_weight_sum: 0.0,
        }
    }

    /// upstream: `Skill.Process` → `ProcessInternal` → `ObjectDifficultyOf`
    pub fn process(&mut self, curr: &OsuDifficultyObject<'_>, objects: &[OsuDifficultyObject<'_>]) {
        // upstream: strainDecay(ms) = pow(0.8, ms/1000)
        let decay = Self::STRAIN_DECAY_BASE.powf(curr.delta_time / 1000.0);
        self.current_strain *= decay;
        self.current_strain += self.calculate_adjusted_difficulty(curr, objects)
            * (1.0 - decay)
            * Self::SKILL_MULTIPLIER;

        self.object_difficulties.push(self.current_strain);
        self.object_start_times.push(curr.start_time);
    }

    fn calculate_adjusted_difficulty(
        &self,
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
    ) -> f64 {
        let mut difficulty = ReadingEvaluator::evaluate_diff_of(
            curr,
            objects,
            self.has_hidden_mod,
            self.preempt,
            self.time_fade_in,
        );

        if self.has_touch_device_mod {
            difficulty = difficulty.powf(0.89);
        }
        if self.magnetised_strength > 0.0 {
            difficulty *= 1.0 - self.magnetised_strength;
        }
        if self.has_relax_mod {
            difficulty *= 0.4;
        }
        if self.has_autopilot_mod {
            difficulty *= 0.1;
        }

        // upstream: `0.825 + Pow(max(0, OD), 2.2) / 1125.0`
        difficulty *= 0.825
            + f64::max(0.0, self.overall_difficulty).powf(2.2) / 1125.0;

        difficulty
    }

    /// upstream: `GetTransformedDifficulties`
    /// 最初の N 個 (map の 60 秒間分) を memorize として弱く。
    fn get_transformed_difficulties(&self) -> Vec<f64> {
        // 0 を除外
        let mut difficulties: Vec<f64> = self
            .object_difficulties
            .iter()
            .copied()
            .filter(|v| *v > 0.0)
            .collect();

        // upstream: reduced_difficulty_base_line = 0.0
        let reduced_note_count = self.calculate_reduced_note_count();

        let take = difficulties.len().min(reduced_note_count);
        for i in 0..take {
            // upstream: scale = log10(lerp(1, 10, clamp(i/reducedNoteCount, 0, 1)))
            let ratio = (i as f64 / reduced_note_count as f64).clamp(0.0, 1.0);
            let scale = (1.0 + ratio * 9.0).log10(); // = log10(lerp(1, 10, ratio))
            // upstream: lerp(reduced_difficulty_base_line, 1.0, scale) = scale (base = 0)
            difficulties[i] *= scale;
        }
        difficulties
    }

    fn calculate_reduced_note_count(&self) -> usize {
        if self.object_start_times.is_empty() {
            return 0;
        }
        let start = self.object_start_times[0];
        let cutoff = start + Self::REDUCED_DIFFICULTY_DURATION;
        self.object_start_times
            .iter()
            .take_while(|&&t| t <= cutoff)
            .count()
    }

    /// upstream: `HarmonicSkill.DifficultyValue`
    pub fn difficulty_value(&mut self) -> f64 {
        if self.object_difficulties.is_empty() {
            return 0.0;
        }

        let mut difficulties = self.get_transformed_difficulties();
        if difficulties.is_empty() {
            return 0.0;
        }
        // 降順 sort
        difficulties.sort_by(|a, b| b.total_cmp(a));

        let mut difficulty = 0.0;
        let mut object_weight_sum = 0.0;
        for (index, &obj) in difficulties.iter().enumerate() {
            if obj <= 0.0 {
                break;
            }
            let idx = index as f64;
            // upstream:
            //   weight = (1 + HarmonicScale/(1+i))
            //          / (Pow(i, DecayExponent) + 1 + HarmonicScale/(1+i))
            let harm_term = Self::HARMONIC_SCALE / (1.0 + idx);
            let weight = (1.0 + harm_term) / (idx.powf(Self::DECAY_EXPONENT) + 1.0 + harm_term);

            object_weight_sum += weight;
            difficulty += obj * weight;
        }

        self.object_weight_sum = object_weight_sum;
        difficulty
    }

    /// upstream: `Reading.CountTopWeightedObjectDifficulties` (Reading 用の override)
    /// `difficulty_value()` を先に呼んでおく必要がある (object_weight_sum を使うため)。
    pub fn count_top_weighted_object_difficulties(&self, difficulty_value: f64) -> f64 {
        if self.object_difficulties.is_empty() || self.object_weight_sum == 0.0 {
            return 0.0;
        }
        let consistent_top_note = difficulty_value / self.object_weight_sum;
        if consistent_top_note == 0.0 {
            return 0.0;
        }
        // upstream: sum(Logistic(d / consistentTopNote, 1.15, 5, 1.1))
        self.object_difficulties
            .iter()
            .map(|d| logistic(d / consistent_top_note, 1.15, 5.0, Some(1.1)))
            .sum()
    }

    /// 現状の difficulty_value を pure 参照から計算する版 (state は変えない)。
    pub fn cloned_difficulty_value(&self) -> f64 {
        let mut cloned = self.clone_for_eval();
        cloned.difficulty_value()
    }

    /// difficulty_value + count_top_weighted_object_difficulties を両方 evaluate したいとき
    /// 用の mut clone。呼び出し側で mut に持って両 API を叩ける。
    pub fn clone_for_eval(&self) -> Reading {
        Reading {
            object_difficulties: self.object_difficulties.clone(),
            object_start_times: self.object_start_times.clone(),
            current_strain: self.current_strain,
            has_hidden_mod: self.has_hidden_mod,
            has_touch_device_mod: self.has_touch_device_mod,
            has_relax_mod: self.has_relax_mod,
            has_autopilot_mod: self.has_autopilot_mod,
            magnetised_strength: self.magnetised_strength,
            preempt: self.preempt,
            overall_difficulty: self.overall_difficulty,
            time_fade_in: self.time_fade_in,
            object_weight_sum: 0.0,
        }
    }

    /// upstream: HarmonicSkill.DifficultyToPerformance = 4 * pow(d, 3)
    #[allow(dead_code)]
    pub fn difficulty_to_performance(difficulty: f64) -> f64 {
        4.0 * difficulty.powi(3)
    }
}

// ────────────────────────────────────────────────────────────────────────────
// upstream `osu.Game.Rulesets.Osu.Difficulty.Evaluators.ReadingEvaluator` の port
// ────────────────────────────────────────────────────────────────────────────

struct ReadingEvaluator;

impl ReadingEvaluator {
    const READING_WINDOW_SIZE: f64 = 3000.0; // 3 秒
    // upstream: NORMALISED_DIAMETER * 1.5 (直径の 1.5 倍)
    const DISTANCE_INFLUENCE_THRESHOLD: f64 =
        (OsuDifficultyObject::NORMALIZED_DIAMETER as f64) * 1.5;

    fn evaluate_diff_of(
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
        hidden: bool,
        preempt: f64,
        time_fade_in: f64,
    ) -> f64 {
        if curr.base.is_spinner() || curr.idx == 0 {
            return 0.0;
        }
        let next_obj = curr.next(0, objects);

        // upstream: `Max(1, LazyJumpDistance / AdjustedDeltaTime)` (buff only)
        let velocity = f64::max(1.0, curr.lazy_jump_dist / curr.strain_time);

        let current_visible_object_density =
            Self::retrieve_current_visible_object_density(curr, objects, preempt);
        let past_object_difficulty_influence =
            Self::get_past_object_difficulty_influence(curr, objects, preempt);
        let constant_angle_nerf_factor = Self::get_constant_angle_nerf_factor(curr, objects);

        let note_density_difficulty = Self::calculate_density_difficulty(
            next_obj,
            velocity,
            constant_angle_nerf_factor,
            past_object_difficulty_influence,
            current_visible_object_density,
        );

        let hidden_difficulty = if hidden {
            Self::calculate_hidden_difficulty(
                curr,
                objects,
                past_object_difficulty_influence,
                current_visible_object_density,
                velocity,
                constant_angle_nerf_factor,
                preempt,
                time_fade_in,
            )
        } else {
            0.0
        };

        let preempt_difficulty =
            Self::calculate_preempt_difficulty(velocity, constant_angle_nerf_factor, preempt);

        // upstream: `Norm(1.5, preempt, hidden, noteDensity)`
        let reading_difficulty = norm_p(
            1.5,
            &[preempt_difficulty, hidden_difficulty, note_density_difficulty],
        );

        // upstream: `readingDifficulty *= highBpmBonus(AdjustedDeltaTime)`
        reading_difficulty * Self::high_bpm_bonus(curr.strain_time)
    }

    fn calculate_density_difficulty(
        next_obj: Option<&OsuDifficultyObject<'_>>,
        velocity: f64,
        constant_angle_nerf_factor: f64,
        past_object_difficulty_influence: f64,
        current_visible_object_density: f64,
    ) -> f64 {
        const DENSITY_MULTIPLIER: f64 = 2.4;
        const DENSITY_DIFFICULTY_BASE: f64 = 2.5;

        // upstream: sqrt(currentVisibleObjectDensity)
        let mut future_object_difficulty_influence = current_visible_object_density.sqrt();
        if let Some(next) = next_obj {
            // upstream: smootherstep(next.LazyJumpDistance, 15, distance_influence_threshold)
            future_object_difficulty_influence *= smootherstep(
                next.lazy_jump_dist,
                15.0,
                Self::DISTANCE_INFLUENCE_THRESHOLD,
            );
        }

        // upstream: pow(past + future, 1.7) * 0.4 * constAngleNerf * velocity
        let mut note_density_difficulty =
            (past_object_difficulty_influence + future_object_difficulty_influence).powf(1.7)
                * 0.4
                * constant_angle_nerf_factor
                * velocity;

        note_density_difficulty = f64::max(0.0, note_density_difficulty - DENSITY_DIFFICULTY_BASE);
        note_density_difficulty = note_density_difficulty.powf(0.45) * DENSITY_MULTIPLIER;
        note_density_difficulty
    }

    fn calculate_preempt_difficulty(
        velocity: f64,
        constant_angle_nerf_factor: f64,
        preempt: f64,
    ) -> f64 {
        const PREEMPT_BALANCING_FACTOR: f64 = 140_000.0;
        const PREEMPT_STARTING_POINT: f64 = 500.0; // AR 9.66 (ms)

        // upstream: `(preemptStart - preempt + Abs(preempt - preemptStart)) / 2` = max(preemptStart - preempt, 0)
        // (これは平滑 relu の形)
        let shifted = (PREEMPT_STARTING_POINT - preempt + (preempt - PREEMPT_STARTING_POINT).abs())
            / 2.0;
        let mut preempt_difficulty = shifted.powf(2.5) / PREEMPT_BALANCING_FACTOR;
        preempt_difficulty *= constant_angle_nerf_factor * velocity;
        preempt_difficulty
    }

    #[allow(clippy::too_many_arguments)]
    fn calculate_hidden_difficulty(
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
        past_object_difficulty_influence: f64,
        current_visible_object_density: f64,
        velocity: f64,
        constant_angle_nerf_factor: f64,
        preempt: f64,
        time_fade_in: f64,
    ) -> f64 {
        const HIDDEN_MULTIPLIER: f64 = 0.28;

        let preempt_factor = preempt.powf(2.2) * 0.01;
        let density_factor =
            (current_visible_object_density + past_object_difficulty_influence).powf(3.3) * 3.0;

        let mut hidden_difficulty =
            (preempt_factor + density_factor) * constant_angle_nerf_factor * velocity * 0.01;
        hidden_difficulty = hidden_difficulty.powf(0.4) * HIDDEN_MULTIPLIER;

        // upstream: 完全 stack (LazyJumpDist == 0) の bonus
        let Some(prev) = curr.previous(0, objects) else {
            return hidden_difficulty;
        };
        if curr.lazy_jump_dist == 0.0
            && curr.opacity_at(prev.base.start_time, true, preempt, time_fade_in) == 0.0
            && prev.start_time > curr.start_time - preempt
        {
            hidden_difficulty += HIDDEN_MULTIPLIER * 2500.0 / curr.strain_time.powf(1.5);
        }
        hidden_difficulty
    }

    fn get_past_object_difficulty_influence(
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
        preempt: f64,
    ) -> f64 {
        let mut influence = 0.0;
        // upstream: retrievePastVisibleObjects
        for i in 0..curr.idx {
            let Some(loop_obj) = curr.previous(i, objects) else {
                break;
            };
            // reading window / preempt 判定 → 抜け
            if curr.start_time - loop_obj.start_time > Self::READING_WINDOW_SIZE
                || loop_obj.start_time < curr.start_time - preempt
            {
                break;
            }
            let mut loop_difficulty = curr.opacity_at(loop_obj.base.start_time, false, preempt, 0.0);
            loop_difficulty *= smootherstep(
                loop_obj.lazy_jump_dist,
                15.0,
                Self::DISTANCE_INFLUENCE_THRESHOLD,
            );
            let time_delta = curr.start_time - loop_obj.start_time;
            loop_difficulty *= Self::get_time_nerf_factor(time_delta);
            influence += loop_difficulty;
        }
        influence
    }

    fn retrieve_current_visible_object_density(
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
        preempt: f64,
    ) -> f64 {
        let mut density = 0.0;
        let mut idx_offset = 0usize;
        loop {
            let Some(loop_obj) = curr.next(idx_offset, objects) else {
                break;
            };
            let dt = loop_obj.start_time - curr.start_time;
            if dt > Self::READING_WINDOW_SIZE || curr.start_time < loop_obj.start_time - preempt {
                break;
            }
            let time_nerf = Self::get_time_nerf_factor(dt);
            // upstream: loopObj.OpacityAt(current.BaseObject.StartTime, false)
            // ← ここは loopObj の opacity_at を curr の base start_time で見る
            density += loop_obj.opacity_at(curr.base.start_time, false, preempt, 0.0) * time_nerf;
            idx_offset += 1;
        }
        density
    }

    /// upstream: `getConstantAngleNerfFactor`。angle が繰り返される (くり返しリズム)
    /// パターンを nerf する。
    fn get_constant_angle_nerf_factor(
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
    ) -> f64 {
        const MINIMUM_ANGLE_RELEVANCY_TIME: f64 = 2000.0;
        const MAXIMUM_ANGLE_RELEVANCY_TIME: f64 = 200.0;

        let mut constant_angle_count = 0.0;
        let mut idx = 0;
        let mut current_time_gap = 0.0;

        // Chain to keep previous 3 objects (for alternating angle detection)
        let mut prev0_idx: Option<usize> = None; // most recent
        let mut prev1_idx: Option<usize> = None;
        let mut prev2_idx: Option<usize> = None;
        // seed: prev0 = curr itself
        let curr_idx_in_seed = curr.idx;

        while current_time_gap < MINIMUM_ANGLE_RELEVANCY_TIME {
            let Some(loop_obj) = curr.previous(idx, objects) else {
                break;
            };
            let long_interval_factor = 1.0
                - reverse_lerp(
                    loop_obj.strain_time,
                    MAXIMUM_ANGLE_RELEVANCY_TIME,
                    MINIMUM_ANGLE_RELEVANCY_TIME,
                );

            if let (Some(loop_angle), Some(curr_angle)) = (loop_obj.angle, curr.angle) {
                let angle_difference = (curr_angle - loop_angle).abs();
                let mut angle_difference_alternating = std::f64::consts::PI;

                // alternating detection needs 3 anchor angles + curr
                let seed_or = |i: Option<usize>| -> Option<&OsuDifficultyObject<'_>> {
                    match i {
                        Some(seed) if seed == curr_idx_in_seed => Some(curr),
                        Some(seed) => objects.get(seed),
                        None => None,
                    }
                };
                if let (Some(p0), Some(p1), Some(p2)) =
                    (seed_or(prev0_idx.or(Some(curr_idx_in_seed))), seed_or(prev1_idx), seed_or(prev2_idx))
                {
                    if let (Some(p0_ang), Some(p1_ang), Some(p2_ang)) =
                        (p0.angle, p1.angle, p2.angle)
                    {
                        angle_difference_alternating =
                            (p1_ang - loop_angle).abs() + (p2_ang - p0_ang).abs();
                        let mut weight = 1.0;
                        // どちらかの angle が sharp/wide を要求
                        weight *= reverse_lerp(
                            f64::min(loop_angle, p0_ang) * 180.0 / std::f64::consts::PI,
                            20.0,
                            5.0,
                        );
                        weight *= reverse_lerp(
                            f64::max(loop_angle, p0_ang) * 180.0 / std::f64::consts::PI,
                            60.0,
                            120.0,
                        );
                        angle_difference_alternating =
                            (std::f64::consts::PI) * (1.0 - weight) + 0.1 * angle_difference_alternating * weight;
                    }
                }

                let stack_factor = smootherstep(
                    loop_obj.lazy_jump_dist,
                    0.0,
                    OsuDifficultyObject::NORMALIZED_RADIUS as f64,
                );

                let angle_min = f64::min(angle_difference, angle_difference_alternating);
                let clamped_angle = f64::min(30.0_f64.to_radians(), angle_min * stack_factor);
                constant_angle_count += (3.0 * clamped_angle).cos() * long_interval_factor;
            }

            current_time_gap = curr.start_time - loop_obj.start_time;
            idx += 1;

            prev2_idx = prev1_idx;
            prev1_idx = prev0_idx;
            prev0_idx = Some(loop_obj.idx);
        }

        (2.0 / constant_angle_count).clamp(0.2, 1.0)
    }

    fn get_time_nerf_factor(delta_time: f64) -> f64 {
        (2.0 - delta_time / (Self::READING_WINDOW_SIZE / 2.0)).clamp(0.0, 1.0)
    }

    fn high_bpm_bonus(ms: f64) -> f64 {
        1.0 / (1.0 - 0.8_f64.powf(ms / 1000.0))
    }
}

// upstream: `DiffUtils.Norm(p, values)` の local 版 (calculator.rs に private があるが再定義)
fn norm_p(p: f64, values: &[f64]) -> f64 {
    let sum: f64 = values.iter().map(|v| v.powf(p)).sum();
    sum.powf(1.0 / p)
}

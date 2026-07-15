use std::cmp;

use crate::{
    any::difficulty::object::IDifficultyObject,
    model::mods::GameMods,
    osu::difficulty::object::OsuDifficultyObject,
    util::{
        difficulty::{
            bpm_to_milliseconds, logistic, milliseconds_to_bpm, reverse_lerp, smoothstep_bell_curve,
        },
        strains_vec::StrainsVec,
    },
};

/// lazer's harmonic speed skill (`20260706`).
#[derive(Clone)]
pub struct Speed {
    current_strain: f64,
    hit_window: f64,
    has_relax_mod: bool,
    has_autopilot_mod: bool,
    object_difficulties: Vec<f64>,
    slider_strains: Vec<f64>,
}

impl Speed {
    const HARMONIC_SCALE: f64 = 20.0;
    const DECAY_EXPONENT: f64 = 0.9;

    pub fn new(hit_window: f64, mods: &GameMods) -> Self {
        Self {
            current_strain: 0.0,
            hit_window,
            has_relax_mod: mods.rx(),
            has_autopilot_mod: mods.ap(),
            object_difficulties: Vec::with_capacity(256),
            slider_strains: Vec::with_capacity(64),
        }
    }

    pub fn process(&mut self, curr: &OsuDifficultyObject<'_>, objects: &[OsuDifficultyObject<'_>]) {
        let difficulty = self.object_difficulty_of(curr, objects);
        self.object_difficulties.push(difficulty);
    }

    fn object_difficulty_of(
        &mut self,
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
    ) -> f64 {
        if self.has_relax_mod {
            return 0.0;
        }

        let decay = 0.3_f64.powf(curr.strain_time / 1000.0);
        self.current_strain *= decay;

        let mut adjusted_difficulty =
            SpeedEvaluator::evaluate_diff_of(curr, objects, self.hit_window);

        if self.has_autopilot_mod {
            adjusted_difficulty *= 0.5;
        }

        self.current_strain += adjusted_difficulty * (1.0 - decay) * 1.16;

        let rhythm = RhythmEvaluator::evaluate_diff_of(curr, objects, self.hit_window);
        let total_strain = self.current_strain * rhythm;

        if curr.base.is_slider() {
            self.slider_strains.push(total_strain);
        }

        total_strain
    }

    fn harmonic_sum(&self) -> (f64, f64) {
        let mut difficulties: Vec<_> = self
            .object_difficulties
            .iter()
            .copied()
            .filter(|value| *value > 0.0)
            .collect();
        difficulties.sort_by(|a, b| b.total_cmp(a));

        let mut difficulty = 0.0;
        let mut object_weight_sum = 0.0;

        for (index, object) in difficulties.into_iter().enumerate() {
            let index = index as f64;
            let harmonic = Self::HARMONIC_SCALE / (1.0 + index);
            let weight = (1.0 + harmonic) / (index.powf(Self::DECAY_EXPONENT) + 1.0 + harmonic);

            object_weight_sum += weight;
            difficulty += object * weight;
        }

        (difficulty, object_weight_sum)
    }

    pub fn cloned_difficulty_value(&self) -> f64 {
        self.harmonic_sum().0
    }

    pub fn count_top_weighted_strains(&self, difficulty_value: f64) -> f64 {
        if self.object_difficulties.is_empty() {
            return 0.0;
        }

        let object_weight_sum = self.harmonic_sum().1;

        if object_weight_sum == 0.0 {
            return 0.0;
        }

        let consistent_top_object = difficulty_value / object_weight_sum;

        if consistent_top_object == 0.0 {
            return 0.0;
        }

        self.object_difficulties
            .iter()
            .map(|difficulty| logistic(difficulty / consistent_top_object, 0.88, 10.0, Some(1.1)))
            .sum()
    }

    pub fn count_top_weighted_sliders(&self, difficulty_value: f64) -> f64 {
        if self.slider_strains.is_empty() {
            return 0.0;
        }

        let object_weight_sum = self.harmonic_sum().1;

        if object_weight_sum == 0.0 {
            return 0.0;
        }

        let consistent_top_object = difficulty_value / object_weight_sum;

        if consistent_top_object == 0.0 {
            return 0.0;
        }

        self.slider_strains
            .iter()
            .map(|strain| logistic(strain / consistent_top_object, 0.88, 10.0, Some(1.1)))
            .sum()
    }

    pub fn relevant_note_count(&self) -> f64 {
        let Some(max_strain) = self.object_difficulties.iter().copied().reduce(f64::max) else {
            return 0.0;
        };

        if max_strain == 0.0 {
            return 0.0;
        }

        self.object_difficulties
            .iter()
            .map(|strain| 1.0 / (1.0 + (-(strain / max_strain * 12.0 - 6.0)).exp()))
            .sum()
    }

    pub fn into_current_strain_peaks(self) -> StrainsVec {
        // Harmonic skills no longer have fixed 400ms peaks. Preserve the public
        // strains API by returning their per-object strains in processing order.
        let mut strains = StrainsVec::with_capacity(self.object_difficulties.len());

        for strain in self.object_difficulties {
            strains.push(strain);
        }

        strains
    }

    pub fn difficulty_to_performance(difficulty: f64) -> f64 {
        4.0 * difficulty.powi(3)
    }
}

struct SpeedEvaluator;

impl SpeedEvaluator {
    fn evaluate_diff_of(
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
        hit_window: f64,
    ) -> f64 {
        if curr.base.is_spinner() {
            return 0.0;
        }

        let mut strain_time = curr.strain_time;
        let double_tap_feasibility =
            1.0 - curr.get_doubletapness(curr.next(0, objects), hit_window);

        strain_time /= ((strain_time / hit_window) / 0.93).clamp(0.92, 1.0);

        let speed_bonus = if milliseconds_to_bpm(strain_time, None) > 200.0 {
            0.75 * ((bpm_to_milliseconds(200.0, None) - strain_time) / 40.0).powi(2)
        } else {
            0.0
        };

        let mut difficulty = (1.0 + speed_bonus) * 1000.0 / strain_time;
        difficulty *= 1.0 / (1.0 - 0.3_f64.powf(curr.strain_time / 1000.0));

        difficulty * double_tap_feasibility
    }
}

struct RhythmEvaluator;

impl RhythmEvaluator {
    const HISTORY_TIME_MAX: f64 = 5000.0;
    const HISTORY_OBJECTS_MAX: usize = 32;

    #[allow(clippy::too_many_lines)]
    fn evaluate_diff_of(
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
        hit_window: f64,
    ) -> f64 {
        if curr.base.is_spinner() {
            return 0.0;
        }

        let mut rhythm_complexity_sum = 0.0;
        let delta_difference_epsilon = hit_window * 0.3;
        let mut island = RhythmIsland::new(i32::MAX);
        let mut previous_island = RhythmIsland::new(i32::MAX);
        let mut islands = Vec::<RhythmIsland>::new();
        let mut start_difficulty = 0.0;
        let mut first_delta_switch = false;
        let historical_note_count = curr.idx.min(Self::HISTORY_OBJECTS_MAX);
        let mut rhythm_start = 0;

        while rhythm_start + 2 < historical_note_count
            && curr
                .previous(rhythm_start, objects)
                .is_some_and(|previous| {
                    curr.start_time - previous.start_time < Self::HISTORY_TIME_MAX
                })
        {
            rhythm_start += 1;
        }

        let Some(mut previous_object) = curr.previous(rhythm_start, objects) else {
            return 1.0;
        };
        let Some(mut previous_previous_object) = curr.previous(rhythm_start + 1, objects) else {
            return 1.0;
        };

        for i in (1..=rhythm_start).rev() {
            let Some(current_object) = curr.previous(i - 1, objects) else {
                break;
            };

            if current_object.base.is_spinner() {
                continue;
            }

            let time_decay = (Self::HISTORY_TIME_MAX
                - (curr.start_time - current_object.start_time))
                / Self::HISTORY_TIME_MAX;
            let note_decay = (historical_note_count - i) as f64 / historical_note_count as f64;
            let current_historical_decay = note_decay.min(time_decay);

            let current_delta = current_object.delta_time.max(1e-7);
            let previous_delta = previous_object.delta_time.max(1e-7);
            let delta_difference = (previous_delta - current_delta).abs();

            if island.delta == i32::MAX {
                island = RhythmIsland::new(current_delta as i32);
            }

            let delta_difference_ratio =
                previous_delta.max(current_delta) / previous_delta.min(current_delta);
            let difference_multiplier = (2.0 - delta_difference_ratio / 8.0).clamp(0.0, 1.0);
            let window_penalty = ((delta_difference - delta_difference_epsilon)
                / delta_difference_epsilon)
                .clamp(0.0, 1.0);
            let mut effective_difficulty = Self::effective_difficulty(delta_difference_ratio)
                * window_penalty
                * difference_multiplier;

            if previous_object.base.is_slider() {
                let lazy_ratio = current_object.min_jump_time.max(current_delta)
                    / current_object.min_jump_time.min(current_delta);
                let real_ratio = current_object.last_object_end_delta_time.max(current_delta)
                    / current_object.last_object_end_delta_time.min(current_delta);
                let slider_difficulty = Self::effective_difficulty(lazy_ratio)
                    .min(Self::effective_difficulty(real_ratio));
                effective_difficulty = effective_difficulty.min(slider_difficulty);
            }

            if delta_difference < delta_difference_epsilon {
                island.add_delta(current_delta as i32);
            }

            if first_delta_switch {
                if delta_difference > delta_difference_epsilon {
                    if current_object.base.is_slider() {
                        effective_difficulty *= 0.5;
                    }

                    if island.is_similar_polarity(&previous_island, delta_difference_epsilon) {
                        effective_difficulty *= 0.5;
                    }

                    if previous_previous_object.delta_time.max(1e-7)
                        > previous_delta + delta_difference_epsilon
                        && previous_delta > current_delta + delta_difference_epsilon
                    {
                        effective_difficulty *= 0.125;
                    }

                    if previous_island.delta_count == island.delta_count {
                        effective_difficulty *= 0.5;
                    }

                    if previous_delta > current_delta + delta_difference_epsilon {
                        effective_difficulty *= 0.65;
                    }

                    let mut found = false;

                    for existing in &mut islands {
                        if existing.almost_equals(&island, delta_difference_epsilon) {
                            if previous_island.almost_equals(&island, delta_difference_epsilon) {
                                existing.occurrences += 1;
                            }

                            let power = logistic(f64::from(island.delta), 58.33, 0.24, Some(2.75));
                            effective_difficulty *= (3.0 / existing.occurrences as f64)
                                .min((1.0 / existing.occurrences as f64).powf(power));
                            found = true;
                            break;
                        }
                    }

                    if !found && island.delta_count > 0 {
                        islands.push(island);
                    }

                    effective_difficulty *= 1.0
                        - previous_object.get_doubletapness(Some(current_object), hit_window)
                            * 0.75;

                    if island.delta_count > 1 {
                        rhythm_complexity_sum += (effective_difficulty * start_difficulty).sqrt()
                            * current_historical_decay;
                    } else {
                        rhythm_complexity_sum += 0.7 * current_historical_decay;
                    }

                    start_difficulty = effective_difficulty;

                    if previous_delta + delta_difference_epsilon < current_delta {
                        first_delta_switch = false;
                    }

                    previous_island = island;
                    island = RhythmIsland::new(current_delta as i32);
                }
            } else if previous_delta > current_delta + delta_difference_epsilon {
                first_delta_switch = true;

                if current_object.base.is_slider() {
                    effective_difficulty *= 0.6;
                }

                if previous_object.base.is_slider() {
                    effective_difficulty *= 0.6;
                }

                start_difficulty = effective_difficulty;
                island = RhythmIsland::new(current_delta as i32);
            }

            previous_previous_object = previous_object;
            previous_object = current_object;
        }

        rhythm_complexity_sum *= reverse_lerp(island.delta_count as f64, 22.0, 3.0);

        (4.0 + rhythm_complexity_sum * 0.95).sqrt() / 2.0
    }

    fn effective_difficulty(delta_difference_ratio: f64) -> f64 {
        let fractional = delta_difference_ratio - delta_difference_ratio.trunc();
        1.0 + 26.0 * smoothstep_bell_curve(fractional).min(0.5)
    }
}

#[derive(Copy, Clone)]
struct RhythmIsland {
    delta: i32,
    delta_count: i32,
    occurrences: usize,
}

impl RhythmIsland {
    fn new(delta: i32) -> Self {
        Self {
            delta: cmp::max(delta, OsuDifficultyObject::MIN_DELTA_TIME as i32),
            delta_count: 1,
            occurrences: 1,
        }
    }

    fn add_delta(&mut self, delta: i32) {
        if self.delta == i32::MAX {
            self.delta = cmp::max(delta, OsuDifficultyObject::MIN_DELTA_TIME as i32);
        }

        self.delta_count += 1;
    }

    fn is_similar_polarity(&self, other: &Self, epsilon: f64) -> bool {
        if self.delta_count <= 1 || other.delta_count <= 1 {
            return false;
        }

        f64::from((self.delta - other.delta).abs()) < epsilon
            && self.delta_count % 2 == other.delta_count % 2
    }

    fn almost_equals(&self, other: &Self, epsilon: f64) -> bool {
        f64::from((self.delta - other.delta).abs()) < epsilon
            && self.delta_count == other.delta_count
    }
}

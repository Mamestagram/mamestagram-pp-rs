use crate::{
    any::difficulty::object::IDifficultyObject,
    model::mods::GameMods,
    osu::difficulty::object::OsuDifficultyObject,
    util::{
        difficulty::{
            logistic, logistic_exp, milliseconds_to_bpm, norm, reverse_lerp, smootherstep,
            smoothstep,
        },
        strains_vec::StrainsVec,
    },
};

/// lazer's variable-length aim strain skill (`20260706`).
#[derive(Clone)]
pub struct Aim {
    include_sliders: bool,
    has_autopilot_mod: bool,
    has_touch_device_mod: bool,
    has_relax_mod: bool,

    current_strain: f64,
    current_section_peak: f64,
    current_section_begin: f64,
    current_section_end: f64,
    strain_peaks: Vec<StrainPeak>,
    total_length: f64,
    queued_strains: Vec<(f64, f64)>,
    final_peak: Option<StrainPeak>,

    object_difficulties: Vec<f64>,
    slider_strains: Vec<f64>,
}

#[derive(Copy, Clone, Debug)]
struct StrainPeak {
    value: f64,
    section_length: f64,
}

impl StrainPeak {
    fn new(value: f64, section_length: f64) -> Self {
        Self {
            value,
            // Match System.Math.Round's default midpoint-to-even mode.
            section_length: section_length.round_ties_even(),
        }
    }
}

impl Aim {
    const DECAY_WEIGHT: f64 = 0.9;
    const MAX_SECTION_LENGTH: f64 = 400.0;
    const MAX_STORED_LENGTH: f64 = 11.0 / (1.0 - Self::DECAY_WEIGHT);
    const REDUCED_SECTION_TIME: f64 = 4000.0;
    const REDUCED_STRAIN_BASELINE: f64 = 0.727;

    pub fn new(mods: &GameMods, include_sliders: bool) -> Self {
        Self {
            include_sliders,
            has_autopilot_mod: mods.ap(),
            has_touch_device_mod: mods.td(),
            has_relax_mod: mods.rx(),
            current_strain: 0.0,
            current_section_peak: 0.0,
            current_section_begin: 0.0,
            current_section_end: 0.0,
            strain_peaks: Vec::with_capacity(256),
            total_length: 0.0,
            queued_strains: Vec::with_capacity(32),
            final_peak: None,
            object_difficulties: Vec::with_capacity(256),
            slider_strains: Vec::with_capacity(64),
        }
    }

    pub fn process(&mut self, curr: &OsuDifficultyObject<'_>, objects: &[OsuDifficultyObject<'_>]) {
        let difficulty = self.process_internal(curr, objects);
        self.object_difficulties.push(difficulty);
    }

    fn process_internal(
        &mut self,
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
    ) -> f64 {
        if curr.idx == 0 {
            self.current_section_begin = curr.start_time;
            self.current_section_end = self.current_section_begin + Self::MAX_SECTION_LENGTH;
            self.current_section_peak = self.strain_value_at(curr, objects);

            return self.current_section_peak;
        }

        self.backfill_peaks(curr, objects);

        let current_strain = self.strain_value_at(curr, objects);

        if current_strain > self.current_section_peak {
            self.queued_strains.clear();
            self.save_current_peak(curr.start_time - self.current_section_begin);
            self.current_section_begin = curr.start_time;
            self.current_section_end = self.current_section_begin + Self::MAX_SECTION_LENGTH;
            self.current_section_peak = current_strain;
        } else {
            while self
                .queued_strains
                .last()
                .is_some_and(|&(strain, _)| strain < current_strain)
            {
                self.queued_strains.pop();
            }

            self.queued_strains.push((current_strain, curr.start_time));
        }

        current_strain
    }

    fn backfill_peaks(
        &mut self,
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
    ) {
        while curr.start_time > self.current_section_end {
            self.save_current_peak(self.current_section_end - self.current_section_begin);
            self.current_section_begin = self.current_section_end;

            if self.queued_strains.is_empty() {
                self.current_section_end = self.current_section_begin + Self::MAX_SECTION_LENGTH;
                self.start_new_section_from(self.current_section_begin, curr, objects);
            } else {
                let (strain, start_time) = self.queued_strains.remove(0);
                self.current_section_end = start_time + Self::MAX_SECTION_LENGTH;
                self.start_new_section_from(self.current_section_begin, curr, objects);
                self.current_section_peak = self.current_section_peak.max(strain);
            }
        }
    }

    fn start_new_section_from(
        &mut self,
        time: f64,
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
    ) {
        let previous_start_time = curr
            .previous(0, objects)
            .map_or(0.0, |previous| previous.start_time);
        self.current_section_peak =
            self.current_strain * Self::strain_decay(time - previous_start_time);
    }

    fn save_current_peak(&mut self, section_length: f64) {
        if let Some(final_peak) = self.final_peak.take() {
            if let Some(index) = self.strain_peaks.iter().position(|peak| {
                peak.value == final_peak.value && peak.section_length == final_peak.section_length
            }) {
                self.strain_peaks.remove(index);
            }
        }

        let peak = StrainPeak::new(self.current_section_peak, section_length);
        self.insert_peak(peak);
        // lazer accumulates the unrounded input length but removes the rounded
        // length stored by `StrainPeak` when pruning.
        self.total_length += section_length;

        while self.total_length > Self::MAX_STORED_LENGTH * Self::MAX_SECTION_LENGTH {
            let Some(removed) = self.strain_peaks.pop() else {
                break;
            };

            self.total_length -= removed.section_length;
        }
    }

    fn insert_peak(&mut self, peak: StrainPeak) {
        let index = self
            .strain_peaks
            .partition_point(|other| other.value > peak.value);
        self.strain_peaks.insert(index, peak);
    }

    fn get_current_strain_peaks(&mut self) -> &[StrainPeak] {
        if self.final_peak.is_none() {
            let peak = StrainPeak::new(
                self.current_section_peak,
                self.current_section_end - self.current_section_begin,
            );
            self.insert_peak(peak);
            self.final_peak = Some(peak);
        }

        &self.strain_peaks
    }

    fn strain_decay(ms: f64) -> f64 {
        0.2_f64.powf(ms / 1000.0)
    }

    fn strain_value_at(
        &mut self,
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
    ) -> f64 {
        if self.has_autopilot_mod {
            return 0.0;
        }

        let decay = Self::strain_decay(curr.strain_time);
        self.current_strain *= decay;
        self.current_strain += self.calculate_adjusted_difficulty(curr, objects) * (1.0 - decay);

        if curr.base.is_slider() {
            self.slider_strains.push(self.current_strain);
        }

        self.current_strain
    }

    fn calculate_adjusted_difficulty(
        &self,
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
    ) -> f64 {
        let snap = SnapAimEvaluator::evaluate_diff_of(curr, objects, self.include_sliders) * 70.9;
        let agility = AgilityEvaluator::evaluate_diff_of(curr, objects) * 2.35;
        let flow = FlowAimEvaluator::evaluate_diff_of(curr, objects, self.include_sliders) * 242.0;

        let mut total = self.calculate_total_value(snap, agility, flow);
        total *= 0.985 + curr.overall_difficulty.max(0.0).powi(2) / 4000.0;

        total
    }

    fn calculate_total_value(&self, mut snap: f64, agility: f64, mut flow: f64) -> f64 {
        let mut combined_snap = norm(1.2, [snap, agility]);
        let probability_snap = Self::calculate_snap_flow_probability(flow / combined_snap);
        let probability_flow = 1.0 - probability_snap;

        if self.has_touch_device_mod {
            snap = snap.powf(0.89);
            combined_snap = norm(1.2, [snap, agility]);
        }

        if self.has_relax_mod {
            combined_snap *= 0.75;
            flow *= 0.6;
        }

        (combined_snap * probability_snap + flow * probability_flow) * 1.12
    }

    fn calculate_snap_flow_probability(ratio: f64) -> f64 {
        if ratio == 0.0 {
            return 0.0;
        }

        if ratio.is_nan() {
            return 1.0;
        }

        // This is the one-argument `DiffUtils.Logistic(exponent)` overload,
        // not the x/midpoint overload used by weighted-count calculations.
        logistic_exp(-7.27 * ratio.ln(), None)
    }

    pub fn difficulty_value(&mut self) -> f64 {
        let mut difficulty = 0.0;
        let mut time = 0.0;

        for strain in self.reduced_strain_peaks() {
            let start_time = time;
            let end_time = time + strain.section_length / Self::MAX_SECTION_LENGTH;
            let weight = Self::DECAY_WEIGHT.powf(start_time) - Self::DECAY_WEIGHT.powf(end_time);

            difficulty += strain.value * weight;
            time = end_time;
        }

        difficulty / (1.0 - Self::DECAY_WEIGHT)
    }

    fn reduced_strain_peaks(&mut self) -> Vec<StrainPeak> {
        let mut strains: Vec<_> = self
            .get_current_strain_peaks()
            .iter()
            .copied()
            .filter(|peak| peak.value > 0.0)
            .collect();

        let mut time = 0.0;
        let mut skip_count = 0;

        while strains.len() > skip_count && time < Self::REDUCED_SECTION_TIME {
            let strain = strains[skip_count];
            let mut added_time = 0.0;

            while added_time < strain.section_length {
                let amount = ((time + added_time) / Self::REDUCED_SECTION_TIME).clamp(0.0, 1.0);
                let scale = (1.0 + 9.0 * amount).log10();
                strains.push(StrainPeak::new(
                    strain.value * lerp(Self::REDUCED_STRAIN_BASELINE, 1.0, scale),
                    20.0_f64.min(strain.section_length - added_time),
                ));
                added_time += 20.0;
            }

            time += strain.section_length;
            skip_count += 1;
        }

        let mut strains = strains.split_off(skip_count);
        strains.sort_by(|a, b| b.value.total_cmp(&a.value));

        strains
    }

    pub fn cloned_difficulty_value(&self) -> f64 {
        let mut cloned = self.clone();
        cloned.difficulty_value()
    }

    pub fn count_top_weighted_strains(&self, difficulty_value: f64) -> f64 {
        if self.object_difficulties.is_empty() {
            return 0.0;
        }

        let consistent_top_strain = difficulty_value * (1.0 - Self::DECAY_WEIGHT);

        if consistent_top_strain == 0.0 {
            return self.object_difficulties.len() as f64;
        }

        self.object_difficulties
            .iter()
            .map(|strain| logistic(strain / consistent_top_strain, 0.88, 10.0, Some(1.1)))
            .sum()
    }

    pub fn get_difficult_sliders(&self) -> f64 {
        let Some(max_strain) = self.slider_strains.iter().copied().reduce(f64::max) else {
            return 0.0;
        };

        if max_strain == 0.0 {
            return 0.0;
        }

        self.slider_strains
            .iter()
            .map(|strain| 1.0 / (1.0 + (-(strain / max_strain * 12.0 - 6.0)).exp()))
            .sum()
    }

    pub fn count_top_weighted_sliders(&self, difficulty_value: f64) -> f64 {
        if self.slider_strains.is_empty() {
            return 0.0;
        }

        let consistent_top_strain = difficulty_value * (1.0 - Self::DECAY_WEIGHT);

        if consistent_top_strain == 0.0 {
            return 0.0;
        }

        self.slider_strains
            .iter()
            .map(|strain| logistic(strain / consistent_top_strain, 0.88, 10.0, Some(1.1)))
            .sum()
    }

    pub fn into_current_strain_peaks(mut self) -> StrainsVec {
        let peaks: Vec<_> = self
            .get_current_strain_peaks()
            .iter()
            .map(|peak| peak.value)
            .collect();
        let mut result = StrainsVec::with_capacity(peaks.len());

        for peak in peaks {
            result.push(peak);
        }

        result
    }

    pub fn difficulty_to_performance(difficulty: f64) -> f64 {
        4.0 * difficulty.powi(3)
    }
}

struct SnapAimEvaluator;

impl SnapAimEvaluator {
    fn evaluate_diff_of(
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
        with_slider_travel_dist: bool,
    ) -> f64 {
        let Some(last) = curr.previous(0, objects) else {
            return 0.0;
        };

        if curr.base.is_spinner() || curr.idx <= 1 || last.base.is_spinner() {
            return 0.0;
        }

        let last2 = curr.previous(2, objects);
        let radius = f64::from(OsuDifficultyObject::NORMALIZED_RADIUS);
        let diameter = f64::from(OsuDifficultyObject::NORMALIZED_DIAMETER);

        let curr_distance = if with_slider_travel_dist {
            curr.lazy_jump_dist
        } else {
            curr.jump_dist
        };
        let mut curr_velocity = curr_distance / curr.strain_time;

        if last.base.is_slider() && with_slider_travel_dist {
            let slider_distance = last.lazy_travel_dist + curr.lazy_jump_dist;
            curr_velocity = curr_velocity.max(slider_distance / curr.strain_time);
        }

        let prev_distance = if with_slider_travel_dist {
            last.lazy_jump_dist
        } else {
            last.jump_dist
        };
        let prev_velocity = prev_distance / last.strain_time;
        let mut difficulty = curr_velocity * Self::vector_angle_repetition(curr, last, objects);

        if let (Some(curr_angle), Some(last_angle)) = (curr.angle, last.angle) {
            let velocity_influence = curr_velocity.min(prev_velocity);
            let mut acute_bonus = 0.0;

            if curr.strain_time.max(last.strain_time)
                < 1.25 * curr.strain_time.min(last.strain_time)
            {
                acute_bonus = Self::angle_acuteness(curr_angle);
                acute_bonus *= 0.08
                    + 0.92 * (1.0 - acute_bonus.min(Self::angle_acuteness(last_angle).powi(3)));
                acute_bonus *= velocity_influence
                    * smootherstep(milliseconds_to_bpm(curr.strain_time, Some(2)), 300.0, 400.0)
                    * smootherstep(curr_distance, 0.0, diameter * 2.0);
            }

            let mut wide_bonus = Self::angle_wideness(curr_angle);
            wide_bonus *=
                0.25 + 0.75 * (1.0 - wide_bonus.min(Self::angle_wideness(last_angle).powi(3)));

            let mut wide_curr_velocity = curr_distance / curr.strain_time.powf(1.45);
            let wide_prev_velocity = prev_distance / last.strain_time.powf(1.45);

            if last.base.is_slider() && with_slider_travel_dist {
                let slider_distance = last.lazy_travel_dist + curr.lazy_jump_dist;
                wide_curr_velocity =
                    wide_curr_velocity.max(slider_distance / curr.strain_time.powf(1.45));
            }

            wide_bonus *= wide_curr_velocity.min(wide_prev_velocity);

            if let Some(last2) = last2 {
                let distance =
                    f64::from((last2.base.stacked_pos() - last.base.stacked_pos()).length());

                if distance < 1.0 {
                    wide_bonus *= 1.0 - 0.55 * (1.0 - distance);
                }
            }

            difficulty += (acute_bonus * 2.41).max(wide_bonus * 9.67);

            let wiggle_bonus = velocity_influence
                * smootherstep(curr_distance, radius, diameter)
                * reverse_lerp(curr_distance, diameter * 3.0, diameter).powf(1.8)
                * smootherstep(curr_angle, 110.0_f64.to_radians(), 60.0_f64.to_radians())
                * smootherstep(prev_distance, radius, diameter)
                * reverse_lerp(prev_distance, diameter * 3.0, diameter).powf(1.8)
                * smootherstep(last_angle, 110.0_f64.to_radians(), 60.0_f64.to_radians());

            difficulty += wiggle_bonus * 1.02;
        }

        if prev_velocity.max(curr_velocity) != 0.0 {
            if with_slider_travel_dist {
                curr_velocity = curr_distance / curr.strain_time;
            }

            let distance_ratio = smoothstep(
                (prev_velocity - curr_velocity).abs() / prev_velocity.max(curr_velocity),
                0.0,
                1.0,
            );
            let overlap_velocity_buff = (diameter * 1.25 / curr.strain_time.min(last.strain_time))
                .min((prev_velocity - curr_velocity).abs());
            let mut velocity_change_bonus = overlap_velocity_buff * distance_ratio;
            velocity_change_bonus *= (curr.strain_time.min(last.strain_time)
                / curr.strain_time.max(last.strain_time))
            .powi(2);
            difficulty += velocity_change_bonus * 0.9;
        }

        if curr.base.is_slider() && with_slider_travel_dist {
            let slider_bonus = curr.travel_dist / curr.travel_time;
            difficulty += if slider_bonus < 1.0 {
                slider_bonus
            } else {
                slider_bonus.powf(0.75)
            } * 1.5;
        }

        difficulty *= curr.small_circle_bonus;
        difficulty *= Self::high_bpm_bonus(curr.strain_time);

        difficulty
    }

    fn high_bpm_bonus(ms: f64) -> f64 {
        1.0 / (1.0 - 0.03_f64.powf((ms / 1000.0).powf(0.65)))
    }

    fn vector_angle_repetition(
        current: &OsuDifficultyObject<'_>,
        previous: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
    ) -> f64 {
        let (Some(curr_angle), Some(last_angle)) = (current.angle, previous.angle) else {
            return 1.0;
        };

        let mut constant_angle_count = 0.0;

        for index in 0..6 {
            let Some(prev) = current.previous(index, objects) else {
                break;
            };

            if current.strain_time.max(prev.strain_time)
                > 1.1 * current.strain_time.min(prev.strain_time)
            {
                break;
            }

            if let (Some(previous_vector), Some(current_vector)) = (
                prev.normalised_vector_angle,
                current.normalised_vector_angle,
            ) {
                let angle_difference = (current_vector - previous_vector).abs();
                constant_angle_count += (8.0 * 11.25_f64.to_radians().min(angle_difference)).cos();
            }
        }

        let vector_repetition = (0.5 / constant_angle_count).min(1.0).powi(2);
        let stack_factor = smootherstep(
            current.lazy_jump_dist,
            0.0,
            f64::from(OsuDifficultyObject::NORMALIZED_DIAMETER),
        );
        let angle_difference_adjusted = (2.0
            * 45.0_f64
                .to_radians()
                .min((curr_angle - last_angle).abs() * stack_factor))
        .cos();
        let base_nerf = 1.0 - 0.15 * Self::angle_acuteness(last_angle) * angle_difference_adjusted;

        (base_nerf + (1.0 - base_nerf) * vector_repetition * 0.5 * stack_factor).powi(2)
    }

    fn angle_wideness(angle: f64) -> f64 {
        smoothstep(angle, 40.0_f64.to_radians(), 140.0_f64.to_radians())
    }

    fn angle_acuteness(angle: f64) -> f64 {
        smoothstep(angle, 140.0_f64.to_radians(), 40.0_f64.to_radians())
    }
}

struct AgilityEvaluator;

impl AgilityEvaluator {
    fn evaluate_diff_of(
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
    ) -> f64 {
        if curr.base.is_spinner() {
            return 0.0;
        }

        let distance_cap = f64::from(OsuDifficultyObject::NORMALIZED_DIAMETER) * 1.2;
        let travel_distance = curr
            .previous(0, objects)
            .map_or(0.0, |previous| previous.lazy_travel_dist);
        let distance_scaled =
            (travel_distance + curr.lazy_jump_dist).min(distance_cap) / distance_cap;

        distance_scaled * 1000.0 / curr.strain_time
            * curr.small_circle_bonus.powf(1.5)
            * (1.0 / (1.0 - 0.2_f64.powf(curr.strain_time / 1000.0)))
    }
}

struct FlowAimEvaluator;

impl FlowAimEvaluator {
    fn evaluate_diff_of(
        curr: &OsuDifficultyObject<'_>,
        objects: &[OsuDifficultyObject<'_>],
        with_slider_travel_dist: bool,
    ) -> f64 {
        let Some(last) = curr.previous(0, objects) else {
            return 0.0;
        };

        if curr.base.is_spinner() || curr.idx <= 1 || last.base.is_spinner() {
            return 0.0;
        }

        let Some(last_last) = curr.previous(1, objects) else {
            return 0.0;
        };

        let curr_distance = if with_slider_travel_dist {
            curr.lazy_jump_dist
        } else {
            curr.jump_dist
        };
        let prev_distance = if with_slider_travel_dist {
            last.lazy_jump_dist
        } else {
            last.jump_dist
        };
        let mut curr_velocity = curr_distance / curr.strain_time;

        if last.base.is_slider() && with_slider_travel_dist {
            let slider_distance = last.lazy_travel_dist + curr.lazy_jump_dist;
            curr_velocity = curr_velocity.max(slider_distance / curr.strain_time);
        }

        let prev_velocity = prev_distance / last.strain_time;
        let mut difficulty = curr_velocity * curr.small_circle_bonus.sqrt();
        difficulty *= 1.0
            + 0.25_f64.min(
                ((curr.strain_time.max(last.strain_time) - curr.strain_time.min(last.strain_time))
                    / 50.0)
                    .powi(4),
            );

        if let (Some(curr_angle), Some(last_angle)) = (curr.angle, last.angle) {
            let angle_difference = (curr_angle - last_angle).abs();
            let angle_difference_adjusted = (angle_difference / 2.0).sin() * 180.0;
            let angular_velocity = angle_difference_adjusted / (curr.strain_time * 0.1);
            difficulty *= 0.8 + (angular_velocity / 270.0).sqrt();
        }

        let mut overlapped_notes_weight = 1.0;

        if curr.idx > 2 {
            overlapped_notes_weight = 1.0
                - Self::overlap_factor(curr, last)
                    * Self::overlap_factor(curr, last_last)
                    * Self::overlap_factor(last, last_last);
        }

        if let Some(angle) = curr.angle {
            difficulty +=
                curr_velocity * SnapAimEvaluator::angle_acuteness(angle) * overlapped_notes_weight;
        }

        if prev_velocity.max(curr_velocity) != 0.0 {
            if with_slider_travel_dist {
                curr_velocity = curr_distance / curr.strain_time;
            }

            let distance_ratio = smoothstep(
                (prev_velocity - curr_velocity).abs() / prev_velocity.max(curr_velocity),
                0.0,
                1.0,
            );
            let overlap_velocity_buff = (f64::from(OsuDifficultyObject::NORMALIZED_DIAMETER)
                * 1.25
                / curr.strain_time.min(last.strain_time))
            .min((prev_velocity - curr_velocity).abs());

            difficulty += overlap_velocity_buff * distance_ratio * overlapped_notes_weight * 0.52;
        }

        if curr.base.is_slider() && with_slider_travel_dist {
            difficulty += curr.travel_dist / curr.travel_time;
        }

        difficulty.powf(1.45)
            * smootherstep(
                curr_distance,
                0.0,
                f64::from(OsuDifficultyObject::NORMALIZED_RADIUS),
            )
    }

    fn overlap_factor(first: &OsuDifficultyObject<'_>, second: &OsuDifficultyObject<'_>) -> f64 {
        let distance = f64::from((first.base.stacked_pos() - second.base.stacked_pos()).length());

        (1.0 - ((distance - first.radius).max(0.0) / first.radius).powi(2)).clamp(0.0, 1.0)
    }
}

fn lerp(start: f64, end: f64, amount: f64) -> f64 {
    start + (end - start) * amount
}

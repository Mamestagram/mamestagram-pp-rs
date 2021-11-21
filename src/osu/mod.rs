#![cfg(feature = "osu")]

mod difficulty_iter;
mod difficulty_object;
mod osu_object;
mod pp;
mod scaling_factor;
mod skill;
mod skill_kind;
mod slider_state;

use std::mem;

pub use difficulty_iter::OsuDifficultyAttributesIter;
use difficulty_object::DifficultyObject;
use osu_object::{ObjectParameters, OsuObject};
pub use pp::*;
use scaling_factor::ScalingFactor;
use skill::Skill;
use skill_kind::SkillKind;
use slider_state::SliderState;

use crate::{curve::CurveBuffers, Beatmap, Mods, Strains};

use self::skill::Skills;

const SECTION_LEN: f64 = 400.0;
const DIFFICULTY_MULTIPLIER: f64 = 0.0675;
const NORMALIZED_RADIUS: f32 = 50.0; // * diameter of 100; easier mental maths.
const STACK_DISTANCE: f32 = 3.0;

/// Difficulty calculation for osu!standard maps.
///
/// In case of a partial play, e.g. a fail, one can specify the amount of passed objects.
pub fn stars(
    map: &Beatmap,
    mods: impl Mods,
    passed_objects: Option<usize>,
) -> OsuDifficultyAttributes {
    let (mut skills, mut attributes) = match calculate_skills(map, mods, passed_objects) {
        Some(tuple) => tuple,
        None => {
            let map_attributes = map.attributes().mods(mods);
            let hit_window = difficulty_range_od(map_attributes.od) / map_attributes.clock_rate;
            let od = (80.0 - hit_window) / 6.0;

            return OsuDifficultyAttributes {
                ar: map_attributes.ar,
                hp: map_attributes.hp,
                od,
                ..Default::default()
            };
        }
    };

    let aim_rating = {
        let aim = skills.aim();
        let mut aim_strains = mem::take(&mut aim.strain_peaks);

        Skill::difficulty_value(&mut aim_strains, aim).sqrt() * DIFFICULTY_MULTIPLIER
    };

    let slider_factor = if aim_rating > 0.0 {
        let aim_no_sliders = skills.aim_no_sliders();

        let mut aim_strains_no_sliders = mem::take(&mut aim_no_sliders.strain_peaks);
        let aim_rating_no_sliders =
            Skill::difficulty_value(&mut aim_strains_no_sliders, aim_no_sliders).sqrt()
                * DIFFICULTY_MULTIPLIER;

        aim_rating_no_sliders / aim_rating
    } else {
        1.0
    };

    let (speed, flashlight) = skills.speed_flashlight();

    let speed_rating = if let Some(speed) = speed {
        let mut speed_strains = mem::take(&mut speed.strain_peaks);

        Skill::difficulty_value(&mut speed_strains, speed).sqrt() * DIFFICULTY_MULTIPLIER
    } else {
        0.0
    };

    let flashlight_rating = if let Some(flashlight) = flashlight {
        let mut flashlight_strains = mem::take(&mut flashlight.strain_peaks);

        Skill::difficulty_value(&mut flashlight_strains, flashlight).sqrt() * DIFFICULTY_MULTIPLIER
    } else {
        0.0
    };

    let star_rating = calculate_star_rating(aim_rating, speed_rating, flashlight_rating);

    attributes.aim_strain = aim_rating;
    attributes.speed_strain = speed_rating;
    attributes.flashlight_rating = flashlight_rating;
    attributes.slider_factor = slider_factor;
    attributes.stars = star_rating;

    attributes
}

fn calculate_star_rating(aim_rating: f64, speed_rating: f64, flashlight_rating: f64) -> f64 {
    let base_aim_performance = {
        let base = 5.0 * (aim_rating / 0.0675).max(1.0) - 4.0;

        base * base * base / 100_000.0
    };

    let base_speed_performance = {
        let base = 5.0 * (speed_rating / 0.0675).max(1.0) - 4.0;

        base * base * base / 100_000.0
    };

    let base_flashlight_performance = flashlight_rating * flashlight_rating * 25.0;

    let base_performance = (base_aim_performance.powf(1.1)
        + base_speed_performance.powf(1.1)
        + base_flashlight_performance.powf(1.1))
    .powf(1.0 / 1.1);

    if base_performance > 0.00001 {
        1.12_f64.cbrt()
            * 0.027
            * ((100_000.0 / (1.0_f64 / 1.1).exp2() * base_performance).cbrt() + 4.0)
    } else {
        0.0
    }
}

/// Essentially the same as the [`stars`] function but instead of
/// evaluating the final strains, it just returns them as is.
///
/// Suitable to plot the difficulty of a map over time.
pub fn strains(map: &Beatmap, mods: impl Mods) -> Strains {
    let mut skills = match calculate_skills(map, mods, None) {
        Some((skills, _)) => skills,
        None => return Strains::default(),
    };

    let mut aim = mem::take(&mut skills.aim().strain_peaks);
    let tuple = skills.speed_flashlight();

    let strains = match tuple {
        (Some(speed), Some(flashlight)) => {
            for ((aim, speed), flashlight) in aim
                .iter_mut()
                .zip(&speed.strain_peaks)
                .zip(&flashlight.strain_peaks)
            {
                *aim += speed + flashlight;
            }

            aim
        }
        (Some(strains), None) | (None, Some(strains)) => {
            for (aim, strain) in aim.iter_mut().zip(&strains.strain_peaks) {
                *aim += strain;
            }

            aim
        }
        (None, None) => aim,
    };

    Strains {
        section_length: SECTION_LEN * mods.speed(),
        strains,
    }
}

fn calculate_skills(
    map: &Beatmap,
    mods: impl Mods,
    passed_objects: Option<usize>,
) -> Option<(Skills, OsuDifficultyAttributes)> {
    let take = passed_objects.unwrap_or_else(|| map.hit_objects.len());

    let map_attributes = map.attributes().mods(mods);
    let hit_window = difficulty_range_od(map_attributes.od) / map_attributes.clock_rate;
    let od = (80.0 - hit_window) / 6.0;

    if take < 2 {
        return None;
    }

    let mut raw_ar = map.ar as f64;
    let hr = mods.hr();

    if hr {
        raw_ar = (raw_ar * 1.4).min(10.0);
    } else if mods.ez() {
        raw_ar *= 0.5;
    }

    let time_preempt = difficulty_range_ar(raw_ar);
    let scaling_factor = ScalingFactor::new(map_attributes.cs);

    let mut params = ObjectParameters {
        map,
        max_combo: 0,
        slider_state: SliderState::new(map),
        ticks: Vec::new(),
        curve_bufs: CurveBuffers::default(),
    };

    let hit_objects_iter = map
        .hit_objects
        .iter()
        .take(take)
        .filter_map(|h| OsuObject::new(h, hr, &mut params));

    let mut hit_objects = Vec::with_capacity(take);
    hit_objects.extend(hit_objects_iter);

    let stack_threshold = time_preempt * map.stack_leniency as f64;

    if map.version >= 6 {
        stacking(&mut hit_objects, stack_threshold);
    } else {
        old_stacking(&mut hit_objects, stack_threshold);
    }

    let mut hit_objects = hit_objects.into_iter().map(|mut h| {
        let stack_offset = scaling_factor.stack_offset(h.stack_height);
        h.pos += stack_offset;

        h
    });

    let mut skills = Skills::new(hit_window, mods.rx(), scaling_factor.radius(), mods.fl());

    let mut prev_prev = None;
    let mut prev = hit_objects.next().unwrap();

    // First object has no predecessor and thus no strain, handle distinctly
    let mut curr_section_end =
        (prev.time / map_attributes.clock_rate / SECTION_LEN).ceil() * SECTION_LEN;

    // Handle second object separately to remove later if-branching
    let curr = hit_objects.next().unwrap();
    let h = DifficultyObject::new(
        &curr,
        &mut prev,
        prev_prev.as_ref(),
        &scaling_factor,
        map_attributes.clock_rate,
    );

    let base_time = h.base.time / map_attributes.clock_rate;

    while base_time > curr_section_end {
        skills.start_new_section_from(curr_section_end);
        curr_section_end += SECTION_LEN;
    }

    skills.process(&h);
    prev_prev = Some(mem::replace(&mut prev, curr));

    // Handle all other objects
    for curr in hit_objects {
        let h = DifficultyObject::new(
            &curr,
            &mut prev,
            prev_prev.as_ref(),
            &scaling_factor,
            map_attributes.clock_rate,
        );

        let base_time = h.base.time / map_attributes.clock_rate;

        while base_time > curr_section_end {
            skills.save_peak_and_start_new_section(curr_section_end);
            curr_section_end += SECTION_LEN;
        }

        skills.process(&h);
        prev_prev = Some(mem::replace(&mut prev, curr));
    }

    skills.save_current_peak();

    let attributes = OsuDifficultyAttributes {
        ar: map_attributes.ar,
        hp: map_attributes.hp,
        od,
        n_circles: map.n_circles as usize,
        n_sliders: map.n_sliders as usize,
        n_spinners: map.n_spinners as usize,
        max_combo: params.max_combo,
        ..Default::default()
    };

    Some((skills, attributes))
}

fn stacking(hit_objects: &mut [OsuObject], stack_threshold: f64) {
    let mut extended_start_idx = 0;
    let extended_end_idx = hit_objects.len() - 1;

    // First big `if` in osu!lazer's function can be skipped

    for i in (1..=extended_end_idx).rev() {
        let mut n = i;
        let mut obj_i_idx = i;
        // * We should check every note which has not yet got a stack.
        // * Consider the case we have two interwound stacks and this will make sense.
        // *   o <-1      o <-2
        // *    o <-3      o <-4
        // * We first process starting from 4 and handle 2,
        // * then we come backwards on the i loop iteration until we reach 3 and handle 1.
        // * 2 and 1 will be ignored in the i loop because they already have a stack value.

        if hit_objects[obj_i_idx].stack_height.abs() > 0.0 || hit_objects[obj_i_idx].is_spinner() {
            continue;
        }

        // * If this object is a hitcircle, then we enter this "special" case.
        // * It either ends with a stack of hitcircles only,
        // * or a stack of hitcircles that are underneath a slider.
        // * Any other case is handled by the "is_slider" code below this.
        if hit_objects[obj_i_idx].is_circle() {
            loop {
                n = match n.checked_sub(1) {
                    Some(n) => n,
                    None => break,
                };

                if hit_objects[n].is_spinner() {
                    continue;
                } else if hit_objects[obj_i_idx].time - hit_objects[n].end_time() > stack_threshold
                {
                    break; // * We are no longer within stacking range of the previous object.
                }

                // * HitObjects before the specified update range haven't been reset yet
                if n < extended_start_idx {
                    hit_objects[n].stack_height = 0.0;
                    extended_start_idx = n;
                }

                // * This is a special case where hticircles are moved DOWN and RIGHT (negative stacking)
                // * if they are under the *last* slider in a stacked pattern.
                // *    o==o <- slider is at original location
                // *        o <- hitCircle has stack of -1
                // *         o <- hitCircle has stack of -2
                if hit_objects[n].is_slider()
                    && hit_objects[n]
                        .end_pos()
                        .distance(hit_objects[obj_i_idx].pos)
                        < STACK_DISTANCE
                {
                    let offset =
                        hit_objects[obj_i_idx].stack_height - hit_objects[n].stack_height + 1.0;

                    for j in n + 1..=i {
                        // * For each object which was declared under this slider, we will offset
                        // * it to appear *below* the slider end (rather than above).
                        if hit_objects[n].end_pos().distance(hit_objects[j].pos) < STACK_DISTANCE {
                            hit_objects[j].stack_height -= offset;
                        }
                    }

                    // * We have hit a slider. We should restart calculation using this as the new base.
                    // * Breaking here will mean that the slider still has StackCount of 0,
                    // * so will be handled in the i-outer-loop.
                    break;
                }

                if hit_objects[n].pos.distance(hit_objects[obj_i_idx].pos) < STACK_DISTANCE {
                    // * Keep processing as if there are no sliders.
                    // * If we come across a slider, this gets cancelled out.
                    // * NOTE: Sliders with start positions stacking
                    // * are a special case that is also handled here.

                    hit_objects[n].stack_height = hit_objects[obj_i_idx].stack_height + 1.0;
                    obj_i_idx = n;
                }
            }
        } else if hit_objects[obj_i_idx].is_slider() {
            // * We have hit the first slider in a possible stack.
            // * From this point on, we ALWAYS stack positive regardless.
            loop {
                n = match n.checked_sub(1) {
                    Some(n) => n,
                    None => break,
                };

                if hit_objects[n].is_spinner() {
                    continue;
                } else if hit_objects[obj_i_idx].time - hit_objects[n].time > stack_threshold {
                    break; // * We are no longer within stacking range of the previous object.
                }

                if hit_objects[n]
                    .end_pos()
                    .distance(hit_objects[obj_i_idx].pos)
                    < STACK_DISTANCE
                {
                    hit_objects[n].stack_height = hit_objects[obj_i_idx].stack_height + 1.0;
                    obj_i_idx = n;
                }
            }
        }
    }
}

fn old_stacking(hit_objects: &mut [OsuObject], stack_threshold: f64) {
    for i in 0..hit_objects.len() {
        if hit_objects[i].stack_height != 0.0 && !hit_objects[i].is_slider() {
            continue;
        }

        let mut start_time = hit_objects[i].end_time();
        let end_pos = hit_objects[i].end_pos();

        let mut slider_stack = 0.0;

        for j in i + 1..hit_objects.len() {
            if hit_objects[j].time - stack_threshold > start_time {
                break;
            }

            if hit_objects[j].pos.distance(hit_objects[i].pos) < STACK_DISTANCE {
                hit_objects[i].stack_height += 1.0;
                start_time = hit_objects[j].end_time();
            } else if hit_objects[j].pos.distance(end_pos) < STACK_DISTANCE {
                slider_stack += 1.0;
                hit_objects[j].stack_height -= slider_stack;
                start_time = hit_objects[j].end_time();
            }
        }
    }
}

#[inline]
fn difficulty_range_ar(ar: f64) -> f64 {
    crate::difficulty_range(ar, 450.0, 1200.0, 1800.0)
}

fn lerp(start: f64, end: f64, percent: f64) -> f64 {
    start + (end - start) * percent
}

/// The result of a difficulty calculation on an osu!standard map.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OsuDifficultyAttributes {
    /// The aim portion of the total strain.
    pub aim_strain: f64,
    /// The speed portion of the total strain.
    pub speed_strain: f64,
    /// The flashlight portion of the total strain.
    pub flashlight_rating: f64,
    /// The ratio of the aim strain with and without considering sliders
    pub slider_factor: f64,
    /// The approach rate.
    pub ar: f64,
    /// The overall difficulty
    pub od: f64,
    /// The health drain rate.
    pub hp: f64,
    /// The amount of circles.
    pub n_circles: usize,
    /// The amount of sliders.
    pub n_sliders: usize,
    /// The amount of spinners.
    pub n_spinners: usize,
    /// The final star rating
    pub stars: f64,
    /// The maximum combo.
    pub max_combo: usize,
}

/// The result of a performance calculation on an osu!standard map.
#[derive(Clone, Debug, Default)]
pub struct OsuPerformanceAttributes {
    /// The difficulty attributes that were used for the performance calculation
    pub difficulty: OsuDifficultyAttributes,
    /// The final performance points.
    pub pp: f64,
    /// The accuracy portion of the final pp.
    pub pp_acc: f64,
    /// The aim portion of the final pp.
    pub pp_aim: f64,
    /// The flashlight portion of the final pp.
    pub pp_flashlight: f64,
    /// The speed portion of the final pp.
    pub pp_speed: f64,
}

impl OsuPerformanceAttributes {
    /// Return the star value.
    #[inline]
    pub fn stars(&self) -> f64 {
        self.difficulty.stars
    }

    /// Return the performance point value.
    #[inline]
    pub fn pp(&self) -> f64 {
        self.pp
    }

    /// Return the maximum combo of the map.
    #[inline]
    pub fn max_combo(&self) -> usize {
        self.difficulty.max_combo
    }
}

impl From<OsuPerformanceAttributes> for OsuDifficultyAttributes {
    fn from(attributes: OsuPerformanceAttributes) -> Self {
        attributes.difficulty
    }
}

#[inline]
fn difficulty_range_od(od: f64) -> f64 {
    super::difficulty_range(od, 20.0, 50.0, 80.0)
}

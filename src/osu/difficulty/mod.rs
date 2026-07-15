use std::{cmp, pin::Pin};

use rosu_map::section::general::GameMode;
use skills::{aim::Aim, flashlight::Flashlight, speed::Speed};

use crate::{
    any::difficulty::Difficulty,
    model::{beatmap::BeatmapAttributes, mode::ConvertError, mods::GameMods},
    osu::{
        convert::convert_objects,
        difficulty::{object::OsuDifficultyObject, scaling_factor::ScalingFactor},
        object::OsuObject,
        performance::PERFORMANCE_BASE_MULTIPLIER,
    },
    Beatmap,
};

use self::skills::OsuSkills;

use super::attributes::OsuDifficultyAttributes;

pub mod gradual;
pub mod legacy_score;
mod object;
pub mod scaling_factor;
pub mod skills;

const DIFFICULTY_MULTIPLIER: f64 = 0.0675;

const HD_FADE_IN_DURATION_MULTIPLIER: f64 = 0.4;
const HD_FADE_OUT_DURATION_MULTIPLIER: f64 = 0.3;

pub fn difficulty(
    difficulty: &Difficulty,
    map: &Beatmap,
) -> Result<OsuDifficultyAttributes, ConvertError> {
    let map = map.convert_ref(GameMode::Osu, difficulty.get_mods())?;

    let DifficultyValues { skills, mut attrs } = DifficultyValues::calculate(difficulty, &map);

    let mods = difficulty.get_mods();

    DifficultyValues::eval(&mut attrs, mods, &skills);

    Ok(attrs)
}

pub struct OsuDifficultySetup {
    scaling_factor: ScalingFactor,
    map_attrs: BeatmapAttributes,
    attrs: OsuDifficultyAttributes,
    time_preempt: f64,
}

impl OsuDifficultySetup {
    pub fn new(difficulty: &Difficulty, map: &Beatmap) -> Self {
        let clock_rate = difficulty.get_clock_rate();
        let map_attrs = map.attributes().difficulty(difficulty).build();
        let scaling_factor = ScalingFactor::new(map_attrs.cs);
        let od_clock_rate = if difficulty.get_od().is_some_and(|od| od.with_mods) {
            1.0
        } else {
            clock_rate
        };
        let great_hit_window =
            ((map_attrs.hit_windows.od_great * od_clock_rate).floor() - 0.5) / od_clock_rate;
        let ok_hit_window = ((map_attrs.hit_windows.od_ok.unwrap_or(0.0) * od_clock_rate).floor()
            - 0.5)
            / od_clock_rate;
        let meh_hit_window =
            ((map_attrs.hit_windows.od_meh.unwrap_or(0.0) * od_clock_rate).floor() - 0.5)
                / od_clock_rate;

        let attrs = OsuDifficultyAttributes {
            ar: map_attrs.ar,
            hp: map_attrs.hp,
            great_hit_window,
            ok_hit_window,
            meh_hit_window,
            ..Default::default()
        };

        // `OsuHitObject.ApplyDefaultsToSelf` uses DifficultyRangeInt, i.e. a
        // truncating cast after applying AR but before clock-rate adjustment.
        let time_preempt = (map_attrs.hit_windows.ar * clock_rate).trunc();

        Self {
            scaling_factor,
            map_attrs,
            attrs,
            time_preempt,
        }
    }
}

pub struct DifficultyValues {
    pub skills: OsuSkills,
    pub attrs: OsuDifficultyAttributes,
}

impl DifficultyValues {
    pub fn calculate(difficulty: &Difficulty, map: &Beatmap) -> Self {
        let mods = difficulty.get_mods();
        let take = difficulty.get_passed_objects();

        let OsuDifficultySetup {
            scaling_factor,
            map_attrs,
            mut attrs,
            time_preempt,
        } = OsuDifficultySetup::new(difficulty, map);

        let mut osu_objects = convert_objects(
            map,
            &scaling_factor,
            mods.reflection(),
            time_preempt,
            take,
            &mut attrs,
        );

        // upstream: OsuLegacyScoreSimulator を回して MaximumLegacyComboScore を得る。
        // 同時に LegacyScoreBaseMultiplier (peppy_stars) と NestedScorePerObject も計算。
        // これらは convert 後の osu_objects と base map から計算するので、difficulty
        // skills を回す前後どちらでも良いが、attrs の他フィールドと近い位置に置く。
        let n_objects = attrs.n_circles + attrs.n_sliders + attrs.n_spinners;
        let progressive_objects = &osu_objects[..cmp::min(take, osu_objects.len())];
        let peppy_stars = legacy_score::utils::calculate_difficulty_peppy_stars(map);
        attrs.legacy_score_base_multiplier = peppy_stars;
        attrs.nested_score_per_object =
            legacy_score::utils::calculate_nested_score_per_object(progressive_objects, n_objects);
        let legacy_attrs = legacy_score::simulator::simulate(progressive_objects, peppy_stars);
        attrs.maximum_legacy_combo_score = legacy_attrs.combo_score;
        // upstream の LegacyScoreAttributes.MaxCombo と mames の max_combo が一致するはず。
        let _ = legacy_attrs.max_combo;

        let osu_object_iter = osu_objects.iter_mut().map(Pin::new);

        let diff_objects = Self::create_difficulty_objects(
            difficulty,
            &scaling_factor,
            osu_object_iter,
            time_preempt,
            (79.5 - attrs.great_hit_window) / 6.0,
        );

        let mut skills = OsuSkills::new(
            mods,
            &scaling_factor,
            &map_attrs,
            time_preempt,
            map.hit_objects.len(),
            attrs.great_hit_window,
        );

        // The first hit object has no difficulty object
        let take_diff_objects = cmp::min(map.hit_objects.len(), take).saturating_sub(1);

        for hit_object in diff_objects.iter().take(take_diff_objects) {
            skills.process(hit_object, &diff_objects);
        }

        Self { skills, attrs }
    }

    /// Process the difficulty values and store the results in `attrs`.
    pub fn eval(attrs: &mut OsuDifficultyAttributes, _mods: &GameMods, skills: &OsuSkills) {
        let OsuSkills {
            aim,
            aim_no_sliders,
            speed,
            flashlight,
            reading,
        } = skills;

        let aim_difficulty_value = aim.cloned_difficulty_value();
        let aim_no_sliders_difficulty_value = aim_no_sliders.cloned_difficulty_value();
        let speed_difficulty_value = speed.cloned_difficulty_value();
        let (reading_difficulty_value, reading_difficult_note_count) = {
            let mut cloned = reading.clone_for_eval();
            let value = cloned.difficulty_value();
            let count = cloned.count_top_weighted_object_difficulties(value);

            (value, count)
        };

        let aim_rating = aim_difficulty_value.powf(0.63) * 0.02275;
        let aim_no_sliders_rating = aim_no_sliders_difficulty_value.powf(0.63) * 0.02275;
        let speed_rating = speed_difficulty_value.sqrt() * DIFFICULTY_MULTIPLIER;
        let reading_rating = reading_difficulty_value.sqrt() * DIFFICULTY_MULTIPLIER;
        let flashlight_rating = flashlight.lazer_difficulty_value().sqrt() * DIFFICULTY_MULTIPLIER;

        let aim_difficult_strain_count = aim.count_top_weighted_strains(aim_difficulty_value);
        let difficult_sliders = aim.get_difficult_sliders();
        let slider_factor = if aim_rating > 0.0 {
            aim_no_sliders_rating / aim_rating
        } else {
            1.0
        };
        let speed_difficult_strain_count = speed.count_top_weighted_strains(speed_difficulty_value);
        let base_aim_performance = Aim::difficulty_to_performance(aim_rating);
        let base_speed_performance = Speed::difficulty_to_performance(speed_rating);
        let base_reading_performance = Speed::difficulty_to_performance(reading_rating);
        let base_flashlight_performance = Flashlight::difficulty_to_performance(flashlight_rating);
        let base_cognition_performance = if base_reading_performance <= 0.0 {
            base_flashlight_performance
        } else if base_flashlight_performance <= 0.0 {
            base_reading_performance
        } else {
            crate::util::difficulty::norm(
                1.1,
                [
                    base_reading_performance,
                    base_flashlight_performance
                        * (base_flashlight_performance / base_reading_performance).clamp(0.25, 1.0),
                ],
            )
        };
        let base_performance = crate::util::difficulty::norm(
            1.1,
            [
                base_aim_performance,
                base_speed_performance,
                base_cognition_performance,
            ],
        );
        let star_rating = (base_performance * PERFORMANCE_BASE_MULTIPLIER).cbrt();

        attrs.aim = aim_rating;
        attrs.aim_difficult_slider_count = difficult_sliders;
        attrs.speed = speed_rating;
        attrs.flashlight = flashlight_rating;
        attrs.slider_factor = slider_factor;
        attrs.aim_difficult_strain_count = aim_difficult_strain_count;
        attrs.speed_difficult_strain_count = speed_difficult_strain_count;
        attrs.stars = star_rating;
        attrs.speed_note_count = speed.relevant_note_count();

        // upstream OsuDifficultyCalculator.cs:52-59
        // aimTopWeightedSliderFactor = aimNoSlidersTopWeightedSliderCount /
        //     max(1, aimNoSlidersDifficultStrainCount - aimNoSlidersTopWeightedSliderCount)
        // speedTopWeightedSliderFactor = speedTopWeightedSliderCount /
        //     max(1, speedDifficultStrainCount - speedTopWeightedSliderCount)
        let aim_no_sliders_difficult_strain_count =
            aim_no_sliders.count_top_weighted_strains(aim_no_sliders_difficulty_value);
        let aim_no_sliders_top_weighted_slider_count =
            aim_no_sliders.count_top_weighted_sliders(aim_no_sliders_difficulty_value);
        attrs.aim_top_weighted_slider_factor = aim_no_sliders_top_weighted_slider_count
            / f64::max(
                1.0,
                aim_no_sliders_difficult_strain_count - aim_no_sliders_top_weighted_slider_count,
            );

        let speed_top_weighted_slider_count =
            speed.count_top_weighted_sliders(speed_difficulty_value);
        attrs.speed_top_weighted_slider_factor = speed_top_weighted_slider_count
            / f64::max(
                1.0,
                speed_difficult_strain_count - speed_top_weighted_slider_count,
            );

        attrs.reading = reading_rating;
        attrs.reading_difficult_note_count = reading_difficult_note_count;
    }

    pub fn create_difficulty_objects<'a>(
        difficulty: &Difficulty,
        scaling_factor: &ScalingFactor,
        osu_objects: impl ExactSizeIterator<Item = Pin<&'a mut OsuObject>>,
        time_preempt: f64,
        overall_difficulty: f64,
    ) -> Vec<OsuDifficultyObject<'a>> {
        let clock_rate = difficulty.get_clock_rate();

        let mut osu_objects_iter = osu_objects
            .map(|h| OsuDifficultyObject::compute_slider_cursor_pos(h, scaling_factor.radius))
            .map(Pin::into_ref);

        let Some(mut last) = osu_objects_iter.next() else {
            return Vec::new();
        };

        let mut diff_objects = Vec::with_capacity(osu_objects_iter.len());

        for (idx, h) in osu_objects_iter.enumerate() {
            let diff_object = OsuDifficultyObject::new(
                h.get_ref(),
                last.get_ref(),
                &diff_objects,
                clock_rate,
                idx,
                scaling_factor,
                time_preempt,
                overall_difficulty,
            );

            diff_objects.push(diff_object);
            last = h;
        }

        diff_objects
    }
}

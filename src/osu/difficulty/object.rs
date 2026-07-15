use std::{borrow::Cow, pin::Pin};

use rosu_map::util::Pos;

use crate::{
    any::difficulty::object::{HasStartTime, IDifficultyObject},
    osu::object::{OsuObject, OsuObjectKind, OsuSlider},
};

use super::{scaling_factor::ScalingFactor, HD_FADE_OUT_DURATION_MULTIPLIER};

pub struct OsuDifficultyObject<'a> {
    pub idx: usize,
    pub base: &'a OsuObject,
    pub start_time: f64,
    pub end_time: f64,
    pub delta_time: f64,

    /// `DeltaTime` capped to 25ms. Named `strain_time` for API compatibility
    /// with the older port; this is lazer's `AdjustedDeltaTime`.
    pub strain_time: f64,
    pub last_object_end_delta_time: f64,
    pub preempt: f64,
    pub jump_dist: f64,
    pub lazy_jump_dist: f64,
    pub min_jump_dist: f64,
    pub min_jump_time: f64,
    pub travel_dist: f64,
    pub travel_time: f64,
    pub lazy_travel_dist: f64,
    pub lazy_travel_time: f64,
    pub angle: Option<f64>,
    pub normalised_vector_angle: Option<f64>,
    pub radius: f64,
    pub small_circle_bonus: f64,
    pub overall_difficulty: f64,
}

impl<'a> OsuDifficultyObject<'a> {
    pub const NORMALIZED_RADIUS: i32 = 50;
    pub const NORMALIZED_DIAMETER: i32 = Self::NORMALIZED_RADIUS * 2;

    pub const MIN_DELTA_TIME: f64 = 25.0;
    const MAX_SLIDER_RADIUS: f32 = Self::NORMALIZED_RADIUS as f32 * 2.4;
    const ASSUMED_SLIDER_RADIUS: f32 = Self::NORMALIZED_RADIUS as f32 * 1.8;

    pub fn new(
        hit_object: &'a OsuObject,
        last_object: &'a OsuObject,
        previous: &[Self],
        clock_rate: f64,
        idx: usize,
        scaling_factor: &ScalingFactor,
        time_preempt: f64,
        overall_difficulty: f64,
    ) -> Self {
        let delta_time = (hit_object.start_time - last_object.start_time) / clock_rate;
        let start_time = hit_object.start_time / clock_rate;
        let end_time = hit_object.end_time() / clock_rate;

        let strain_time = delta_time.max(Self::MIN_DELTA_TIME);
        let last_object_end_delta_time = previous.last().map_or(strain_time, |last| {
            (start_time - last.end_time).max(Self::MIN_DELTA_TIME)
        });
        let small_circle_bonus = (1.0 + (30.0 - scaling_factor.radius) / 70.0).max(1.0);

        let mut this = Self {
            idx,
            base: hit_object,
            start_time,
            end_time,
            delta_time,
            strain_time,
            last_object_end_delta_time,
            preempt: time_preempt / clock_rate,
            jump_dist: 0.0,
            lazy_jump_dist: 0.0,
            min_jump_dist: 0.0,
            min_jump_time: 0.0,
            travel_dist: 0.0,
            travel_time: 0.0,
            lazy_travel_dist: 0.0,
            lazy_travel_time: 0.0,
            angle: None,
            normalised_vector_angle: None,
            radius: scaling_factor.radius,
            small_circle_bonus,
            overall_difficulty,
        };

        this.set_distances(last_object, previous, clock_rate, scaling_factor);

        this
    }

    pub fn opacity_at(&self, time: f64, hidden: bool, time_preempt: f64, time_fade_in: f64) -> f64 {
        if time > self.base.start_time {
            // * Consider a hitobject as being invisible when its start time is passed.
            // * In reality the hitobject will be visible beyond its start time up until its hittable window has passed,
            // * but this is an approximation and such a case is unlikely to be hit where this function is used.
            return 0.0;
        }

        let fade_in_start_time = self.base.start_time - time_preempt;
        // This is intentionally the unmodified fade-in duration. Hidden only
        // changes `TimeFadeIn`, which is used as the fade-out start below.
        let fade_in_duration =
            400.0 * (time_preempt / crate::osu::object::OsuObject::PREEMPT_MIN).min(1.0);

        if hidden {
            // * Taken from OsuModHidden.
            let fade_out_start_time = self.base.start_time - time_preempt + time_fade_in;
            let fade_out_duration = time_preempt * HD_FADE_OUT_DURATION_MULTIPLIER;

            (((time - fade_in_start_time) / fade_in_duration).clamp(0.0, 1.0))
                .min(1.0 - ((time - fade_out_start_time) / fade_out_duration).clamp(0.0, 1.0))
        } else {
            ((time - fade_in_start_time) / fade_in_duration).clamp(0.0, 1.0)
        }
    }

    pub fn get_doubletapness(&self, next: Option<&Self>, hit_window: f64) -> f64 {
        let Some(next) = next else { return 0.0 };

        // Spinner and all its nested objects use HitWindows.Empty in lazer,
        // so DifficultyHitObject.HitWindowGreat resolves to zero.
        let hit_window = if self.base.is_spinner() {
            0.0
        } else {
            hit_window
        };

        let curr_delta_time = self.delta_time.max(1.0);
        let next_delta_time = next.delta_time.max(1.0);
        let delta_diff = (next_delta_time - curr_delta_time).abs();
        let speed_ratio = curr_delta_time / curr_delta_time.max(delta_diff);
        let window_ratio = (curr_delta_time / hit_window).min(1.0).powf(5.0);

        // Intersecting circles can be hit together; separated circles cannot.
        let distance_factor = crate::util::difficulty::reverse_lerp(
            self.lazy_jump_dist,
            f64::from(Self::NORMALIZED_DIAMETER),
            f64::from(Self::NORMALIZED_RADIUS),
        )
        .powi(2);

        1.0 - speed_ratio.powf(distance_factor * (1.0 - window_ratio))
    }

    fn set_distances(
        &mut self,
        last_object: &OsuObject,
        previous: &[Self],
        clock_rate: f64,
        scaling_factor: &ScalingFactor,
    ) {
        if let OsuObjectKind::Slider(ref slider) = self.base.kind {
            self.lazy_travel_dist = slider.lazy_travel_dist;
            self.lazy_travel_time = slider.lazy_travel_time;
            self.travel_dist =
                self.lazy_travel_dist * (slider.repeat_count() as f64).powf(0.3).max(1.0);
            self.travel_time = (self.lazy_travel_time / clock_rate).max(Self::MIN_DELTA_TIME);
        }

        self.min_jump_time = self.strain_time;

        if self.base.is_spinner() || last_object.is_spinner() {
            return;
        }

        let scaling_factor = scaling_factor.factor;

        let last_difficulty_object = previous.last();
        let last_last_difficulty_object = previous.iter().rev().nth(1);
        let mut last_cursor_pos = last_difficulty_object
            .map(Self::get_end_cursor_pos)
            .unwrap_or_else(|| last_object.stacked_pos());

        self.jump_dist = f64::from(
            (last_object.stacked_pos() - self.base.stacked_pos()).length() * scaling_factor,
        );

        self.lazy_jump_dist =
            f64::from((self.base.stacked_pos() - last_cursor_pos).length() * scaling_factor);
        self.min_jump_dist = self.lazy_jump_dist;

        if let (OsuObjectKind::Slider(ref last_slider), Some(last_difficulty_object)) =
            (&last_object.kind, last_difficulty_object)
        {
            let last_travel_time =
                (last_difficulty_object.lazy_travel_time / clock_rate).max(Self::MIN_DELTA_TIME);
            self.min_jump_time = (self.strain_time - last_travel_time).max(Self::MIN_DELTA_TIME);

            let tail_pos = last_slider.tail().map_or(last_object.pos, |tail| tail.pos);
            let stacked_tail_pos = tail_pos + last_object.stack_offset;

            let tail_jump_dist =
                (stacked_tail_pos - self.base.stacked_pos()).length() * scaling_factor;

            let diff = f64::from(
                OsuDifficultyObject::MAX_SLIDER_RADIUS - OsuDifficultyObject::ASSUMED_SLIDER_RADIUS,
            );

            let min = f64::from(tail_jump_dist - OsuDifficultyObject::MAX_SLIDER_RADIUS);
            self.min_jump_dist = ((self.lazy_jump_dist - diff).min(min)).max(0.0);
        }

        if let Some(last_last_difficulty_object) =
            last_last_difficulty_object.filter(|h| !h.base.is_spinner())
        {
            let last_difficulty_object = last_difficulty_object
                .expect("a second previous difficulty object implies a first one");

            if last_difficulty_object.base.is_slider() && last_difficulty_object.travel_dist > 0.0 {
                last_cursor_pos = last_difficulty_object.base.stacked_pos();
            }

            let last_last_cursor_pos = Self::get_end_cursor_pos(last_last_difficulty_object);
            let angle = Self::calculate_angle(
                self.base.stacked_pos(),
                last_cursor_pos,
                last_last_cursor_pos,
            );
            let slider_angle = Self::calculate_slider_angle(
                last_difficulty_object,
                self.base.stacked_pos(),
                last_last_cursor_pos,
            );

            let vector = self.base.stacked_pos() - last_cursor_pos;
            self.normalised_vector_angle =
                Some(f64::from(vector.y.abs()).atan2(f64::from(vector.x.abs())));
            self.angle = Some(angle.min(slider_angle));
        }
    }

    /// The [`Pin<&mut OsuObject>`](std::pin::Pin) denotes that the object will
    /// be mutated but not moved.
    pub fn compute_slider_cursor_pos(
        mut h: Pin<&mut OsuObject>,
        radius: f64,
    ) -> Pin<&mut OsuObject> {
        let pos = h.pos;
        let stack_offset = h.stack_offset;
        let start_time = h.start_time;

        let OsuObjectKind::Slider(ref mut slider) = h.kind else {
            return h;
        };

        let mut nested = Cow::Borrowed(slider.nested_objects.as_slice());
        let duration = slider.end_time - start_time;
        OsuSlider::lazy_travel_time(start_time, duration, &mut nested);
        let nested = nested.as_ref();

        let mut curr_cursor_pos = pos + stack_offset;
        let scaling_factor = f64::from(OsuDifficultyObject::NORMALIZED_RADIUS) / radius;

        for (curr_movement_obj, i) in nested.iter().zip(1..) {
            let mut curr_movement = curr_movement_obj.pos + stack_offset - curr_cursor_pos;
            let mut curr_movement_len = scaling_factor * f64::from(curr_movement.length());
            let mut required_movement = f64::from(OsuDifficultyObject::ASSUMED_SLIDER_RADIUS);

            if i == nested.len() {
                let lazy_movement = slider.lazy_end_pos - curr_cursor_pos;

                if lazy_movement.length() < curr_movement.length() {
                    curr_movement = lazy_movement;
                }

                curr_movement_len = scaling_factor * f64::from(curr_movement.length());
            } else if curr_movement_obj.is_repeat() {
                required_movement = f64::from(OsuDifficultyObject::NORMALIZED_RADIUS);
            }

            if curr_movement_len > required_movement {
                curr_cursor_pos += curr_movement
                    * ((curr_movement_len - required_movement) / curr_movement_len) as f32;
                curr_movement_len *= (curr_movement_len - required_movement) / curr_movement_len;
                slider.lazy_travel_dist += curr_movement_len;
            }

            if i == nested.len() {
                slider.lazy_end_pos = curr_cursor_pos;
            }
        }

        h
    }

    fn calculate_slider_angle(last: &Self, current_pos: Pos, mut last_last_cursor_pos: Pos) -> f64 {
        let last_cursor_pos = Self::get_end_cursor_pos(last);

        if let OsuObjectKind::Slider(slider) = &last.base.kind {
            if last.travel_dist > 0.0 {
                // lazer's nested list includes the slider head whereas ours
                // starts after it. For a head-tail-only slider the second-last
                // nested object is therefore the head.
                last_last_cursor_pos = slider
                    .nested_objects
                    .len()
                    .checked_sub(2)
                    .and_then(|idx| slider.nested_objects.get(idx))
                    .map_or_else(
                        || last.base.stacked_pos(),
                        |nested| nested.pos + last.base.stack_offset,
                    );
            }
        }

        Self::calculate_angle(current_pos, last_cursor_pos, last_last_cursor_pos)
    }

    fn calculate_angle(current: Pos, last: Pos, last_last: Pos) -> f64 {
        let v1 = last_last - last;
        let v2 = current - last;
        let dot = v1.dot(v2);
        let det = v1.x * v2.y - v1.y * v2.x;

        f64::from(det).atan2(f64::from(dot)).abs()
    }

    fn get_end_cursor_pos(hit_object: &Self) -> Pos {
        if let OsuObjectKind::Slider(ref slider) = hit_object.base.kind {
            slider.lazy_end_pos
        } else {
            hit_object.base.stacked_pos()
        }
    }
}

impl IDifficultyObject for OsuDifficultyObject<'_> {
    type DifficultyObjects = [Self];

    fn idx(&self) -> usize {
        self.idx
    }
}

impl HasStartTime for OsuDifficultyObject<'_> {
    fn start_time(&self) -> f64 {
        self.start_time
    }
}

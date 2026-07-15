use crate::{
    taiko::difficulty::object::TaikoDifficultyObject,
    util::{
        interval_grouping::HasInterval,
        sync::{RefCount, Weak},
    },
};

#[derive(Debug)]
pub struct SameRhythmHitObjectGrouping {
    pub hit_objects: Vec<Weak<TaikoDifficultyObject>>,
    /// Use [`Self::upgraded_previous`] to access
    previous: Option<Weak<SameRhythmHitObjectGrouping>>,
    pub hit_object_interval: Option<f64>,
    pub hit_object_interval_ratio: f64,
    pub interval: f64,
}

impl SameRhythmHitObjectGrouping {
    const SNAP_TOLERANCE: f64 = 5.0;

    pub fn new(
        previous: Option<Weak<Self>>,
        hit_objects: Vec<Weak<TaikoDifficultyObject>>,
    ) -> Self {
        let upgraded_prev = upgraded_previous(previous.as_ref());

        // Cluster delta times within the snap tolerance and replace each value
        // with its cluster median, matching lazer's `DeltaTimeNormaliser`.
        let normalised_delta_times = normalise_delta_times(&hit_objects, Self::SNAP_TOLERANCE);
        let modal_delta = normalised_delta_times
            .get(1)
            .map(|delta| delta.round_ties_even());

        let hit_object_interval = modal_delta.map(|modal_delta| {
            upgraded_prev
                .as_ref()
                .and_then(|prev| prev.get().hit_object_interval)
                .filter(|previous_delta| {
                    (modal_delta - previous_delta).abs() <= Self::SNAP_TOLERANCE
                })
                .unwrap_or(modal_delta)
        });

        // * Calculate the ratio between this group's interval and the previous group's interval
        let hit_object_interval_ratio = if let Some((prev, curr)) = upgraded_prev
            .as_ref()
            .and_then(|prev| prev.get().hit_object_interval)
            .zip(hit_object_interval)
        {
            curr / prev
        } else {
            1.0
        };

        // * Calculate the interval from the previous group's start time
        let interval = upgraded_prev
            .as_ref()
            .and_then(|prev| prev.get().start_time())
            .zip(start_time(&hit_objects))
            .map_or(f64::INFINITY, |(prev, curr)| {
                let interval = curr - prev;

                if interval.abs() <= Self::SNAP_TOLERANCE {
                    0.0
                } else {
                    interval
                }
            });

        Self {
            hit_objects,
            previous,
            hit_object_interval,
            hit_object_interval_ratio,
            interval,
        }
    }

    pub fn upgraded_previous(&self) -> Option<RefCount<Self>> {
        upgraded_previous(self.previous.as_ref())
    }

    pub fn first_hit_object(&self) -> Option<RefCount<TaikoDifficultyObject>> {
        first_hit_object(&self.hit_objects)
    }

    pub fn start_time(&self) -> Option<f64> {
        start_time(&self.hit_objects)
    }

    pub fn duration(&self) -> Option<f64> {
        duration(&self.hit_objects)
    }

    pub fn upgraded_hit_objects(
        &self,
    ) -> impl Iterator<Item = RefCount<TaikoDifficultyObject>> + use<'_> {
        self.hit_objects.iter().filter_map(Weak::upgrade)
    }
}

fn normalise_delta_times(
    hit_objects: &[Weak<TaikoDifficultyObject>],
    margin_of_error: f64,
) -> Vec<f64> {
    let object_delta_times: Vec<f64> = hit_objects
        .iter()
        .filter_map(Weak::upgrade)
        .map(|hit_object| hit_object.get().delta_time)
        .collect();

    let mut distinct_delta_times = object_delta_times.clone();
    distinct_delta_times.sort_by(f64::total_cmp);
    distinct_delta_times.dedup_by(|left, right| *left == *right);

    let mut sets: Vec<Vec<f64>> = Vec::new();

    for value in distinct_delta_times {
        if let Some(current) = sets
            .last_mut()
            .filter(|current| (value - current[0]).abs() <= margin_of_error)
        {
            current.push(value);
        } else {
            sets.push(vec![value]);
        }
    }

    let median_lookup: Vec<(f64, f64)> = sets
        .into_iter()
        .flat_map(|set| {
            let mid = set.len() / 2;
            let median = if set.len() % 2 == 1 {
                set[mid]
            } else {
                (set[mid - 1] + set[mid]) / 2.0
            };

            set.into_iter().map(move |value| (value, median))
        })
        .collect();

    object_delta_times
        .into_iter()
        .map(|delta_time| {
            median_lookup
                .binary_search_by(|(value, _)| value.total_cmp(&delta_time))
                .ok()
                .map(|idx| median_lookup[idx].1)
                .unwrap_or(delta_time)
        })
        .collect()
}

fn upgraded_previous(
    previous: Option<&Weak<SameRhythmHitObjectGrouping>>,
) -> Option<RefCount<SameRhythmHitObjectGrouping>> {
    previous.and_then(Weak::upgrade)
}

fn first_hit_object(
    hit_objects: &[Weak<TaikoDifficultyObject>],
) -> Option<RefCount<TaikoDifficultyObject>> {
    hit_objects.first().and_then(Weak::upgrade)
}

fn start_time(hit_objects: &[Weak<TaikoDifficultyObject>]) -> Option<f64> {
    first_hit_object(hit_objects).map(|h| h.get().start_time)
}

fn duration(hit_objects: &[Weak<TaikoDifficultyObject>]) -> Option<f64> {
    hit_objects
        .last()
        .and_then(Weak::upgrade)
        .zip(start_time(hit_objects))
        .map(|(last, start)| last.get().start_time - start)
}

impl HasInterval for SameRhythmHitObjectGrouping {
    fn interval(&self) -> f64 {
        self.interval
    }
}

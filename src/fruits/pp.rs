use super::{stars, FruitsDifficultyAttributes, FruitsPerformanceAttributes};
use crate::{Beatmap, DifficultyAttributes, Mods, PerformanceAttributes};

/// Performance calculator on osu!ctb maps.
///
/// # Example
///
/// ```
/// use rosu_pp::{FruitsPP, Beatmap};
///
/// # /*
/// let map: Beatmap = ...
/// # */
/// # let map = Beatmap::default();
///
/// let pp_result = FruitsPP::new(&map)
///     .mods(8 + 64) // HDDT
///     .combo(1234)
///     .misses(1)
///     .accuracy(98.5)
///     .calculate();
///
/// println!("PP: {} | Stars: {}", pp_result.pp(), pp_result.stars());
///
/// let next_result = FruitsPP::new(&map)
///     .attributes(pp_result)  // reusing previous results for performance
///     .mods(8 + 64)           // has to be the same to reuse attributes
///     .accuracy(99.5)
///     .calculate();
///
/// println!("PP: {} | Stars: {}", next_result.pp(), next_result.stars());
/// ```
#[derive(Clone, Debug)]
#[allow(clippy::upper_case_acronyms)]
pub struct FruitsPP<'map> {
    map: &'map Beatmap,
    attributes: Option<FruitsDifficultyAttributes>,
    mods: u32,
    combo: Option<usize>,

    n_fruits: Option<usize>,
    n_droplets: Option<usize>,
    n_tiny_droplets: Option<usize>,
    n_tiny_droplet_misses: Option<usize>,
    n_misses: usize,
    passed_objects: Option<usize>,
}

impl<'map> FruitsPP<'map> {
    /// Create a new performance calculator for osu!ctb maps.
    #[inline]
    pub fn new(map: &'map Beatmap) -> Self {
        Self {
            map,
            attributes: None,
            mods: 0,
            combo: None,

            n_fruits: None,
            n_droplets: None,
            n_tiny_droplets: None,
            n_tiny_droplet_misses: None,
            n_misses: 0,
            passed_objects: None,
        }
    }

    /// Provide the result of previous a difficulty or performance calculation.
    /// If you already calculated the attributes for the current map-mod combination,
    /// be sure to put them in here so that they don't have to be recalculated.
    #[inline]
    pub fn attributes(mut self, attributes: impl FruitsAttributeProvider) -> Self {
        if let Some(attributes) = attributes.attributes() {
            self.attributes.replace(attributes);
        }

        self
    }

    /// Specify mods through their bit values.
    ///
    /// See [https://github.com/ppy/osu-api/wiki#mods](https://github.com/ppy/osu-api/wiki#mods)
    #[inline]
    pub fn mods(mut self, mods: u32) -> Self {
        self.mods = mods;

        self
    }

    /// Specify the max combo of the play.
    #[inline]
    pub fn combo(mut self, combo: usize) -> Self {
        self.combo.replace(combo);

        self
    }

    /// Specify the amount of fruits of a play i.e. n300.
    #[inline]
    pub fn fruits(mut self, n_fruits: usize) -> Self {
        self.n_fruits.replace(n_fruits);

        self
    }

    /// Specify the amount of droplets of a play i.e. n100.
    #[inline]
    pub fn droplets(mut self, n_droplets: usize) -> Self {
        self.n_droplets.replace(n_droplets);

        self
    }

    /// Specify the amount of tiny droplets of a play i.e. n50.
    #[inline]
    pub fn tiny_droplets(mut self, n_tiny_droplets: usize) -> Self {
        self.n_tiny_droplets.replace(n_tiny_droplets);

        self
    }

    /// Specify the amount of tiny droplet misses of a play i.e. n_katu.
    #[inline]
    pub fn tiny_droplet_misses(mut self, n_tiny_droplet_misses: usize) -> Self {
        self.n_tiny_droplet_misses.replace(n_tiny_droplet_misses);

        self
    }

    /// Specify the amount of fruit / droplet misses of the play.
    #[inline]
    pub fn misses(mut self, n_misses: usize) -> Self {
        self.n_misses = n_misses;

        self
    }

    /// Amount of passed objects for partial plays, e.g. a fail.
    #[inline]
    pub fn passed_objects(mut self, passed_objects: usize) -> Self {
        self.passed_objects.replace(passed_objects);

        self
    }

    /// Generate the hit results with respect to the given accuracy between `0` and `100`.
    ///
    /// Be sure to set `misses` beforehand! Also, if available, set `attributes` beforehand.
    pub fn accuracy(mut self, mut acc: f64) -> Self {
        if self.attributes.is_none() {
            self.attributes = Some(stars(self.map, self.mods, self.passed_objects));
        }

        let attributes = self.attributes.as_ref().unwrap();

        let n_droplets = self
            .n_droplets
            .unwrap_or_else(|| attributes.n_droplets.saturating_sub(self.n_misses));

        let n_fruits = self.n_fruits.unwrap_or_else(|| {
            attributes
                .max_combo
                .saturating_sub(self.n_misses)
                .saturating_sub(n_droplets)
        });

        let max_tiny_droplets = attributes.n_tiny_droplets;
        acc /= 100.0;

        let n_tiny_droplets = self.n_tiny_droplets.unwrap_or_else(|| {
            ((acc * (attributes.max_combo + max_tiny_droplets) as f64).round() as usize)
                .saturating_sub(n_fruits)
                .saturating_sub(n_droplets)
        });

        let n_tiny_droplet_misses = max_tiny_droplets.saturating_sub(n_tiny_droplets);

        self.n_fruits.replace(n_fruits);
        self.n_droplets.replace(n_droplets);
        self.n_tiny_droplets.replace(n_tiny_droplets);
        self.n_tiny_droplet_misses.replace(n_tiny_droplet_misses);

        self
    }

    fn assert_hitresults(self, attributes: FruitsDifficultyAttributes) -> FruitsPPInner {
        let correct_combo_hits = self
            .n_fruits
            .and_then(|f| self.n_droplets.map(|d| f + d + self.n_misses))
            .filter(|h| *h == attributes.max_combo);

        let correct_fruits = self
            .n_fruits
            .filter(|f| *f >= attributes.n_fruits.saturating_sub(self.n_misses));

        let correct_droplets = self
            .n_droplets
            .filter(|d| *d >= attributes.n_droplets.saturating_sub(self.n_misses));

        let correct_tinies = self
            .n_tiny_droplets
            .and_then(|t| self.n_tiny_droplet_misses.map(|m| t + m))
            .filter(|h| *h == attributes.n_tiny_droplets);

        if correct_combo_hits
            .and(correct_fruits)
            .and(correct_droplets)
            .and(correct_tinies)
            .is_none()
        {
            let mut n_fruits = self.n_fruits.unwrap_or(0);
            let mut n_droplets = self.n_droplets.unwrap_or(0);
            let mut n_tiny_droplets = self.n_tiny_droplets.unwrap_or(0);
            let n_tiny_droplet_misses = self.n_tiny_droplet_misses.unwrap_or(0);

            let missing = attributes
                .max_combo
                .saturating_sub(n_fruits)
                .saturating_sub(n_droplets)
                .saturating_sub(self.n_misses);

            let missing_fruits =
                missing.saturating_sub(attributes.n_droplets.saturating_sub(n_droplets));

            n_fruits += missing_fruits;
            n_droplets += missing.saturating_sub(missing_fruits);
            n_tiny_droplets += attributes
                .n_tiny_droplets
                .saturating_sub(n_tiny_droplets)
                .saturating_sub(n_tiny_droplet_misses);

            return FruitsPPInner {
                attributes,
                mods: self.mods,
                combo: self.combo,
                n_fruits,
                n_droplets,
                n_tiny_droplets,
                n_tiny_droplet_misses,
                n_misses: self.n_misses,
            };
        }

        FruitsPPInner {
            attributes,
            mods: self.mods,
            combo: self.combo,
            n_fruits: self.n_fruits.unwrap_or(0),
            n_droplets: self.n_droplets.unwrap_or(0),
            n_tiny_droplets: self.n_tiny_droplets.unwrap_or(0),
            n_tiny_droplet_misses: self.n_tiny_droplet_misses.unwrap_or(0),
            n_misses: self.n_misses,
        }
    }

    /// Calculate all performance related values, including pp and stars.
    pub fn calculate(mut self) -> FruitsPerformanceAttributes {
        let attributes = self
            .attributes
            .take()
            .unwrap_or_else(|| stars(self.map, self.mods, self.passed_objects));

        self.assert_hitresults(attributes).calculate()
    }
}

struct FruitsPPInner {
    attributes: FruitsDifficultyAttributes,
    mods: u32,
    combo: Option<usize>,
    n_fruits: usize,
    n_droplets: usize,
    n_tiny_droplets: usize,
    n_tiny_droplet_misses: usize,
    n_misses: usize,
}

impl FruitsPPInner {
    fn calculate(self) -> FruitsPerformanceAttributes {
        let attributes = &self.attributes;
        let stars = attributes.stars;

        // Relying heavily on aim
        let mut pp = (5.0 * (stars / 0.0049).max(1.0) - 4.0).powi(2) / 100_000.0;

        let mut combo_hits = self.combo_hits();

        if combo_hits == 0 {
            combo_hits = attributes.max_combo;
        }

        // Longer maps are worth more
        let len_bonus = 0.95
            + 0.3 * (combo_hits as f64 / 2500.0).min(1.0)
            + (combo_hits > 2500) as u8 as f64 * (combo_hits as f64 / 2500.0).log10() * 0.475;

        pp *= len_bonus;

        // Penalize misses exponentially
        pp *= 0.97_f64.powi(self.n_misses as i32);

        // Combo scaling
        if let Some(combo) = self.combo.filter(|_| attributes.max_combo > 0) {
            pp *= (combo as f64 / attributes.max_combo as f64)
                .powf(0.8)
                .min(1.0);
        }

        // AR scaling
        let ar = attributes.ar;
        let mut ar_factor = 1.0;
        if ar > 9.0 {
            ar_factor += 0.1 * (ar - 9.0) + (ar > 10.0) as u8 as f64 * 0.1 * (ar - 10.0);
        } else if ar < 8.0 {
            ar_factor += 0.025 * (8.0 - ar);
        }
        pp *= ar_factor;

        // HD bonus
        if self.mods.hd() {
            if ar <= 10.0 {
                pp *= 1.05 + 0.075 * (10.0 - ar);
            } else if ar > 10.0 {
                pp *= 1.01 + 0.04 * (11.0 - ar.min(11.0));
            }
        }

        // FL bonus
        if self.mods.fl() {
            pp *= 1.35 * len_bonus;
        }

        // Accuracy scaling
        pp *= self.acc().powf(5.5);

        // NF penalty
        if self.mods.nf() {
            pp *= 0.9;
        }

        FruitsPerformanceAttributes {
            attributes: self.attributes,
            pp,
        }
    }

    #[inline]
    fn combo_hits(&self) -> usize {
        self.n_fruits + self.n_droplets + self.n_misses
    }

    #[inline]
    fn successful_hits(&self) -> usize {
        self.n_fruits + self.n_droplets + self.n_tiny_droplets
    }

    #[inline]
    fn total_hits(&self) -> usize {
        self.successful_hits() + self.n_tiny_droplet_misses + self.n_misses
    }

    #[inline]
    fn acc(&self) -> f64 {
        let total_hits = self.total_hits();

        if total_hits == 0 {
            1.0
        } else {
            (self.successful_hits() as f64 / total_hits as f64)
                .max(0.0)
                .min(1.0)
        }
    }
}

/// Abstract type to provide flexibility when passing difficulty attributes to a performance calculation.
pub trait FruitsAttributeProvider {
    /// Provide the actual difficulty attributes.
    fn attributes(self) -> Option<FruitsDifficultyAttributes>;
}

impl FruitsAttributeProvider for FruitsDifficultyAttributes {
    #[inline]
    fn attributes(self) -> Option<FruitsDifficultyAttributes> {
        Some(self)
    }
}

impl FruitsAttributeProvider for FruitsPerformanceAttributes {
    #[inline]
    fn attributes(self) -> Option<FruitsDifficultyAttributes> {
        Some(self.attributes)
    }
}

impl FruitsAttributeProvider for DifficultyAttributes {
    #[inline]
    fn attributes(self) -> Option<FruitsDifficultyAttributes> {
        #[allow(irrefutable_let_patterns)]
        if let Self::Fruits(attributes) = self {
            Some(attributes)
        } else {
            None
        }
    }
}

impl FruitsAttributeProvider for PerformanceAttributes {
    #[inline]
    fn attributes(self) -> Option<FruitsDifficultyAttributes> {
        #[allow(irrefutable_let_patterns)]
        if let Self::Fruits(attributes) = self {
            Some(attributes.attributes)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::Beatmap;

    fn attributes() -> FruitsDifficultyAttributes {
        FruitsDifficultyAttributes {
            n_fruits: 1234,
            n_droplets: 567,
            n_tiny_droplets: 2345,
            max_combo: 1234 + 567,
            ..Default::default()
        }
    }

    #[test]
    fn fruits_only_accuracy() {
        let map = Beatmap::default();
        let attributes = attributes();

        let total_objects = attributes.n_fruits + attributes.n_droplets;
        let target_acc = 97.5;

        let calculator = FruitsPP::new(&map)
            .attributes(attributes)
            .passed_objects(total_objects)
            .accuracy(target_acc);

        let numerator = calculator.n_fruits.unwrap_or(0)
            + calculator.n_droplets.unwrap_or(0)
            + calculator.n_tiny_droplets.unwrap_or(0);
        let denominator =
            numerator + calculator.n_tiny_droplet_misses.unwrap_or(0) + calculator.n_misses;
        let acc = 100.0 * numerator as f64 / denominator as f64;

        assert!(
            (target_acc - acc).abs() < 1.0,
            "Expected: {} | Actual: {}",
            target_acc,
            acc
        );
    }

    #[test]
    fn fruits_accuracy_droplets_and_tiny_droplets() {
        let map = Beatmap::default();
        let attributes = attributes();

        let total_objects = attributes.n_fruits + attributes.n_droplets;
        let target_acc = 97.5;
        let n_droplets = 550;
        let n_tiny_droplets = 2222;

        let calculator = FruitsPP::new(&map)
            .attributes(attributes)
            .passed_objects(total_objects)
            .droplets(n_droplets)
            .tiny_droplets(n_tiny_droplets)
            .accuracy(target_acc);

        assert_eq!(
            n_droplets,
            calculator.n_droplets.unwrap(),
            "Expected: {} | Actual: {}",
            n_droplets,
            calculator.n_droplets.unwrap()
        );

        let numerator = calculator.n_fruits.unwrap_or(0)
            + calculator.n_droplets.unwrap_or(0)
            + calculator.n_tiny_droplets.unwrap_or(0);
        let denominator =
            numerator + calculator.n_tiny_droplet_misses.unwrap_or(0) + calculator.n_misses;
        let acc = 100.0 * numerator as f64 / denominator as f64;

        assert!(
            (target_acc - acc).abs() < 1.0,
            "Expected: {} | Actual: {}",
            target_acc,
            acc
        );
    }

    #[test]
    fn fruits_missing_objects() {
        let map = Beatmap::default();
        let attributes = attributes();

        let total_objects = attributes.n_fruits + attributes.n_droplets;
        let n_fruits = attributes.n_fruits - 10;
        let n_droplets = attributes.n_droplets - 5;
        let n_tiny_droplets = attributes.n_tiny_droplets - 50;
        let n_tiny_droplet_misses = 20;
        let n_misses = 2;

        let calculator = FruitsPP::new(&map)
            .attributes(attributes.clone())
            .passed_objects(total_objects)
            .fruits(n_fruits)
            .droplets(n_droplets)
            .tiny_droplets(n_tiny_droplets)
            .tiny_droplet_misses(n_tiny_droplet_misses)
            .misses(n_misses)
            .assert_hitresults(attributes.clone());

        assert!(
            (attributes.n_fruits as i32 - calculator.n_fruits as i32).abs() <= n_misses as i32,
            "Expected: {} | Actual: {} [+/- {} misses]",
            attributes.n_fruits,
            calculator.n_fruits,
            n_misses
        );

        assert_eq!(
            attributes.n_droplets,
            calculator.n_droplets - (n_misses - (attributes.n_fruits - calculator.n_fruits)),
            "Expected: {} | Actual: {}",
            attributes.n_droplets,
            calculator.n_droplets - (n_misses - (attributes.n_fruits - calculator.n_fruits)),
        );

        assert_eq!(
            attributes.n_tiny_droplets,
            calculator.n_tiny_droplets + calculator.n_tiny_droplet_misses,
            "Expected: {} | Actual: {}",
            attributes.n_tiny_droplets,
            calculator.n_tiny_droplets + calculator.n_tiny_droplet_misses,
        );
    }
}

//! upstream `osu.Game.Rulesets.Osu.Difficulty.OsuLegacyScoreSimulator` +
//! `LegacyScoreUtils` の Rust 移植。
//!
//! `LegacyScoreBaseMultiplier` / `NestedScorePerObject` /
//! `MaximumLegacyComboScore` の 3 attribute を計算し、OsuDifficultyAttributes に
//! set する。これらが埋まると `OsuLegacyScoreMissCalculator` が classic score
//! で score-based miss 推定を実行できるようになる。

pub mod miss_calculator;
pub mod simulator;
pub mod utils;

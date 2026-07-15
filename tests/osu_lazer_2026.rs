use mames_pp::{
    osu::{
        Osu, OsuDifficultyAttributes, OsuGradualDifficulty, OsuPerformance,
        OsuPerformanceAttributes,
    },
    Beatmap, Difficulty, GameMods,
};
use rosu_mods::{GameModIntermode, GameModsIntermode};

const PRECISE_EPSILON: f64 = 1e-9;
const LAZER_DIFFICULTY_EPSILON: f64 = 1e-5;

fn assert_close(actual: f64, expected: f64, epsilon: f64, name: &str) {
    let difference = (actual - expected).abs();

    assert!(
        difference <= epsilon,
        "{name}: expected {expected:?}, got {actual:?} (difference {difference:?})",
    );
}

fn map(name: &str) -> Beatmap {
    Beatmap::from_path(format!("./resources/{name}.osu")).unwrap()
}

fn intermode(game_mod: GameModIntermode) -> GameMods {
    let mut mods = GameModsIntermode::new();
    mods.insert(game_mod);

    mods.into()
}

fn difficulty(map: &Beatmap, mods: GameMods, lazer: bool) -> OsuDifficultyAttributes {
    Difficulty::new()
        .mods(mods)
        .lazer(lazer)
        .calculate_for_mode::<Osu>(map)
        .unwrap()
}

fn ss(map: &Beatmap, mods: GameMods) -> OsuPerformanceAttributes {
    let attrs = difficulty(map, mods.clone(), true);

    OsuPerformance::new(attrs.clone())
        .mods(mods)
        .combo(attrs.max_combo)
        .n300(attrs.n_objects())
        .n100(0)
        .n50(0)
        .misses(0)
        .calculate()
        .unwrap()
}

fn imperfect(map: &Beatmap, mods: GameMods, lazer: bool) -> OsuPerformance<'_> {
    let attrs = difficulty(map, mods.clone(), lazer);
    let calc = OsuPerformance::new(attrs.clone())
        .mods(mods)
        .lazer(lazer)
        .combo(200)
        .n300(120)
        .n100(2)
        .n50(1)
        .misses(1);

    if lazer {
        calc.slider_end_hits(30)
            .large_tick_hits(attrs.n_large_ticks - 2)
    } else {
        calc
    }
}

#[test]
fn lazer_difficulty_reference_maps() {
    let cases = [
        ("diffcalc-test", 6.524317026548358, 239),
        ("zero-length-sliders", 1.3280410795791415, 54),
        ("very-fast-slider", 0.4086732514769756, 4),
        ("nan-slider", 0.8705817579435355, 6),
    ];

    for (name, expected_stars, expected_combo) in cases {
        let attrs = difficulty(&map(name), 0_u32.into(), true);
        assert_close(
            attrs.stars,
            expected_stars,
            LAZER_DIFFICULTY_EPSILON,
            &format!("{name} NM stars"),
        );
        assert_eq!(attrs.max_combo, expected_combo, "{name} NM max combo");
    }

    let cases = [
        ("diffcalc-test", 9.46776079006463, 239),
        ("zero-length-sliders", 1.6856612715618886, 54),
        ("very-fast-slider", 0.5358847318657256, 4),
    ];

    for (name, expected_stars, expected_combo) in cases {
        let attrs = difficulty(&map(name), 64_u32.into(), true);
        assert_close(
            attrs.stars,
            expected_stars,
            LAZER_DIFFICULTY_EPSILON,
            &format!("{name} DT stars"),
        );
        assert_eq!(attrs.max_combo, expected_combo, "{name} DT max combo");
    }
}

#[test]
fn lazer_difficulty_components_and_legacy_score_attributes() {
    let attrs = difficulty(&map("diffcalc-test"), 0_u32.into(), true);

    for (name, actual, expected) in [
        ("stars", attrs.stars, 6.524323005451469),
        ("aim", attrs.aim, 3.8093990088670195),
        ("speed", attrs.speed, 1.564710574509105),
        ("reading", attrs.reading, 1.7803497292117219),
        ("flashlight", attrs.flashlight, 0.0),
        ("slider factor", attrs.slider_factor, 0.9988227250155558),
        (
            "aim difficult strains",
            attrs.aim_difficult_strain_count,
            28.73087574834563,
        ),
        (
            "speed difficult strains",
            attrs.speed_difficult_strain_count,
            29.206477306149882,
        ),
        ("speed notes", attrs.speed_note_count, 35.00731556214938),
        (
            "difficult sliders",
            attrs.aim_difficult_slider_count,
            11.137949908959307,
        ),
        (
            "aim top weighted slider factor",
            attrs.aim_top_weighted_slider_factor,
            0.000555251240443441,
        ),
        (
            "speed top weighted slider factor",
            attrs.speed_top_weighted_slider_factor,
            0.00060855352799461,
        ),
        (
            "reading difficult notes",
            attrs.reading_difficult_note_count,
            32.18033420358422,
        ),
        (
            "nested score per object",
            attrs.nested_score_per_object,
            172.90322580645162,
        ),
        (
            "legacy score base multiplier",
            attrs.legacy_score_base_multiplier,
            3.0,
        ),
        (
            "maximum legacy combo score",
            attrs.maximum_legacy_combo_score,
            389_808.0,
        ),
    ] {
        assert_close(actual, expected, PRECISE_EPSILON, name);
    }

    assert_eq!(attrs.n_circles, 79);
    assert_eq!(attrs.n_sliders, 33);
    assert_eq!(attrs.n_spinners, 12);
    assert_eq!(attrs.n_large_ticks, 82);
    assert_eq!(attrs.max_combo, 239);
}

#[test]
fn lazer_timed_difficulty_reference() {
    let map = map("diffcalc-test");
    let attrs: Vec<_> = OsuGradualDifficulty::new(Difficulty::new(), &map)
        .unwrap()
        .collect();

    for (index, expected_stars, expected_combo) in [
        (0, 0.0, 1),
        (1, 0.16948857823852109, 2),
        (2, 0.26606949612932967, 3),
        (3, 0.35775631754708426, 4),
        (4, 0.5631934359955623, 5),
    ] {
        assert_close(
            attrs[index].stars,
            expected_stars,
            LAZER_DIFFICULTY_EPSILON,
            "early timed stars",
        );
        assert_eq!(attrs[index].max_combo, expected_combo);
    }

    for (combo, expected_stars) in [
        (62, 5.668546540678204),
        (63, 5.72726693627777),
        (64, 5.734776255596153),
        (160, 6.524056585255374),
        (176, 6.524060977792584),
        (195, 6.524061780698326),
    ] {
        let timed = attrs.iter().find(|attrs| attrs.max_combo == combo).unwrap();
        assert_close(
            timed.stars,
            expected_stars,
            LAZER_DIFFICULTY_EPSILON,
            &format!("timed stars at combo {combo}"),
        );
    }
}

#[test]
fn lazer_performance_ss_mod_branches() {
    let map = map("diffcalc-test");

    let nm = ss(&map, 0_u32.into());
    for (name, actual, expected) in [
        ("NM total", nm.pp, 291.1510007361912),
        ("NM aim", nm.pp_aim, 214.8629763621098),
        ("NM speed", nm.pp_speed, 12.604838421331243),
        ("NM accuracy", nm.pp_acc, 27.715025148546076),
        ("NM reading", nm.pp_reading, 22.572307597136877),
        ("NM flashlight", nm.pp_flashlight, 0.0),
        (
            "NM speed deviation",
            nm.speed_deviation.unwrap(),
            23.389240815832192,
        ),
        ("DT total", ss(&map, 64_u32.into()).pp, 878.0491184948831),
        ("FL total", ss(&map, 1024_u32.into()).pp, 311.1421693304931),
        (
            "FL component",
            ss(&map, 1024_u32.into()).pp_flashlight,
            24.907422014865507,
        ),
        ("AP total", ss(&map, 8192_u32.into()).pp, 35.62142465118087),
        ("HD total", ss(&map, 8_u32.into()).pp, 309.85969270030483),
        ("HR total", ss(&map, 16_u32.into()).pp, 412.6240441644093),
        (
            "Blinds total",
            ss(&map, intermode(GameModIntermode::Blinds)).pp,
            410.31449461155665,
        ),
        (
            "Traceable total",
            ss(&map, intermode(GameModIntermode::Traceable)).pp,
            321.41044546460927,
        ),
        (
            "SpunOut total",
            ss(&map, 4096_u32.into()).pp,
            251.15524439494553,
        ),
    ] {
        assert_close(actual, expected, PRECISE_EPSILON, name);
    }

    let ap = ss(&map, 8192_u32.into());
    assert_eq!(ap.pp_aim, 0.0);
    assert_close(ap.pp_speed, 4.808139280927651, PRECISE_EPSILON, "AP speed");
}

#[test]
fn lazer_performance_imperfect_classic_and_score_v2() {
    let map = map("diffcalc-test");

    let nm = imperfect(&map, 0_u32.into(), true).calculate().unwrap();
    for (name, actual, expected) in [
        ("NM imperfect total", nm.pp, 229.23521971545478),
        ("NM imperfect aim", nm.pp_aim, 177.15200507604442),
        ("NM imperfect speed", nm.pp_speed, 7.926745743985033),
        ("NM imperfect accuracy", nm.pp_acc, 13.924162221104808),
        ("NM imperfect reading", nm.pp_reading, 17.50984981216188),
        ("NM effective misses", nm.effective_miss_count, 1.18),
        ("NM combo misses", nm.combo_based_estimated_miss_count, 1.18),
        (
            "NM imperfect deviation",
            nm.speed_deviation.unwrap(),
            32.18687617388791,
        ),
        (
            "NF imperfect total",
            imperfect(&map, 1_u32.into(), true).calculate().unwrap().pp,
            223.82526853017004,
        ),
        (
            "ScoreV2 imperfect total",
            imperfect(&map, intermode(GameModIntermode::ScoreV2), true)
                .calculate()
                .unwrap()
                .pp,
            229.23521971545478,
        ),
    ] {
        assert_close(actual, expected, PRECISE_EPSILON, name);
    }

    let classic = imperfect(&map, 0_u32.into(), false).calculate().unwrap();
    for (name, actual, expected) in [
        ("classic total", classic.pp, 227.43105814203795),
        ("classic aim", classic.pp_aim, 178.67118454660095),
        ("classic speed", classic.pp_speed, 7.933013026488546),
        ("classic accuracy", classic.pp_acc, 9.350424270192487),
        ("classic reading", classic.pp_reading, 17.917042087426),
        (
            "classic effective misses",
            classic.effective_miss_count,
            1.168399996947791,
        ),
        (
            "classic aim slider breaks",
            classic.aim_estimated_slider_breaks,
            3.9735017773634594e-6,
        ),
        (
            "classic speed slider breaks",
            classic.speed_estimated_slider_breaks,
            4.354908675485977e-6,
        ),
    ] {
        assert_close(actual, expected, PRECISE_EPSILON, name);
    }

    // The legacy integer bit for ScoreV2 must be recognised too. This checks
    // the ScoreV2 exception that includes sliders in accuracy pp.
    let classic_score_v2 = imperfect(&map, 536_870_912_u32.into(), false)
        .calculate()
        .unwrap();
    assert_close(
        classic_score_v2.pp,
        231.27424955355528,
        PRECISE_EPSILON,
        "classic ScoreV2 total",
    );
    assert_close(
        classic_score_v2.pp_acc,
        13.924162221104808,
        PRECISE_EPSILON,
        "classic ScoreV2 accuracy",
    );
}

#[test]
fn lazer_legacy_score_miss_estimation() {
    let map = map("diffcalc-test");
    let attrs = imperfect(&map, 0_u32.into(), false)
        .legacy_total_score(250_000)
        .calculate()
        .unwrap();

    for (name, actual, expected) in [
        ("legacy score total", attrs.pp, 222.9124271871467),
        ("legacy score aim", attrs.pp_aim, 174.9885695089992),
        ("legacy score speed", attrs.pp_speed, 7.7702204788656575),
        ("legacy score reading", attrs.pp_reading, 17.558654820495565),
        (
            "legacy effective misses",
            attrs.effective_miss_count,
            1.4756347294773247,
        ),
        (
            "legacy combo misses",
            attrs.combo_based_estimated_miss_count,
            1.168399996947791,
        ),
        (
            "legacy score misses",
            attrs.score_based_estimated_miss_count.unwrap(),
            1.4756347294773247,
        ),
        (
            "legacy aim slider breaks",
            attrs.aim_estimated_slider_breaks,
            3.079603957617613e-5,
        ),
        (
            "legacy speed slider breaks",
            attrs.speed_estimated_slider_breaks,
            3.375200378321988e-5,
        ),
    ] {
        assert_close(actual, expected, PRECISE_EPSILON, name);
    }
}

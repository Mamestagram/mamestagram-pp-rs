use mames_pp::{
    taiko::{
        Taiko, TaikoDifficultyAttributes, TaikoGradualDifficulty, TaikoPerformance,
        TaikoPerformanceAttributes,
    },
    Beatmap, Difficulty,
};

const EPSILON: f64 = 1e-12;

fn assert_close(actual: f64, expected: f64, name: &str) {
    let difference = (actual - expected).abs();

    assert!(
        difference <= EPSILON,
        "{name}: expected {expected:?}, got {actual:?} (difference {difference:?})",
    );
}

fn map() -> Beatmap {
    // Copied from lazer's osu.Game.Rulesets.Taiko difficulty test resources.
    Beatmap::from_path("./resources/taiko-diffcalc-test.osu").unwrap()
}

fn difficulty(map: &Beatmap, mods: u32) -> TaikoDifficultyAttributes {
    Difficulty::new()
        .mods(mods)
        .calculate_for_mode::<Taiko>(map)
        .unwrap()
}

fn ss(attrs: TaikoDifficultyAttributes, mods: u32) -> TaikoPerformanceAttributes {
    let max_combo = attrs.max_combo;

    TaikoPerformance::new(attrs)
        .mods(mods)
        .combo(max_combo)
        .n300(max_combo)
        .calculate()
        .unwrap()
}

fn imperfect(attrs: TaikoDifficultyAttributes, mods: u32) -> TaikoPerformanceAttributes {
    TaikoPerformance::new(attrs)
        .mods(mods)
        .combo(120)
        .n300(180)
        .n100(15)
        .misses(5)
        .calculate()
        .unwrap()
}

#[test]
fn lazer_difficulty_attributes() {
    let map = map();

    let cases = [
        (
            "NM",
            0,
            3.319084940658167,
            0.028816271027626596,
            3.6524510383682645e-9,
            0.711226393181619,
            2.5790422727964706,
            3.2902686659780898,
            5.0442201096101396e-5,
            53.84465596592382,
            0.5670264868917223,
            28.5,
            67.5,
        ),
        (
            "DT",
            64,
            4.455142137225538,
            0.04378682744127979,
            1.4093430743067654e-5,
            0.8476265558773907,
            3.5637146604761245,
            4.411341216353515,
            2.8856511532511586e-5,
            57.57070100448967,
            0.5621663391093243,
            19.0,
            45.0,
        ),
        (
            "HR",
            16,
            3.319087606292086,
            0.029127446214986422,
            0.0038148740659474673,
            0.7103350809638885,
            2.5758102050472638,
            3.2861452860111524,
            5.0442201096101396e-5,
            53.84465596592382,
            0.5670272609576724,
            19.5,
            50.5,
        ),
    ];

    for (
        name,
        mods,
        stars,
        rhythm,
        reading,
        color,
        stamina,
        mechanical,
        mono,
        top_strains,
        consistency,
        great_window,
        ok_window,
    ) in cases
    {
        let attrs = difficulty(&map, mods);

        for (component, actual, expected) in [
            ("stars", attrs.stars, stars),
            ("rhythm", attrs.rhythm, rhythm),
            ("reading", attrs.reading, reading),
            ("color", attrs.color, color),
            ("stamina", attrs.stamina, stamina),
            ("mechanical", attrs.mechanical_difficulty, mechanical),
            ("mono stamina factor", attrs.mono_stamina_factor, mono),
            (
                "stamina top strains",
                attrs.stamina_top_strains,
                top_strains,
            ),
            ("consistency", attrs.consistency_factor, consistency),
            ("great hit window", attrs.great_hit_window, great_window),
            ("ok hit window", attrs.ok_hit_window, ok_window),
        ] {
            assert_close(actual, expected, &format!("{name} {component}"));
        }

        assert_eq!(attrs.max_combo, 200, "{name} max combo");
    }
}

#[test]
fn lazer_performance_reference() {
    let map = map();

    for (name, mods, expected_ss, expected_imperfect) in [
        ("NM", 0, 168.8723016556401, 92.08177751812931),
        ("DT", 64, 324.15879494305017, 217.44816754242584),
        ("HD", 8, 178.7943689047284, 96.67110040458707),
        ("HR", 16, 241.60549382258074, 148.4901904879706),
        ("FL", 1024, 171.3662104203851, 94.18792602649452),
    ] {
        let attrs = difficulty(&map, mods);
        let ss = ss(attrs.clone(), mods);
        let imperfect = imperfect(attrs, mods);

        assert_close(ss.pp, expected_ss, &format!("{name} SS total"));
        assert_close(
            imperfect.pp,
            expected_imperfect,
            &format!("{name} imperfect total"),
        );
    }

    let nm = ss(difficulty(&map, 0), 0);
    assert_close(nm.pp_difficulty, 49.87918863790332, "NM SS difficulty");
    assert_close(nm.pp_acc, 118.99311301773677, "NM SS accuracy");
    assert_close(
        nm.estimated_unstable_rate.unwrap(),
        128.34017825301785,
        "NM SS unstable rate",
    );
}

#[test]
fn lazer_gradual_ends_on_regular_attributes() {
    let map = map();

    for mods in [0, 64, 16] {
        let regular = difficulty(&map, mods);
        let gradual: Vec<_> = TaikoGradualDifficulty::new(Difficulty::new().mods(mods), &map)
            .unwrap()
            .collect();

        assert_eq!(gradual.last().unwrap(), &regular, "mods={mods}");

        if mods == 0 {
            for (combo, expected_stars) in [
                (3, 0.05719459561422588),
                (50, 2.423808608432519),
                (100, 3.0975528930661853),
                (117, 3.3095182066286664),
                (118, 3.310213109283213),
                (119, 3.3111994557418964),
                (120, 3.3115369517519393),
                (150, 3.3162295789601277),
                (200, 3.319084940658167),
            ] {
                assert_close(
                    gradual[combo - 1].stars,
                    expected_stars,
                    &format!("NM gradual stars at combo {combo}"),
                );
            }
        }
    }
}

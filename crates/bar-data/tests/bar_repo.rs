//! Tests against a real Beyond-All-Reason checkout. They run only when
//! `BAR_ROOT` points to one, for example:
//! `BAR_ROOT=../Beyond-All-Reason cargo test -p bar-data`

use std::path::PathBuf;

use bar_data::UnitDefs;

fn bar_root() -> Option<PathBuf> {
    std::env::var_os("BAR_ROOT").map(PathBuf::from)
}

#[test]
fn loads_t1_economy_units() {
    let Some(root) = bar_root() else {
        eprintln!("BAR_ROOT is not set, skipping");
        return;
    };
    let defs = UnitDefs::load(&root).unwrap();
    assert!(defs.len() > 300, "only {} unit defs", defs.len());

    let com = defs.get("armcom").unwrap();
    assert!(com.is_commander() && com.is_builder && !com.is_factory);
    assert!(com.build_options.iter().any(|o| o == "armmex"));

    let mex = defs.get("armmex").unwrap();
    assert!(mex.extracts_metal > 0.0);
    assert!(defs.get("armsolar").unwrap().energy_upkeep < 0.0);
    assert!(defs.get("armwin").unwrap().wind_generator > 0.0);
    assert!(defs.get("armmakr").unwrap().energy_conversion().is_some());
    assert!(defs.get("armlab").unwrap().is_factory);
    assert_eq!(defs.get("armmex").unwrap().human_name, "Metal Extractor");
}

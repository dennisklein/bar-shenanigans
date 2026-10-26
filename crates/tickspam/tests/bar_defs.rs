//! Checks the reference catalog against the real BAR unit defs.
//! Set `BAR_ROOT` to a Beyond-All-Reason checkout to run it.

use std::path::PathBuf;

use tickspam::catalog::{Catalog, Role};

#[test]
fn reference_catalog_matches_bar_unit_defs() {
    let Some(root) = std::env::var_os("BAR_ROOT").map(PathBuf::from) else {
        eprintln!("BAR_ROOT not set, skipped");
        return;
    };
    let defs = bar_data::UnitDefs::load(&root).unwrap();
    let real = Catalog::from_unit_defs(&defs).unwrap();
    let reference = Catalog::reference();
    for role in Role::ALL {
        let (a, b) = (real.get(role), reference.get(role));
        // the reference does not carry every value, compare the ones it has
        let fields = [
            ("metal_cost", a.metal_cost, b.metal_cost),
            ("energy_cost", a.energy_cost, b.energy_cost),
            ("build_time", a.build_time, b.build_time),
            ("worker_time", a.worker_time, b.worker_time),
            ("metal_make", a.metal_make, b.metal_make),
            ("energy_make", a.energy_make, b.energy_make),
            ("energy_upkeep", a.energy_upkeep, b.energy_upkeep),
            ("wind_generator", a.wind_generator, b.wind_generator),
            ("extracts_metal", a.extracts_metal, b.extracts_metal),
            ("metal_storage", a.metal_storage, b.metal_storage),
            ("energy_storage", a.energy_storage, b.energy_storage),
        ];
        for (field, x, y) in fields {
            assert!((x - y).abs() < 1e-4, "{}.{field}: BAR {x}, reference {y}", a.name);
        }
    }
}

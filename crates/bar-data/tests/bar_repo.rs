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

#[test]
fn spot_finder_finds_painted_spots() {
    use std::rc::Rc;

    use bar_data::import::find_metal_spots;
    use bar_data::map::Xz;

    let Some(root) = bar_root() else {
        eprintln!("BAR_ROOT is not set, skipping");
        return;
    };
    // 1024 x 1024 elmo map, 64 x 64 metal squares, two 3 x 3 blobs.
    let width = 64;
    let mut metal = vec![0.0; width * width];
    for (cx, cz) in [(10, 10), (40, 50)] {
        for dz in 0..3 {
            for dx in 0..3 {
                metal[(cz + dz) * width + cx + dx] = 0.2;
            }
        }
    }
    let size = Xz { x: 1024.0, z: 1024.0 };
    let mut spots = find_metal_spots(&root, size, width, Rc::new(metal), 90.0).unwrap();
    spots.sort_by(|a, b| a.x.total_cmp(&b.x));
    assert_eq!(spots.len(), 2);
    // Centre of squares 10..12 is 16 * 11 = 176 elmos.
    assert!((spots[0].x - 176.0).abs() <= 8.0 && (spots[0].z - 176.0).abs() <= 8.0, "{:?}", spots[0]);
    // Worth is 9 * 0.2 = 1.8, shown as 0.0018 metal per second.
    assert!((spots[0].metal - 0.002).abs() < 1e-9, "{:?}", spots[0]);
}

#[test]
fn imports_extracted_map_directory() {
    let Some(root) = bar_root() else {
        eprintln!("BAR_ROOT is not set, skipping");
        return;
    };
    let dir = std::env::temp_dir().join(format!("bar-data-map-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("maps")).unwrap();
    std::fs::write(
        dir.join("mapinfo.lua"),
        r#"
        local mapinfo = {
            name = "Unit Test Map", version = "v1", maxMetal = 2.55, extractorradius = 90,
            tidalstrength = 15,
            atmosphere = { minWind = 3, maxWind = 18 },
            teams = { [0] = { startPos = { x = 100, z = 120 } }, [1] = { startPos = { x = 900, z = 880 } } },
        }
        if Spring.GetMapOptions then local _ = Spring.GetMapOptions() end
        local function lowerkeys(t)
            local n = {}
            for k, v in pairs(t) do
                if type(v) == "table" then v = lowerkeys(v) end
                n[type(k) == "string" and k:lower() or k] = v
            end
            return n
        end
        return lowerkeys(mapinfo)
        "#,
    )
    .unwrap();
    // 128 x 128 map squares = 1024 x 1024 elmos.
    let (sx, sz) = (128i32, 128i32);
    let mut smf = vec![0u8; 80];
    smf[..16].copy_from_slice(b"spring map file\0");
    smf[24..28].copy_from_slice(&sx.to_le_bytes());
    smf[28..32].copy_from_slice(&sz.to_le_bytes());
    smf[68..72].copy_from_slice(&80i32.to_le_bytes());
    let w = (sx / 2) as usize;
    let mut metal = vec![0u8; w * w];
    for dz in 0..3 {
        for dx in 0..3 {
            metal[(20 + dz) * w + 30 + dx] = 255;
        }
    }
    smf.extend_from_slice(&metal);
    std::fs::write(dir.join("maps/unit_test.smf"), smf).unwrap();

    let spec = bar_data::import::import_map(&dir, &root).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(spec.name, "Unit Test Map v1");
    assert_eq!(spec.size.x, 1024.0);
    assert_eq!((spec.wind.min, spec.wind.max, spec.tidal), (3.0, 18.0, 15.0));
    assert_eq!(spec.start_positions.len(), 2);
    assert_eq!(spec.start("P2").unwrap().x, 900.0);
    assert_eq!(spec.metal_spots.len(), 1);
    // 9 squares at 2.55 metal each: worth 22.95, value 0.023.
    assert!((spec.metal_spots[0].metal - 0.023).abs() < 1e-9, "{:?}", spec.metal_spots[0]);
}

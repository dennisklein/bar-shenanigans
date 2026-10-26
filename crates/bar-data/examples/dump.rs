//! Print unit defs as JSON: `cargo run -p bar-data --example dump -- <bar_root> [unit...]`
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let root = args.next().expect("usage: dump <bar_root> [unit...]");
    let defs = bar_data::UnitDefs::load(std::path::Path::new(&root))?;
    eprintln!("{} unit defs, commit {:?}, {} warnings", defs.len(), defs.commit, defs.warnings.len());
    for w in defs.warnings.iter().take(20) {
        eprintln!("warning: {w}");
    }
    for name in args {
        println!("{}", serde_json::to_string_pretty(&defs.get(&name))?);
    }
    Ok(())
}

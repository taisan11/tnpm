//! TNPM_BENCH_PACKUMENT=/path/to/typescript.json cargo test --release --bin tnpm
//! registry::metadata_benchmark::metadata -- --ignored --nocapture
use super::*;
use std::{hint::black_box, time::Instant};

#[test]
#[ignore = "requires an explicitly supplied local packument; no network requests"]
fn metadata() {
    let text = std::fs::read_to_string(
        std::env::var("TNPM_BENCH_PACKUMENT").expect("set TNPM_BENCH_PACKUMENT"),
    )
    .unwrap();
    let selector = std::env::var("TNPM_BENCH_SELECTOR").unwrap_or_else(|_| "5.9.3".into());
    let policy = tests::policy();
    let mut samples = Vec::new();
    for i in 0..33 {
        let start = Instant::now();
        let value = Value::parse(black_box(&text)).unwrap();
        let parse = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let document = Document::open(
            cache::encode("benchmark", black_box(&value), &policy).unwrap(),
            "benchmark",
        )
        .unwrap()
        .unwrap();
        let indexed = resolve_binary(&document, Query::versions(&selector, true)).unwrap();
        let index = start.elapsed().as_secs_f64() * 1000.;
        let start = Instant::now();
        let (selected, raw) =
            resolve_raw(black_box(&text), &selector, &mut Profile::new("benchmark")).unwrap();
        let selective = start.elapsed().as_secs_f64() * 1000.;
        assert_eq!(selected.versions, indexed.versions);
        let start = Instant::now();
        let reconstructed = Value::from_raw(raw.value()).unwrap();
        let tree = start.elapsed().as_secs_f64() * 1000.;
        assert_eq!(reconstructed, value);
        if i >= 3 {
            samples.push([parse, index, selective, tree]);
        }
    }
    println!("metadata bytes={} samples={}", text.len(), samples.len());
    for (i, name) in [
        "owned_parse_ms",
        "index_open_resolve_ms",
        "borrowed_parse_select_ms",
        "background_owned_tree_ms",
    ]
    .iter()
    .enumerate()
    {
        let mut values: Vec<_> = samples.iter().map(|sample| sample[i]).collect();
        values.sort_by(f64::total_cmp);
        println!(
            "{name}: mean={:.3} median={:.3} min={:.3} max={:.3}",
            values.iter().sum::<f64>() / values.len() as f64,
            values[values.len() / 2],
            values[0],
            values[values.len() - 1]
        );
    }
}

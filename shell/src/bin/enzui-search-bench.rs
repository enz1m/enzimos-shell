//! Benchmarks search against generated data and emits timing JSON.
#![allow(
    clippy::print_stdout,
    reason = "This command writes its result or probe protocol to stdout; diagnostics use stderr."
)]
use enzui_shell::i18n;
use enzui_shell::search::{
    self, Category, Item, Target, calculator, files, preferences::Preferences,
};
use serde_json::{Value, json};
use std::{error::Error, fs, hint::black_box, path::Path, sync::atomic::AtomicBool, time::Instant};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

#[derive(Debug, PartialEq)]
struct Options {
    items: usize,
    files: usize,
    samples: usize,
}
impl Options {
    fn parse(args: &[String]) -> Result<Self> {
        let mut value = Self {
            items: 2000,
            files: 20_000,
            samples: 100,
        };
        if args.first().map(String::as_str) != Some("--synthetic") {
            return Err("usage: enzui-search-bench --synthetic [--items 1..2048] [--files 0..20000] [--samples 10..1000]".into());
        }
        for pair in args[1..].chunks(2) {
            if pair.len() != 2 {
                return Err("benchmark option needs a value".into());
            }
            let number = pair[1].parse::<usize>()?;
            match pair[0].as_str() {
                "--items" if (1..=2048).contains(&number) => value.items = number,
                "--files" if number <= files::MAX_FILES => value.files = number,
                "--samples" if (10..=1000).contains(&number) => value.samples = number,
                _ => return Err("unknown or out-of-bounds benchmark option".into()),
            }
        }
        Ok(value)
    }
}

fn stats(mut values: Vec<u64>) -> Value {
    values.sort_unstable();
    let percentile =
        |percent: usize| values[(values.len() * percent).div_ceil(100).saturating_sub(1)];
    json!({"samples": values.len(), "min_ns": values[0], "p50_ns": percentile(50),
        "p95_ns": percentile(95), "p99_ns": percentile(99), "max_ns": values[values.len()-1],
        "percentile_method": "nearest rank"})
}

fn measure(mut work: impl FnMut(), samples: usize) -> Value {
    for _ in 0..10 {
        work();
    }
    let mut values = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        work();
        values.push(u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX));
    }
    stats(values)
}

fn rank_cases(
    items: &[Item],
    category: Category,
    samples: usize,
    preferences: &Preferences,
) -> Value {
    let mut cases = Vec::new();
    for query in ["", "fixture", "report 019", "fxtr", "no-matching-token"] {
        let expected = search::rank(items.iter(), query, Some(category), preferences);
        assert!(
            expected.len() <= search::MAX_RESULTS,
            "the ranking fixture must fit within the result limit"
        );
        let identities: Vec<_> = expected.iter().map(|item| &item.id).collect();
        let result = measure(
            || {
                let ranked = search::rank(
                    black_box(items.iter()),
                    black_box(query),
                    Some(category),
                    preferences,
                );
                assert_eq!(
                    ranked.iter().map(|item| &item.id).collect::<Vec<_>>(),
                    identities,
                    "repeated ranking must preserve result identities and order"
                );
                black_box(ranked);
            },
            samples,
        );
        cases.push(json!({"query": query, "returned": expected.len(), "timing_includes_identity_assertion": true, "wall_time": result}));
    }
    json!(cases)
}

fn file_benchmark(
    root: &Path,
    count: usize,
    samples: usize,
    preferences: &Preferences,
) -> Result<Value> {
    if count == 0 {
        return Ok(Value::Null);
    }
    let fixture = tempfile::Builder::new()
        .prefix("search-bench-")
        .tempdir_in(root)?;
    let create = Instant::now();
    for index in 0..count {
        fs::File::create(
            fixture
                .path()
                .join(format!("Fixture report {index:05}.txt")),
        )?;
    }
    let create_ns = create.elapsed().as_nanos();
    let roots = vec![fixture.path().to_string_lossy().into_owned()];
    let cancel = AtomicBool::new(false);
    let mut builds = Vec::new();
    let mut index = files::Index::default();
    for _ in 0..3 {
        let start = Instant::now();
        index = files::build(&roots, &[], &cancel);
        builds.push(u64::try_from(start.elapsed().as_nanos())?);
        if index.errors != 0
            || index.items.is_empty()
            || index.scanned > files::MAX_FILES
            || index.items.len() > count
        {
            return Err("synthetic index did not meet its bounds".into());
        }
    }
    let ranking = rank_cases(&index.items, Category::Files, samples, preferences);
    let canceled = AtomicBool::new(true);
    let cancellation = measure(
        || {
            assert!(
                files::build(&roots, &[], &canceled).items.is_empty(),
                "canceled indexing must publish no results"
            );
        },
        samples,
    );
    let result = json!({"created_empty_files": count, "fixture_creation_ns": create_ns, "scanned_entries": index.scanned,
        "indexed_items": index.items.len(), "truncated": index.truncated, "errors": index.errors,
        "index_build_wall_time": stats(builds), "cache_scope": "newly created fixture; warm filesystem metadata, not cold disk",
        "file_rank": ranking, "already_canceled_build": cancellation});
    fixture.close()?;
    Ok(result)
}

fn main() -> Result<()> {
    i18n::initialize();
    let options = Options::parse(&std::env::args().skip(1).collect::<Vec<_>>())?;
    let mut preferences = Preferences::default();
    preferences.categories.push(Category::Files);
    preferences.favorites.push("app:fixture-0003".into());
    preferences.usage.insert("app:fixture-0007".into(), 40);
    let start = Instant::now();
    let items: Vec<_> = (0..options.items)
        .map(|index| {
            Item::new(
                format!("app:fixture-{index:04}"),
                Category::Apps,
                format!("Fixture report {index:05}"),
                "Synthetic editor application",
                "Open",
                "app-window",
                Some(Target::Copy("never activated".into())),
            )
        })
        .collect();
    let preparation_ns = start.elapsed().as_nanos();
    let app_rank = rank_cases(&items, Category::Apps, options.samples, &preferences);
    let mut calculations = Vec::new();
    for (query, expected) in [
        ("2+3*4", "14"),
        ("(64/2)^2", "1024"),
        ("32 F to C", "0 °C"),
        ("1 GiB to MiB", "1024 MiB"),
    ] {
        let timing = measure(
            || {
                let value = calculator::evaluate(black_box(query))
                    .expect("the benchmark corpus contains valid calculations")
                    .expect("the benchmark corpus contains valid calculations");
                assert_eq!(
                    value.value, expected,
                    "the calculator must match the benchmark oracle"
                );
                black_box(value);
            },
            options.samples,
        );
        calculations.push(json!({"query": query, "expected": expected, "wall_time": timing}));
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("missing benchmark repository")?
        .join(".build/quick-search");
    fs::create_dir_all(&root)?;
    let index = file_benchmark(&root, options.files, options.samples, &preferences)?;
    let peak_kib = fs::read_to_string("/proc/self/status")?
        .lines()
        .find(|line| line.starts_with("VmHWM:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u64>().ok());
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"synthetic_only": true, "debug_assertions": cfg!(debug_assertions),
        "architecture": std::env::consts::ARCH, "app_items": options.items, "corpus_preparation_ns": preparation_ns,
        "app_rank": app_rank, "calculator": calculations, "file_index": index, "process_peak_rss_kib": peak_kib,
        "fixture_removed": true, "native_ui_latency_claim": false, "physical_latency_claim": false})
        )?
    );
    Ok(())
}

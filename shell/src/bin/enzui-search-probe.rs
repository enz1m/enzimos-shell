//! Search-provider diagnostic CLI; see docs/search-providers.md for declarations and protocol.
#![allow(
    clippy::print_stdout,
    reason = "This command writes its result or probe protocol to stdout; diagnostics use stderr."
)]

use enzui_shell::i18n;
use enzui_shell::search::providers;
use std::time::Instant;

#[tokio::main(flavor = "current_thread")]
async fn main() -> std::process::ExitCode {
    i18n::initialize();
    let mut args = std::env::args().skip(1);
    let Some(file) = args.next() else {
        eprintln!("usage: enzui-search-probe <declaration.ini> [query…]");
        return std::process::ExitCode::from(2);
    };
    let Some(declaration) = providers::read_declaration(std::path::Path::new(&file)) else {
        eprintln!(
            "{file}: not a valid enzUI (Version=1) or GNOME (Version=2) search provider declaration"
        );
        return std::process::ExitCode::FAILURE;
    };
    println!(
        "provider  {} ({:?})",
        declaration.desktop_id, declaration.protocol
    );
    println!(
        "service   {} {}",
        declaration.bus_name, declaration.object_path
    );
    let dirs = providers::data_dirs();
    match providers::app_name(&declaration.desktop_id, &dirs) {
        Some(name) => println!("app       {name}"),
        None => println!(
            "app       none shown: Search will not ask this provider (missing, hidden or not allowed)"
        ),
    }
    let Some(bus) = providers::session().await else {
        eprintln!("no session bus");
        return std::process::ExitCode::FAILURE;
    };
    if !providers::reachable(&bus, &declaration).await {
        eprintln!(
            "{} is neither running nor activatable on this session bus",
            declaration.bus_name
        );
        return std::process::ExitCode::FAILURE;
    }
    let mut queries: Vec<String> = args.collect();
    if queries.is_empty() {
        queries = vec!["a".into(), "test".into()];
    }
    let mut failed = false;
    for query in queries {
        if query.chars().count() < declaration.min_query {
            println!(
                "\n“{query}”: shorter than MinQueryLength {}; Search would not ask",
                declaration.min_query
            );
            continue;
        }
        let started = Instant::now();
        match providers::search(&bus, &declaration, &query).await {
            Ok(found) => {
                println!(
                    "\n“{query}”: {} result(s) in {} ms",
                    found.len(),
                    started.elapsed().as_millis()
                );
                for row in found.iter().take(providers::SHOWN) {
                    println!(
                        "  {} — {}  [id {}{}]",
                        row.title,
                        row.subtitle,
                        row.id,
                        row.icon
                            .as_ref()
                            .map(|i| format!(", icon {i}"))
                            .unwrap_or_default()
                    );
                }
                if found.len() > providers::SHOWN {
                    println!(
                        "  … {} more not shown (Search shows {})",
                        found.len() - providers::SHOWN,
                        providers::SHOWN
                    );
                }
            }
            Err(providers::Silence::NotRunning) => {
                failed = true;
                println!(
                    "\n“{query}”: the app is not running; Search only asks running apps (start it, then probe again)"
                );
            }
            Err(providers::Silence::Failed(error)) => {
                failed = true;
                println!(
                    "\n“{query}”: failed after {} ms: {error}",
                    started.elapsed().as_millis()
                );
            }
        }
    }
    if failed {
        std::process::ExitCode::FAILURE
    } else {
        std::process::ExitCode::SUCCESS
    }
}

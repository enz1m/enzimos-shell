//! The enzUI location agent: tells GeoClue how precisely this user's apps
//! may be located (nothing at all until Location is turned on in Settings).
//! See `enzui_shell::location`.

fn main() {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("enzui-location-agent: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = runtime.block_on(enzui_shell::location::run_agent()) {
        eprintln!("enzui-location-agent: {error}");
        std::process::exit(1);
    }
}

//! `enzui-agent-mock`: the stand-in agent service the assistant panel talks
//! to until the real agent harness is connected. See `enzui_shell::agent_mock`.

use enzui_shell::i18n;
fn main() {
    i18n::initialize();
    let Some(path) = std::env::var_os("ENZUI_AGENT_SOCKET")
        .map(std::path::PathBuf::from)
        .or_else(enzui_shell::agent_mock::socket_path)
    else {
        eprintln!("enzui-agent-mock: no runtime folder");
        std::process::exit(2);
    };
    // Started by the shell: leave when the shell does (its end of stdin closes).
    if std::env::var("ENZUI_AGENT_MOCK_PARENT").as_deref() == Ok("stdin") {
        std::thread::spawn(|| {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
            std::process::exit(0);
        });
    }
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        std::process::exit(2)
    };
    if let Err(error) = runtime.block_on(enzui_shell::agent_mock::serve(&path)) {
        eprintln!("enzui-agent-mock: {error}");
        std::process::exit(1);
    }
}

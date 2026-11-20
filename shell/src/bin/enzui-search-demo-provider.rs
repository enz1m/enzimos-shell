//! Scripted Search provider; --install DIR writes its declaration, service and desktop entry.
//! Open requests are recorded in the private runtime log.
#![allow(
    clippy::print_stdout,
    reason = "This command writes its result or probe protocol to stdout; diagnostics use stderr."
)]

use enzui_shell::i18n;
use std::collections::HashMap;
use std::io::Write;
use zbus::zvariant::{OwnedValue, Value};

const NAME: &str = "dev.enzui.SearchDemo";
const PATH: &str = "/dev/enzui/SearchDemo/SearchProvider";
const MESSAGES: [(&str, &str, &str); 6] = [
    (
        "m1",
        "Invoice for September",
        "Ana Kovács · accounts@example.org",
    ),
    ("m2", "Team lunch on Friday", "Ben Murphy · Friday 12:30"),
    (
        "m3",
        "Your invoice is ready",
        "Utility Co. · Due 20 October",
    ),
    ("m4", "Photos from the trip", "Chloe · 14 attachments"),
    (
        "m5",
        "Re: invoice question",
        "Ana Kovács · Thanks, that helps",
    ),
    ("m6", "Release notes draft", "Dev team · Please review"),
];

fn log(line: String) {
    let path = std::path::PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").unwrap_or_default())
        .join("enzui-search-demo.log");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "{line}");
    }
}

struct Demo;

#[zbus::interface(name = "dev.enzui.SearchProvider1")]
impl Demo {
    fn search(
        &self,
        query: String,
        options: HashMap<String, OwnedValue>,
    ) -> Vec<HashMap<String, OwnedValue>> {
        let limit = options
            .get("limit")
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(5) as usize;
        let wanted = query.to_lowercase();
        MESSAGES
            .iter()
            .filter(|(_, title, from)| {
                wanted.split_whitespace().all(|word| {
                    title.to_lowercase().contains(word) || from.to_lowercase().contains(word)
                })
            })
            .take(limit)
            .map(|(id, title, from)| {
                [
                    ("id", Value::from(*id)),
                    ("title", Value::from(*title)),
                    ("subtitle", Value::from(*from)),
                    ("icon", Value::from("mail-unread")),
                ]
                .into_iter()
                .map(|(k, v)| {
                    (
                        k.to_owned(),
                        v.try_to_owned()
                            .expect("owned static strings have no file descriptors to duplicate"),
                    )
                })
                .collect()
            })
            .collect()
    }
    fn activate(&self, id: String, query: String, _options: HashMap<String, OwnedValue>) {
        log(format!("activate {id} {query}"));
    }
    fn launch_search(&self, query: String, _options: HashMap<String, OwnedValue>) {
        log(format!("launch {query}"));
    }
    #[zbus(property)]
    fn version(&self) -> u32 {
        1
    }
}

fn install(dir: &std::path::Path) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    for sub in ["enzui/search-providers", "dbus-1/services", "applications"] {
        std::fs::create_dir_all(dir.join(sub))?;
    }
    std::fs::write(
        dir.join("enzui/search-providers/dev.enzui.SearchDemo.ini"),
        format!(
            "[enzUI Search Provider]\nDesktopId={NAME}.desktop\nBusName={NAME}\nObjectPath={PATH}\nVersion=1\n"
        ),
    )?;
    std::fs::write(
        dir.join(format!("dbus-1/services/{NAME}.service")),
        format!("[D-BUS Service]\nName={NAME}\nExec={}\n", exe.display()),
    )?;
    std::fs::write(
        dir.join(format!("applications/{NAME}.desktop")),
        format!(
            "[Desktop Entry]\nType=Application\nName=Demo Mail\nComment=Sample search provider\nIcon=mail-unread\nExec={}\n",
            exe.display()
        ),
    )?;
    println!("installed the demo provider under {}", dir.display());
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    i18n::initialize();
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--install") {
        let dir = args
            .get(2)
            .ok_or("usage: enzui-search-demo-provider --install DATA_DIR")?;
        install(std::path::Path::new(dir))?;
        return Ok(());
    }
    let _connection = zbus::connection::Builder::session()?
        .name(NAME)?
        .serve_at(PATH, Demo)?
        .build()
        .await?;
    log("started".into());
    // Serves until it is stopped; D-Bus activation starts it when asked.
    std::future::pending::<()>().await;
    Ok(())
}

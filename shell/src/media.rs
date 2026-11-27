//! Event-driven MPRIS player controls; album art accepts local files only.

use enzui_shell::i18n::tr;
use std::collections::HashMap;
use std::path::PathBuf;
use zbus::zvariant::OwnedValue;

const PREFIX: &str = "org.mpris.MediaPlayer2.";
const PATH: &str = "/org/mpris/MediaPlayer2";
const MAX_PLAYERS: usize = 16;
const MAX_TEXT: usize = 256;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    Playing,
    Paused,
    #[default]
    Stopped,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Player {
    /// The player's bus name.
    pub bus: String,
    pub identity: String,
    pub status: Status,
    pub title: String,
    pub artist: String,
    pub art: Option<PathBuf>,
    pub can_play_pause: bool,
    pub can_next: bool,
    pub can_previous: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    PlayPause,
    Next,
    Previous,
    Stop,
}

fn bounded(text: &str) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    let mut end = clean.len().min(MAX_TEXT);
    while !clean.is_char_boundary(end) {
        end -= 1;
    }
    clean.get(..end).unwrap_or_default().to_owned()
}

/// A local album-art file from an `mpris:artUrl`; anything else is dropped.
#[must_use]
pub fn local_art(url: &str) -> Option<PathBuf> {
    let path = url.strip_prefix("file://")?;
    let mut bytes = Vec::new();
    let raw = path.as_bytes();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            let hex = std::str::from_utf8(&raw[i + 1..i + 3]).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    let path = PathBuf::from(String::from_utf8(bytes).ok()?);
    (path.is_absolute()
        && std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= 16 << 20))
    .then_some(path)
}

fn text(metadata: &HashMap<String, OwnedValue>, key: &str) -> String {
    metadata
        .get(key)
        .and_then(|value| String::try_from(value.clone()).ok())
        .map(|t| bounded(&t))
        .unwrap_or_default()
}

fn artists(metadata: &HashMap<String, OwnedValue>) -> String {
    metadata
        .get("xesam:artist")
        .and_then(|value| Vec::<String>::try_from(value.clone()).ok())
        .map(|list| bounded(&list.join(", ")))
        .unwrap_or_default()
}

async fn read(connection: &zbus::Connection, bus: &str) -> Option<Player> {
    let root = zbus::Proxy::new(connection, bus.to_owned(), PATH, "org.mpris.MediaPlayer2")
        .await
        .ok()?;
    let player = zbus::Proxy::new(
        connection,
        bus.to_owned(),
        PATH,
        "org.mpris.MediaPlayer2.Player",
    )
    .await
    .ok()?;
    let identity: String = root
        .get_property("Identity")
        .await
        .unwrap_or_else(|_| bus.trim_start_matches(PREFIX).to_owned());
    let status: String = player
        .get_property("PlaybackStatus")
        .await
        .unwrap_or_default();
    let metadata: HashMap<String, OwnedValue> =
        player.get_property("Metadata").await.unwrap_or_default();
    let flag = |value: Result<bool, zbus::Error>| value.unwrap_or(false);
    let can_play = flag(player.get_property("CanPlay").await);
    let can_pause = flag(player.get_property("CanPause").await);
    Some(Player {
        bus: bus.to_owned(),
        identity: bounded(&identity),
        status: match status.as_str() {
            "Playing" => Status::Playing,
            "Paused" => Status::Paused,
            _ => Status::Stopped,
        },
        title: text(&metadata, "xesam:title"),
        artist: artists(&metadata),
        art: metadata
            .get("mpris:artUrl")
            .and_then(|value| String::try_from(value.clone()).ok())
            .and_then(|url| local_art(&url)),
        can_play_pause: can_play || can_pause,
        can_next: flag(player.get_property("CanGoNext").await),
        can_previous: flag(player.get_property("CanGoPrevious").await),
    })
}

/// Every player on the bus, in bus-name order.
pub async fn players(connection: &zbus::Connection) -> Vec<Player> {
    let Ok(dbus) = zbus::fdo::DBusProxy::new(connection).await else {
        return Vec::new();
    };
    let mut names: Vec<String> = dbus
        .list_names()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|name| name.to_string())
        .filter(|name| name.starts_with(PREFIX))
        .collect();
    names.sort();
    names.truncate(MAX_PLAYERS);
    let mut found = Vec::new();
    for name in names {
        if let Some(player) = read(connection, &name).await {
            found.push(player);
        }
    }
    found
}

/// The player media keys and the control center act on: one that plays
/// (the last one used if it does), else the last one used, else the first.
#[must_use]
pub fn active<'a>(players: &'a [Player], last: Option<&str>) -> Option<&'a Player> {
    let last_player = last.and_then(|bus| players.iter().find(|p| p.bus == bus));
    last_player
        .filter(|p| p.status == Status::Playing)
        .or_else(|| players.iter().find(|p| p.status == Status::Playing))
        .or(last_player)
        .or_else(|| players.first())
}

pub async fn control(bus: String, control: Control) -> Result<(), String> {
    if !bus.starts_with(PREFIX) {
        return Err(tr("shell-not-a-media-player").into());
    }
    let connection = zbus::Connection::session()
        .await
        .map_err(|_| tr("shell-no-session-bus"))?;
    let player = zbus::Proxy::new(&connection, bus, PATH, "org.mpris.MediaPlayer2.Player")
        .await
        .map_err(|_| tr("shell-the-player-left"))?;
    let method = match control {
        Control::PlayPause => "PlayPause",
        Control::Next => "Next",
        Control::Previous => "Previous",
        Control::Stop => "Stop",
    };
    player
        .call_method(method, &())
        .await
        .map(drop)
        .map_err(|_| tr("shell-the-player-did-not-respond").into())
}

/// The player list, re-read whenever a player appears, leaves or changes.
pub fn subscription() -> iced::Subscription<Vec<Player>> {
    iced::Subscription::run(stream)
}

fn stream() -> impl iced::futures::Stream<Item = Vec<Player>> {
    use iced::futures::{SinkExt, StreamExt};
    iced::stream::channel(4, async |mut output| {
        loop {
            if let Ok(connection) = zbus::Connection::session().await {
                let changes = zbus::MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .interface("org.freedesktop.DBus.Properties")
                    .and_then(|rule| rule.member("PropertiesChanged"))
                    .and_then(|rule| rule.path(PATH))
                    .map(zbus::match_rule::Builder::build);
                let owners = zbus::MatchRule::builder()
                    .msg_type(zbus::message::Type::Signal)
                    .sender("org.freedesktop.DBus")
                    .and_then(|rule| rule.member("NameOwnerChanged"))
                    .and_then(|rule| rule.arg0ns("org.mpris.MediaPlayer2"))
                    .map(zbus::match_rule::Builder::build);
                if let (Ok(changes), Ok(owners)) = (changes, owners)
                    && let (Ok(changes), Ok(owners)) = (
                        zbus::MessageStream::for_match_rule(changes, &connection, Some(32)).await,
                        zbus::MessageStream::for_match_rule(owners, &connection, Some(32)).await,
                    )
                {
                    let mut events = iced::futures::stream::select(changes, owners);
                    loop {
                        if output.send(players(&connection).await).await.is_err() {
                            return;
                        }
                        // Coalesce a burst (a track change sends several).
                        if events.next().await.is_none() {
                            break;
                        }
                        while let Ok(Some(_)) = tokio::time::timeout(
                            std::time::Duration::from_millis(100),
                            events.next(),
                        )
                        .await
                        {}
                    }
                }
            }
            let _ = output.send(Vec::new()).await;
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    })
}

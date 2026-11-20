use super::manifest::{Manifest, Permission};
use crate::bar_layout::{Item, Section};
use enzui_shell::i18n::tr;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

const MAX_NODES: usize = 160;
const MAX_DEPTH: usize = 6;
#[derive(Clone)]
pub struct Snapshot {
    pub revision: u64,
    pub elements: Vec<Element>,
}
#[derive(Clone)]
pub struct Element {
    pub id: String,
    pub icon: String,
    pub label: String,
    pub tooltip: String,
    pub panel: Option<Panel>,
}
#[derive(Clone)]
pub struct Panel {
    pub title: String,
    pub width: u16,
    pub height: u16,
    pub nodes: Vec<Node>,
    pub footer: Option<Vec<Node>>,
    pub entry_list: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Muted,
    Title,
    Danger,
}
#[derive(Clone)]
pub struct Node {
    pub key: String,
    pub kind: NodeKind,
}
#[derive(Clone)]
pub enum NodeKind {
    Text {
        value: String,
        tone: Tone,
    },
    Action {
        id: String,
        label: String,
        detail: Option<String>,
        icon: Option<String>,
        thumbnail: Option<String>,
        enabled: bool,
    },
    Toggle {
        id: String,
        label: String,
        value: bool,
    },
    IconAction {
        id: String,
        label: String,
        icon: String,
        enabled: bool,
    },
    IconToggle {
        id: String,
        label: String,
        icon: String,
        value: bool,
    },
    Entry {
        label: String,
        thumbnail: Option<String>,
        copy: String,
        delete: String,
        pin: String,
        pinned: bool,
    },
    Image {
        blob: String,
        alt: String,
        width: u16,
        height: u16,
    },
    Row(Vec<Node>),
    Column(Vec<Node>),
    Separator,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellView {
    Applications,
    Desktops,
    Controls,
    Settings,
}
#[derive(Clone)]
pub enum Request {
    ApplicationDock {
        element: Option<String>,
    },
    ClipboardSubscribe(bool),
    NotificationSubscribe(bool),
    ClipboardRestore {
        blob: String,
        gesture: String,
    },
    ReleaseBlobs(Vec<String>),
    Theme(BTreeMap<String, [u8; 3]>),
    Layout {
        element: String,
        section: Section,
        before: Option<String>,
        enabled: Option<bool>,
    },
    Open {
        view: ShellView,
        gesture: String,
    },
}
pub enum Output {
    Ready,
    Pong(u64),
    Snapshot(Snapshot),
    Request { id: u64, request: Request },
}
impl Snapshot {
    pub fn permits_action(&self, element: &str, id: &str, value: Option<bool>) -> bool {
        fn contains(nodes: &[Node], id: &str, value: Option<bool>) -> bool {
            nodes.iter().any(|n| match &n.kind {
                NodeKind::Action {
                    id: action,
                    enabled,
                    ..
                } => action == id && *enabled && value.is_none(),
                NodeKind::IconAction {
                    id: action,
                    enabled,
                    ..
                } => action == id && *enabled && value.is_none(),
                NodeKind::Toggle { id: action, .. } | NodeKind::IconToggle { id: action, .. } => {
                    action == id && value.is_some()
                }
                NodeKind::Entry {
                    copy,
                    delete,
                    pin,
                    pinned,
                    ..
                } => {
                    ((id == copy || id == delete) && value.is_none())
                        || (id == pin && value == Some(!pinned))
                }
                NodeKind::Row(nodes) | NodeKind::Column(nodes) => contains(nodes, id, value),
                _ => false,
            })
        }
        self.elements
            .iter()
            .find(|e| e.id == element)
            .and_then(|e| e.panel.as_ref())
            .is_some_and(|p| {
                contains(&p.nodes, id, value)
                    || p.footer
                        .as_ref()
                        .is_some_and(|nodes| contains(nodes, id, value))
            })
    }
}
fn fields(value: &Value, keys: &[&str]) -> Result<(), String> {
    let object = value
        .as_object()
        .ok_or_else(|| tr("shell-expected-a-plugin-message-object"))?;
    if object.len() != keys.len() || object.keys().any(|k| !keys.contains(&k.as_str())) {
        return Err(tr("shell-unknown-or-missing-plugin-message-field").into());
    }
    Ok(())
}
fn array(value: &Value, max: usize) -> Result<&[Value], String> {
    value
        .as_array()
        .filter(|a| a.len() <= max)
        .map(Vec::as_slice)
        .ok_or_else(|| tr("shell-excessive-or-invalid-plugin-collection").into())
}
fn text(value: &Value, max: usize, multiline: bool) -> Result<String, String> {
    let s = value
        .as_str()
        .ok_or_else(|| tr("shell-expected-plugin-text"))?;
    if s.chars().count() > max
        || s.chars().any(|c| {
            (c.is_control() && !(multiline && matches!(c, '\n' | '\t')))
                || matches!(c as u32,0x200b|0x200e..=0x200f|0x202a..=0x202e|0x2060..=0x206f|0xfeff)
        })
    {
        return Err(tr("shell-invalid-or-excessive-plugin-text").into());
    }
    Ok(s.to_owned())
}
fn action(value: &Value) -> Result<String, String> {
    let s = value
        .as_str()
        .filter(|s| {
            !s.is_empty()
                && s.len() <= 96
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._:/-".contains(&b))
        })
        .ok_or_else(|| tr("shell-invalid-plugin-action-identity"))?;
    Ok(s.to_owned())
}
fn opaque(value: &Value) -> Result<String, String> {
    let s = value
        .as_str()
        .filter(|s| {
            s.len() == 32
                && s.bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
        .ok_or_else(|| tr("shell-invalid-plugin-capability-reference"))?;
    Ok(s.to_owned())
}
fn icon(value: &Value) -> Result<String, String> {
    let s = value
        .as_str()
        .filter(|s| s.len() <= 48 && enzui_icons::bundled_icon_svg(s).is_some())
        .ok_or("Unknown plugin icon")?;
    Ok(s.to_owned())
}
fn optional<T>(
    value: &Value,
    parse: impl FnOnce(&Value) -> Result<T, String>,
) -> Result<Option<T>, String> {
    if value.is_null() {
        Ok(None)
    } else {
        parse(value).map(Some)
    }
}
fn boolean(value: &Value) -> Result<bool, String> {
    value
        .as_bool()
        .ok_or_else(|| tr("shell-expected-plugin-boolean").into())
}
fn dimension(value: &Value, min: u16, max: u16) -> Result<u16, String> {
    value
        .as_u64()
        .filter(|n| (*n >= u64::from(min)) && (*n <= u64::from(max)))
        .map(|n| n as u16)
        .ok_or_else(|| tr("shell-plugin-dimension-is-out-of-bounds").into())
}
fn nodes(
    value: &Value,
    depth: usize,
    left: &mut usize,
    keys: &mut BTreeSet<String>,
) -> Result<Vec<Node>, String> {
    nodes_with_limit(value, depth, left, keys, 16)
}
fn nodes_with_limit(
    value: &Value,
    depth: usize,
    left: &mut usize,
    keys: &mut BTreeSet<String>,
    limit: usize,
) -> Result<Vec<Node>, String> {
    if depth > MAX_DEPTH {
        return Err(tr("shell-plugin-ui-nesting-is-too-deep").into());
    }
    array(value, limit)?
        .iter()
        .map(|value| {
            *left = left
                .checked_sub(1)
                .ok_or_else(|| tr("shell-too-many-plugin-ui-nodes"))?;
            let key = action(&value["key"])?;
            if !keys.insert(key.clone()) {
                return Err(tr("shell-duplicate-plugin-ui-key").into());
            }
            let kind = match value["type"].as_str() {
                Some("text") => {
                    fields(value, &["type", "key", "text", "tone"])?;
                    let tone = match value["tone"].as_str() {
                        Some("normal") => Tone::Normal,
                        Some("muted") => Tone::Muted,
                        Some("title") => Tone::Title,
                        Some("danger") => Tone::Danger,
                        _ => {
                            return Err(tr("shell-invalid-plugin-text-tone").into());
                        }
                    };
                    NodeKind::Text {
                        value: text(&value["text"], 512, true)?,
                        tone,
                    }
                }
                Some("action") => {
                    fields(
                        value,
                        &[
                            "type",
                            "key",
                            "id",
                            "label",
                            "detail",
                            "icon",
                            "thumbnail",
                            "enabled",
                        ],
                    )?;
                    NodeKind::Action {
                        id: action(&value["id"])?,
                        label: text(&value["label"], 120, false)?,
                        detail: optional(&value["detail"], |v| text(v, 240, true))?,
                        icon: optional(&value["icon"], icon)?,
                        thumbnail: optional(&value["thumbnail"], opaque)?,
                        enabled: boolean(&value["enabled"])?,
                    }
                }
                Some("toggle") => {
                    fields(value, &["type", "key", "id", "label", "value"])?;
                    NodeKind::Toggle {
                        id: action(&value["id"])?,
                        label: text(&value["label"], 120, false)?,
                        value: boolean(&value["value"])?,
                    }
                }
                Some("icon_action") => {
                    fields(value, &["type", "key", "id", "label", "icon", "enabled"])?;
                    NodeKind::IconAction {
                        id: action(&value["id"])?,
                        label: text(&value["label"], 120, false)?,
                        icon: icon(&value["icon"])?,
                        enabled: boolean(&value["enabled"])?,
                    }
                }
                Some("icon_toggle") => {
                    fields(value, &["type", "key", "id", "label", "icon", "value"])?;
                    NodeKind::IconToggle {
                        id: action(&value["id"])?,
                        label: text(&value["label"], 120, false)?,
                        icon: icon(&value["icon"])?,
                        value: boolean(&value["value"])?,
                    }
                }
                Some("entry") => {
                    fields(
                        value,
                        &[
                            "type",
                            "key",
                            "label",
                            "thumbnail",
                            "copy",
                            "delete",
                            "pin",
                            "pinned",
                        ],
                    )?;
                    let copy = action(&value["copy"])?;
                    let delete = action(&value["delete"])?;
                    let pin = action(&value["pin"])?;
                    if copy == delete || copy == pin || delete == pin {
                        return Err(tr("shell-ambiguous-entry-actions").into());
                    }
                    NodeKind::Entry {
                        label: text(&value["label"], 120, false)?,
                        thumbnail: optional(&value["thumbnail"], opaque)?,
                        copy,
                        delete,
                        pin,
                        pinned: boolean(&value["pinned"])?,
                    }
                }
                Some("image") => {
                    fields(value, &["type", "key", "blob", "alt", "width", "height"])?;
                    NodeKind::Image {
                        blob: opaque(&value["blob"])?,
                        alt: text(&value["alt"], 120, false)?,
                        width: dimension(&value["width"], 1, 512)?,
                        height: dimension(&value["height"], 1, 512)?,
                    }
                }
                Some("row") => {
                    fields(value, &["type", "key", "children"])?;
                    NodeKind::Row(nodes(&value["children"], depth + 1, left, keys)?)
                }
                Some("column") => {
                    fields(value, &["type", "key", "children"])?;
                    NodeKind::Column(nodes(&value["children"], depth + 1, left, keys)?)
                }
                Some("separator") => {
                    fields(value, &["type", "key"])?;
                    NodeKind::Separator
                }
                _ => return Err(tr("shell-unsupported-plugin-ui-node").into()),
            };
            Ok(Node { key, kind })
        })
        .collect()
}
fn snapshot(
    value: &Value,
    manifest: &Manifest,
    grants: &BTreeSet<Permission>,
) -> Result<Snapshot, String> {
    fields(value, &["type", "revision", "elements"])?;
    let revision = value["revision"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or_else(|| tr("shell-invalid-plugin-ui-revision"))?;
    let mut seen = BTreeSet::new();
    let mut left = MAX_NODES;
    let mut elements = Vec::new();
    let mut keys = BTreeSet::new();
    for element in array(&value["elements"], manifest.elements.len())? {
        if !grants.contains(&Permission::Bar) {
            return Err(tr("shell-plugin-bar-permission-is-not-granted").into());
        }
        fields(element, &["id", "icon", "label", "tooltip", "panel"])?;
        let local = element["id"]
            .as_str()
            .ok_or_else(|| tr("shell-missing-plugin-element-identity"))?;
        if manifest.element_id(local).is_none() || !seen.insert(local) {
            return Err(tr("shell-unknown-or-duplicate-plugin-element-identity").into());
        }
        let panel = optional(&element["panel"], |v| {
            if !grants.contains(&Permission::Panels) {
                return Err(tr("shell-plugin-panel-permission-is-not-granted").into());
            }
            let mut names = vec!["title", "width", "height", "nodes"];
            if v.get("footer").is_some() {
                names.push("footer");
            }
            if v.get("entry_list").is_some() {
                names.push("entry_list");
            }
            fields(v, &names)?;
            let entry_list = v
                .get("entry_list")
                .map(boolean)
                .transpose()?
                .unwrap_or(false);
            let panel_nodes = nodes_with_limit(
                &v["nodes"],
                0,
                &mut left,
                &mut keys,
                if entry_list { 130 } else { 16 },
            )?;
            if entry_list
                && (v.get("footer").is_none()
                    || panel_nodes
                        .iter()
                        .filter(|node| matches!(node.kind, NodeKind::Entry { .. }))
                        .count()
                        > 128
                    || panel_nodes
                        .iter()
                        .filter(|node| matches!(node.kind, NodeKind::Text { .. }))
                        .count()
                        > 2
                    || panel_nodes.iter().any(|node| {
                        !matches!(node.kind, NodeKind::Entry { .. } | NodeKind::Text { .. })
                    }))
            {
                return Err(
                    tr("shell-entry-lists-require-a-footer-and-only-entry-text-nodes").into(),
                );
            }
            Ok(Panel {
                title: text(&v["title"], 80, false)?,
                width: dimension(&v["width"], 240, 640)?,
                height: dimension(&v["height"], 120, 800)?,
                nodes: panel_nodes,
                entry_list,
                footer: v
                    .get("footer")
                    .map(|v| nodes(v, 0, &mut left, &mut keys))
                    .transpose()?,
            })
        })?;
        elements.push(Element {
            id: local.to_owned(),
            icon: icon(&element["icon"])?,
            label: text(&element["label"], 80, false)?,
            tooltip: text(&element["tooltip"], 240, false)?,
            panel,
        });
    }
    Ok(Snapshot { revision, elements })
}
fn require(grants: &BTreeSet<Permission>, permission: Permission) -> Result<(), String> {
    if grants.contains(&permission) {
        Ok(())
    } else {
        Err(tr("shell-plugin-operation-permission-is-not-granted").into())
    }
}
fn request(value: &Value, grants: &BTreeSet<Permission>) -> Result<Request, String> {
    match value["operation"].as_str() {
        Some("applications.dock") => {
            fields(value, &["operation", "element"])?;
            require(grants, Permission::Applications)?;
            require(grants, Permission::Bar)?;
            Ok(Request::ApplicationDock {
                element: if value["element"].is_null() {
                    None
                } else {
                    Some(action(&value["element"])?)
                },
            })
        }
        Some("clipboard.subscribe") => {
            fields(value, &["operation", "enabled"])?;
            require(grants, Permission::ClipboardRead)?;
            Ok(Request::ClipboardSubscribe(boolean(&value["enabled"])?))
        }
        Some("notifications.subscribe") => {
            fields(value, &["operation", "enabled"])?;
            require(grants, Permission::NotificationRead)?;
            Ok(Request::NotificationSubscribe(boolean(&value["enabled"])?))
        }
        Some("clipboard.restore") => {
            fields(value, &["operation", "blob", "gesture"])?;
            require(grants, Permission::ClipboardWrite)?;
            Ok(Request::ClipboardRestore {
                blob: opaque(&value["blob"])?,
                gesture: opaque(&value["gesture"])?,
            })
        }
        Some("blobs.release") => {
            fields(value, &["operation", "blobs"])?;
            Ok(Request::ReleaseBlobs(
                array(&value["blobs"], 128)?
                    .iter()
                    .map(opaque)
                    .collect::<Result<_, _>>()?,
            ))
        }
        Some("shell.theme") => {
            fields(value, &["operation", "colors"])?;
            require(grants, Permission::Theme)?;
            let colors = value["colors"]
                .as_object()
                .filter(|o| o.len() <= 4)
                .ok_or_else(|| tr("shell-invalid-plugin-colors"))?;
            let mut parsed = BTreeMap::new();
            for (key, value) in colors {
                if !matches!(key.as_str(), "background" | "surface" | "text" | "accent") {
                    return Err(tr("shell-unknown-plugin-color-target").into());
                }
                let value = value
                    .as_str()
                    .filter(|s| {
                        s.len() == 7
                            && s.starts_with('#')
                            && s.strip_prefix('#')
                                .is_some_and(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()))
                    })
                    .ok_or_else(|| tr("shell-invalid-plugin-color"))?;
                let mut rgb = [0; 3];
                for (i, pair) in value.as_bytes()[1..].as_chunks::<2>().0.iter().enumerate() {
                    rgb[i] = u8::from_str_radix(
                        std::str::from_utf8(pair).map_err(|_| tr("shell-invalid-plugin-color"))?,
                        16,
                    )
                    .map_err(|_| tr("shell-invalid-plugin-color"))?;
                }
                parsed.insert(key.clone(), rgb);
            }
            Ok(Request::Theme(parsed))
        }
        Some("shell.layout") => {
            fields(
                value,
                &["operation", "element", "section", "before", "enabled"],
            )?;
            require(grants, Permission::Layout)?;
            let element = value["element"]
                .as_str()
                .ok_or_else(|| tr("shell-missing-plugin-layout-target"))?;
            Item::new(element, true)?;
            let before = optional(&value["before"], |v| {
                let s = v
                    .as_str()
                    .ok_or_else(|| tr("shell-invalid-plugin-layout-anchor"))?;
                Item::new(s, true)?;
                Ok(s.to_owned())
            })?;
            Ok(Request::Layout {
                element: element.into(),
                section: Section::parse(
                    value["section"]
                        .as_str()
                        .ok_or_else(|| tr("shell-missing-plugin-layout-section"))?,
                )?,
                before,
                enabled: optional(&value["enabled"], boolean)?,
            })
        }
        Some("shell.open") => {
            fields(value, &["operation", "view", "gesture"])?;
            require(grants, Permission::Behavior)?;
            let view = match value["view"].as_str() {
                Some("applications") => ShellView::Applications,
                Some("desktops") => ShellView::Desktops,
                Some("controls") => ShellView::Controls,
                Some("settings") => ShellView::Settings,
                _ => return Err(tr("shell-unknown-plugin-shell-view").into()),
            };
            Ok(Request::Open {
                view,
                gesture: opaque(&value["gesture"])?,
            })
        }
        _ => Err(tr("shell-unsupported-plugin-operation").into()),
    }
}
pub fn decode(
    value: &Value,
    manifest: &Manifest,
    grants: &BTreeSet<Permission>,
) -> Result<Output, String> {
    if !grants.is_subset(&manifest.permissions) {
        return Err(tr("shell-plugin-grant-context-does-not-match-its-manifest").into());
    }
    match value["type"].as_str() {
        Some("ready") => {
            fields(value, &["type"])?;
            Ok(Output::Ready)
        }
        Some("pong") => {
            fields(value, &["type", "sequence"])?;
            Ok(Output::Pong(
                value["sequence"]
                    .as_u64()
                    .ok_or_else(|| tr("shell-invalid-plugin-heartbeat"))?,
            ))
        }
        Some("snapshot") => snapshot(value, manifest, grants).map(Output::Snapshot),
        Some("request") => {
            fields(value, &["type", "id", "request"])?;
            let id = value["id"]
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or_else(|| tr("shell-invalid-plugin-request-identity"))?;
            Ok(Output::Request {
                id,
                request: request(&value["request"], grants)?,
            })
        }
        _ => Err(tr("shell-unsupported-plugin-message").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::manifest::Package;
    use super::*;
    use serde_json::json;
    fn manifest() -> Manifest {
        Package::parse(&serde_json::to_vec(&json!({"format":1,"id":"fixture","name":"Fixture","version":"1","description":"Disposable fixture","permissions":["shell.bar","shell.panels","clipboard.write"],"elements":[{"id":"status","section":"end","before":null}],"settings":[],"program":"pass"})).unwrap()).unwrap().manifest().clone()
    }
    fn ui() -> Value {
        json!({"type":"snapshot","revision":1,"elements":[{"id":"status","icon":"clipboard","label":"Clipboard","tooltip":"History","panel":{"title":"Clipboard","width":400,"height":500,"nodes":[{"type":"action","key":"clear","id":"clear","label":"Clear history","detail":null,"icon":"trash-2","thumbnail":null,"enabled":true}]}}]})
    }
    #[test]
    fn protocol_behaviour() {
        // Application surface requires both grants and rejects guest commands.
        {
            let package = super::super::applications::package().unwrap();
            let manifest = package.manifest();
            let value = json!({"type":"request","id":1,"request":{"operation":"applications.dock","element":"applications"}});
            assert!(decode(&value, manifest, &BTreeSet::from([Permission::Bar])).is_err());
            assert!(
                decode(
                    &value,
                    manifest,
                    &BTreeSet::from([Permission::Applications])
                )
                .is_err()
            );
            assert!(matches!(
                decode(&value, manifest, &manifest.permissions),
                Ok(Output::Request {
                    request: Request::ApplicationDock { .. },
                    ..
                })
            ));
            let mut release = value.clone();
            release["request"]["element"] = Value::Null;
            assert!(decode(&release, manifest, &manifest.permissions).is_ok());
            let mut command = value;
            command["request"]["command"] = json!("sh");
            assert!(decode(&command, manifest, &manifest.permissions).is_err());
        }
        // Images cannot be paths and nested ui has a finite budget.
        {
            let manifest = manifest();
            let grants = BTreeSet::from([Permission::Bar, Permission::Panels]);
            let mut v = ui();
            v["elements"][0]["panel"]["nodes"][0]["thumbnail"] = json!("/home/private.png");
            assert!(decode(&v, &manifest, &grants).is_err());
            let mut node = json!({"type":"separator","key":"separator"});
            for i in 0..8 {
                node = json!({"type":"column","key":format!("column-{i}"),"children":[node]});
            }
            v = ui();
            v["elements"][0]["panel"]["nodes"] = json!([node]);
            assert!(decode(&v, &manifest, &grants).is_err());
        }
        // Operations are explicit and do not accept host commands.
        {
            let manifest = manifest();
            let restore = json!({"type":"request","id":1,"request":{"operation":"clipboard.restore","blob":"0".repeat(32),"gesture":"1".repeat(32)}});
            assert!(decode(&restore, &manifest, &BTreeSet::new()).is_err());
            assert!(matches!(
                decode(
                    &restore,
                    &manifest,
                    &BTreeSet::from([Permission::ClipboardWrite])
                ),
                Ok(Output::Request {
                    request: Request::ClipboardRestore { .. },
                    ..
                })
            ));
            let exec = json!({"type":"request","id":1,"request":{"operation":"host.exec","command":"echo forbidden"}});
            assert!(decode(&exec, &manifest, &Permission::ALL.into_iter().collect()).is_err());
        }
    }
}

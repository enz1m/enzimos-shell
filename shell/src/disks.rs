//! Disks and partitions through UDisks2, for the Settings Disks page.
//!
//! Every change goes through UDisks, which asks polkit: drives that can be
//! unplugged change without a password in the active session, internal
//! disks need an administrator. A disk holding the running system (its root,
//! boot, home or swap, directly or through encryption) is shown but never
//! offered for a change here.

use enzui_shell::i18n::tr;
use std::collections::HashMap;
use std::time::Duration;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const UDISKS: &str = "org.freedesktop.UDisks2";
const READ: Duration = Duration::from_secs(20);
/// Formatting and partitioning, including the administrator dialog.
const CHANGE: Duration = Duration::from_secs(600);
/// Smallest unallocated region worth offering.
const MIN_FREE: u64 = 16 * 1024 * 1024;
const MIB: u64 = 1024 * 1024;

/// File systems offered when formatting or creating a partition, with
/// their MBR partition type.
pub const FILESYSTEMS: [(&str, &str); 5] = [
    ("ext4", "0x83"),
    ("btrfs", "0x83"),
    ("exfat", "0x07"),
    ("ntfs", "0x07"),
    ("vfat", "0x0c"),
];

/// Mount points that make a disk part of the running system.
const SYSTEM_MOUNTS: [&str; 7] = ["/", "/boot", "/efi", "/boot/efi", "/home", "/usr", "/var"];

/// One partition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Partition {
    /// UDisks block object path.
    pub path: String,
    pub number: u32,
    pub offset: u64,
    pub size: u64,
    /// File system or content type (`ext4`, `crypto_LUKS`, `swap`, …).
    pub content: String,
    pub label: String,
    /// The partition type's name (`EFI System`, `Linux filesystem`, …).
    pub kind: String,
    pub mounts: Vec<String>,
    /// Part of the running system.
    pub system: bool,
}

/// A region of a disk without a partition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Free {
    pub offset: u64,
    pub size: u64,
}

/// One disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disk {
    /// UDisks block object path of the whole disk.
    pub path: String,
    pub name: String,
    pub size: u64,
    /// `gpt`, `dos`, or empty when there is no partition table.
    pub table: String,
    /// The whole disk holds a file system without a partition table.
    pub content: String,
    pub label: String,
    pub removable: bool,
    /// USB, SD card, NVMe, …
    pub bus: String,
    pub read_only: bool,
    /// Holds the running system: nothing on it is offered for a change.
    pub system: bool,
    pub partitions: Vec<Partition>,
    pub free: Vec<Free>,
}

type Interfaces = HashMap<String, HashMap<String, OwnedValue>>;
type Objects = HashMap<OwnedObjectPath, Interfaces>;

fn prop<'a>(o: &'a Interfaces, iface: &str, key: &str) -> Option<&'a OwnedValue> {
    o.get(&format!("org.freedesktop.UDisks2.{iface}"))
        .and_then(|p| p.get(key))
}
fn has(o: &Interfaces, iface: &str) -> bool {
    o.contains_key(&format!("org.freedesktop.UDisks2.{iface}"))
}
fn text(o: &Interfaces, iface: &str, key: &str) -> String {
    prop(o, iface, key)
        .and_then(|v| String::try_from(v.clone()).ok())
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(120)
        .collect()
}
fn number(o: &Interfaces, iface: &str, key: &str) -> u64 {
    prop(o, iface, key)
        .and_then(|v| u64::try_from(v.clone()).ok())
        .unwrap_or_default()
}
fn flag(o: &Interfaces, iface: &str, key: &str) -> bool {
    prop(o, iface, key)
        .and_then(|v| bool::try_from(v).ok())
        .unwrap_or(false)
}
fn object(o: &Interfaces, iface: &str, key: &str) -> Option<String> {
    prop(o, iface, key)
        .and_then(|v| OwnedObjectPath::try_from(v.clone()).ok())
        .map(|p| p.to_string())
        .filter(|p| p != "/")
}
fn mounts(o: &Interfaces) -> Vec<String> {
    prop(o, "Filesystem", "MountPoints")
        .and_then(|v| <Vec<Vec<u8>>>::try_from(v.clone()).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|raw| {
            String::from_utf8_lossy(raw.split(|b| *b == 0).next().unwrap_or_default()).into_owned()
        })
        .collect()
}

/// Readable names of common partition types.
fn kind_name(kind: &str) -> String {
    match kind.to_ascii_lowercase().as_str() {
        "c12a7328-f81f-11d2-ba4b-00a0c93ec93b" | "0xef" => tr("shell-disks-type-efi").into(),
        "0fc63daf-8483-4772-8e79-3d69d8477de4" | "0x83" => tr("shell-disks-type-linux").into(),
        "4f68bce3-e8cd-4db1-96e7-fbcaf984b709" => tr("shell-disks-type-linux-root").into(),
        "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f" | "0x82" => tr("shell-disks-type-swap").into(),
        "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7" | "0x07" | "0x0c" | "0x0b" => {
            tr("shell-disks-type-basic-data").into()
        }
        "e3c9e316-0b5c-4db8-817d-f92df00215ae" => tr("shell-disks-type-microsoft-reserved").into(),
        "de94bba4-06d1-4d40-a16a-bfd50179d6ac" => tr("shell-disks-type-windows-recovery").into(),
        "21686148-6449-6e6f-744e-656564454649" => tr("shell-disks-type-bios-boot").into(),
        _ => String::new(),
    }
}

/// Unallocated regions of a disk of `size` bytes with these partitions,
/// within the usable area (1 MiB from the start, 1 MiB before the end for
/// the GPT backup), large enough to use.
#[must_use]
pub fn free_regions(size: u64, partitions: &[(u64, u64)]) -> Vec<Free> {
    let start = MIB;
    let end = size.saturating_sub(MIB);
    let mut sorted: Vec<(u64, u64)> = partitions.to_vec();
    sorted.sort_unstable();
    let mut out = Vec::new();
    let mut cursor = start;
    for (offset, length) in sorted {
        if offset > cursor && offset - cursor >= MIN_FREE {
            out.push(Free {
                offset: cursor.div_ceil(MIB) * MIB,
                size: (offset - cursor) / MIB * MIB,
            });
        }
        cursor = cursor.max(offset.saturating_add(length));
    }
    if end > cursor && end - cursor >= MIN_FREE {
        let offset = cursor.div_ceil(MIB) * MIB;
        out.push(Free {
            offset,
            size: (end - offset) / MIB * MIB,
        });
    }
    out
}

/// Build the disk list from UDisks objects.
#[must_use]
pub fn disks(objects: &Objects) -> Vec<Disk> {
    // Which block devices back the running system, following encryption.
    let mut system_blocks: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut ordered: Vec<(&OwnedObjectPath, &Interfaces)> = objects.iter().collect();
    ordered.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    for (path, o) in ordered {
        let points = mounts(o);
        let swap_active = has(o, "Swapspace") && flag(o, "Swapspace", "Active");
        if swap_active || points.iter().any(|p| SYSTEM_MOUNTS.contains(&p.as_str())) {
            system_blocks.insert(path.to_string());
            if let Some(backing) = object(o, "Block", "CryptoBackingDevice") {
                system_blocks.insert(backing);
            }
        }
    }
    let mut out = Vec::new();
    let mut sorted: Vec<(&OwnedObjectPath, &Interfaces)> = objects.iter().collect();
    sorted.sort_by(|a, b| a.0.as_str().cmp(b.0.as_str()));
    for &(path, o) in &sorted {
        // Whole disks: a block with a drive that is not a partition.
        if !has(o, "Block")
            || has(o, "Partition")
            || flag(o, "Block", "HintIgnore")
            || has(o, "Loop")
        {
            continue;
        }
        let Some(drive_path) = object(o, "Block", "Drive") else {
            continue;
        };
        let Some(drive) = objects
            .iter()
            .find(|(p, _)| p.as_str() == drive_path)
            .map(|(_, d)| d)
        else {
            continue;
        };
        if flag(drive, "Drive", "Optical") || number(o, "Block", "Size") == 0 {
            continue;
        }
        let size = number(o, "Block", "Size");
        let model = format!(
            "{} {}",
            text(drive, "Drive", "Vendor"),
            text(drive, "Drive", "Model")
        )
        .trim()
        .to_owned();
        let mut partitions: Vec<Partition> = sorted
            .iter()
            .filter(|(_, p)| {
                has(p, "Partition")
                    && object(p, "Partition", "Table").as_deref() == Some(path.as_str())
            })
            .map(|(ppath, p)| {
                let cleartext = object(p, "Encrypted", "CleartextDevice").and_then(|c| {
                    objects
                        .iter()
                        .find(|(cp, _)| cp.as_str() == c)
                        .map(|(_, x)| x)
                });
                let shown = cleartext.unwrap_or(p);
                let mut points = mounts(p);
                points.extend(mounts(shown));
                Partition {
                    path: ppath.to_string(),
                    number: u32::try_from(number(p, "Partition", "Number")).unwrap_or(0),
                    offset: number(p, "Partition", "Offset"),
                    size: number(p, "Partition", "Size"),
                    content: text(p, "Block", "IdType"),
                    label: {
                        let own = text(shown, "Block", "IdLabel");
                        if own.is_empty() {
                            text(p, "Partition", "Name")
                        } else {
                            own
                        }
                    },
                    kind: kind_name(&text(p, "Partition", "Type")),
                    system: system_blocks.contains(ppath.as_str()),
                    mounts: points,
                }
            })
            .collect();
        partitions.sort_by_key(|p| p.offset);
        let table = text(o, "PartitionTable", "Type");
        let free = if table.is_empty() {
            Vec::new()
        } else {
            free_regions(
                size,
                &partitions
                    .iter()
                    .map(|p| (p.offset, p.size))
                    .collect::<Vec<_>>(),
            )
        };
        let system = system_blocks.contains(path.as_str()) || partitions.iter().any(|p| p.system);
        out.push(Disk {
            path: path.to_string(),
            name: if model.is_empty() {
                text(o, "Block", "PreferredDevice")
            } else {
                model
            },
            size,
            table,
            content: if has(o, "PartitionTable") {
                String::new()
            } else {
                text(o, "Block", "IdType")
            },
            label: text(o, "Block", "IdLabel"),
            removable: flag(drive, "Drive", "Removable") || flag(drive, "Drive", "MediaRemovable"),
            bus: text(drive, "Drive", "ConnectionBus"),
            read_only: flag(o, "Block", "ReadOnly"),
            system,
            partitions,
            free,
        });
    }
    // The system disk first, then internal disks, then removable ones.
    out.sort_by(|a, b| (!a.system, a.removable, &a.name).cmp(&(!b.system, b.removable, &b.name)));
    out
}

/// Read every disk.
///
/// # Errors
/// A sentence when UDisks2 does not answer.
pub async fn read() -> Result<Vec<Disk>, String> {
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus").to_owned())?;
    let proxy = zbus::fdo::ObjectManagerProxy::builder(&system)
        .destination(UDISKS)
        .and_then(|b| b.path("/org/freedesktop/UDisks2"))
        .map_err(|_| tr("shell-disks-service-unavailable").to_owned())?
        .build()
        .await
        .map_err(|_| tr("shell-disks-service-unavailable").to_owned())?;
    let objects = tokio::time::timeout(READ, proxy.get_managed_objects())
        .await
        .map_err(|_| tr("shell-disks-service-did-not-answer").to_owned())?
        .map_err(|_| tr("shell-disks-service-unavailable").to_owned())?;
    let objects: Objects = objects
        .into_iter()
        .map(|(path, ifaces)| {
            (
                path,
                ifaces
                    .into_iter()
                    .map(|(name, props)| (name.to_string(), props))
                    .collect(),
            )
        })
        .collect();
    Ok(disks(&objects))
}

/// A volume label valid for file system `fs` (empty means none).
#[must_use]
pub fn valid_label(fs: &str, label: &str) -> bool {
    if label.chars().any(char::is_control) {
        return false;
    }
    match fs {
        "vfat" => {
            label.len() <= 11
                && label.is_ascii()
                && !label.contains([
                    '"', '*', '+', ',', '.', '/', ':', ';', '<', '=', '>', '?', '[', '\\', ']', '|',
                ])
        }
        "exfat" => label.encode_utf16().count() <= 15,
        "ntfs" => label.encode_utf16().count() <= 32,
        "ext4" => label.len() <= 16,
        "btrfs" => label.len() <= 255,
        _ => false,
    }
}

fn options() -> HashMap<&'static str, Value<'static>> {
    HashMap::new()
}

fn format_options(fs: &str, label: &str) -> HashMap<&'static str, Value<'static>> {
    // No "tear-down": it also edits fstab/crypttab and so always needs an
    // administrator; `release` closes what is open instead.
    let mut options = options();
    options.insert("update-partition-type", Value::from(true));
    if !label.is_empty() {
        options.insert(
            "label",
            Value::from(if fs == "vfat" {
                label.to_ascii_uppercase()
            } else {
                label.to_owned()
            }),
        );
    }
    if fs == "ext4" || fs == "btrfs" {
        options.insert("take-ownership", Value::from(true));
    }
    options
}

async fn call<B, R>(path: &str, iface: &str, method: &str, body: &B) -> Result<R, String>
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
    R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
{
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus").to_owned())?;
    let proxy = zbus::Proxy::new(
        &system,
        UDISKS,
        path.to_owned(),
        format!("org.freedesktop.UDisks2.{iface}"),
    )
    .await
    .map_err(|_| tr("shell-disks-service-unavailable").to_owned())?;
    match tokio::time::timeout(
        CHANGE,
        proxy.call_with_flags::<_, B, R>(
            method,
            zbus::proxy::MethodFlags::AllowInteractiveAuth.into(),
            body,
        ),
    )
    .await
    {
        Err(_) => Err(tr("shell-the-change-took-too-long-nothing-was-confirmed").into()),
        Ok(Ok(Some(reply))) => Ok(reply),
        Ok(Ok(None)) => Err(tr("shell-disks-change-failed").into()),
        Ok(Err(zbus::Error::MethodError(name, message, _))) => Err(explain(
            name.as_str(),
            message.as_deref().unwrap_or_default(),
        )),
        Ok(Err(_)) => Err(tr("shell-disks-change-failed").into()),
    }
}

fn explain(name: &str, message: &str) -> String {
    if name.ends_with("NotAuthorized")
        || name.ends_with("NotAuthorizedCanObtain")
        || name.contains("AccessDenied")
        || name.ends_with("Cancelled")
    {
        tr("shell-an-administrator-did-not-approve-this-change").into()
    } else if name.ends_with("DeviceBusy") || message.contains("busy") {
        tr("shell-disks-busy").into()
    } else {
        tr("shell-disks-change-failed").into()
    }
}

/// Close everything open on block `path` and the blocks inside it
/// (partitions, unlocked encrypted volumes): unmount, then lock.
async fn release(path: &str) -> Result<(), String> {
    let system = zbus::Connection::system()
        .await
        .map_err(|_| tr("shell-no-system-bus").to_owned())?;
    let proxy = zbus::fdo::ObjectManagerProxy::builder(&system)
        .destination(UDISKS)
        .and_then(|b| b.path("/org/freedesktop/UDisks2"))
        .map_err(|_| tr("shell-disks-service-unavailable").to_owned())?
        .build()
        .await
        .map_err(|_| tr("shell-disks-service-unavailable").to_owned())?;
    let objects: Objects = tokio::time::timeout(READ, proxy.get_managed_objects())
        .await
        .map_err(|_| tr("shell-disks-service-did-not-answer").to_owned())?
        .map_err(|_| tr("shell-disks-service-unavailable").to_owned())?
        .into_iter()
        .map(|(p, ifaces)| {
            (
                p,
                ifaces
                    .into_iter()
                    .map(|(n, props)| (n.to_string(), props))
                    .collect(),
            )
        })
        .collect();
    // The block itself, its partitions, and their unlocked encrypted contents.
    let mut targets: Vec<String> = vec![path.to_owned()];
    targets.extend(
        objects
            .iter()
            .filter(|(_, o)| object(o, "Partition", "Table").as_deref() == Some(path))
            .map(|(p, _)| p.to_string()),
    );
    let cleartext: Vec<String> = targets
        .iter()
        .filter_map(|t| objects.iter().find(|(p, _)| p.as_str() == t))
        .filter_map(|(_, o)| object(o, "Encrypted", "CleartextDevice"))
        .collect();
    for block in cleartext.iter().chain(targets.iter()) {
        if let Some((_, o)) = objects.iter().find(|(p, _)| p.as_str() == block)
            && !mounts(o).is_empty()
        {
            call::<_, ()>(
                block,
                "Filesystem",
                "Unmount",
                &(HashMap::<&str, Value<'_>>::new(),),
            )
            .await?;
        }
    }
    for block in &targets {
        if let Some((_, o)) = objects.iter().find(|(p, _)| p.as_str() == block)
            && object(o, "Encrypted", "CleartextDevice").is_some()
        {
            call::<_, ()>(
                block,
                "Encrypted",
                "Lock",
                &(HashMap::<&str, Value<'_>>::new(),),
            )
            .await?;
        }
    }
    Ok(())
}

/// Quick-format partition (or partition-less disk) `path` as `fs`.
///
/// # Errors
/// A sentence when UDisks or polkit refused.
pub async fn format(path: &str, fs: &str, label: &str) -> Result<(), String> {
    release(path).await?;
    call::<_, ()>(path, "Block", "Format", &(fs, format_options(fs, label))).await
}

/// Remove partition `path`.
///
/// # Errors
/// A sentence when UDisks or polkit refused.
pub async fn delete(path: &str) -> Result<(), String> {
    release(path).await?;
    call::<_, ()>(path, "Partition", "Delete", &(options(),)).await
}

/// Create a partition of `size` bytes (0: the whole region) at `offset` on
/// disk `disk` with partition table `table`, formatted as `fs`.
///
/// # Errors
/// A sentence when UDisks or polkit refused.
pub async fn create(
    disk: &str,
    table: &str,
    offset: u64,
    size: u64,
    fs: &str,
    label: &str,
) -> Result<(), String> {
    let kind = if table == "dos" {
        FILESYSTEMS
            .iter()
            .find(|(name, _)| *name == fs)
            .map_or("0x83", |(_, kind)| *kind)
    } else {
        ""
    };
    call::<_, OwnedObjectPath>(
        disk,
        "PartitionTable",
        "CreatePartitionAndFormat",
        &(
            offset,
            size,
            kind,
            "",
            HashMap::<&str, Value<'_>>::new(),
            fs,
            format_options(fs, label),
        ),
    )
    .await
    .map(|_| ())
}

/// Erase disk `disk`: a new empty partition table (`gpt`, or `dos` for
/// cameras, cars and older computers).
///
/// # Errors
/// A sentence when UDisks or polkit refused.
pub async fn erase(disk: &str, table: &str) -> Result<(), String> {
    release(disk).await?;
    call::<_, ()>(disk, "Block", "Format", &(table, options())).await
}

/// Size text, base 10 like drive makers print it.
#[must_use]
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["MB", "GB", "TB", "PB"];
    let mut value = bytes as f64 / 1_000_000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    let number = if value >= 10.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    };
    format!("{} {}", enzui_shell::i18n::number(number), UNITS[unit])
}

/// Sample data for the Settings preview.
#[must_use]
pub fn demo() -> Vec<Disk> {
    let gb = 1_000_000_000u64;
    let part = |n: u32,
                offset: u64,
                size: u64,
                content: &str,
                label: &str,
                kind: &str,
                mounts: &[&str],
                system: bool| Partition {
        path: format!("/demo/{n}"),
        number: n,
        offset,
        size,
        content: content.into(),
        label: label.into(),
        kind: kind.into(),
        mounts: mounts.iter().map(|m| (*m).to_owned()).collect(),
        system,
    };
    vec![
        Disk {
            path: "/demo/nvme0n1".into(),
            name: "Samsung SSD 990 PRO 2TB".into(),
            size: 2000 * gb,
            table: "gpt".into(),
            content: String::new(),
            label: String::new(),
            removable: false,
            bus: String::new(),
            read_only: false,
            system: true,
            partitions: vec![
                part(1, MIB, gb, "vfat", "EFI", "EFI System", &["/efi"], true),
                part(
                    2,
                    MIB + gb,
                    1999 * gb - 2 * MIB,
                    "crypto_LUKS",
                    "enzimOS",
                    "Linux filesystem",
                    &["/", "/home"],
                    true,
                ),
            ],
            free: Vec::new(),
        },
        Disk {
            path: "/demo/sda".into(),
            name: "WD Elements 1TB".into(),
            size: 1000 * gb,
            table: "gpt".into(),
            content: String::new(),
            label: String::new(),
            removable: true,
            bus: "usb".into(),
            read_only: false,
            system: false,
            partitions: vec![part(
                1,
                MIB,
                600 * gb,
                "exfat",
                "Backups",
                "Basic data",
                &["/run/media/alice/Backups"],
                false,
            )],
            free: vec![Free {
                offset: 600 * gb + MIB,
                size: 400 * gb - 2 * MIB,
            }],
        },
        Disk {
            path: "/demo/sdb".into(),
            name: "SanDisk Ultra".into(),
            size: 32 * gb,
            table: "dos".into(),
            content: String::new(),
            label: String::new(),
            removable: true,
            bus: "usb".into(),
            read_only: false,
            system: false,
            partitions: vec![part(
                1,
                MIB,
                32 * gb - MIB,
                "vfat",
                "STICK",
                "Basic data",
                &[],
                false,
            )],
            free: Vec::new(),
        },
    ]
}

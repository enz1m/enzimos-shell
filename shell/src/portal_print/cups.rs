#![allow(
    unsafe_code,
    reason = "The optional CUPS 2 client is loaded at runtime."
)]

use super::{Duplex, Paper, Printer, Selection};
use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fs::File;
use std::os::fd::{AsRawFd, OwnedFd};
use std::ptr::null_mut;
use std::sync::OnceLock;

#[repr(C)]
struct Destination {
    name: *const c_char,
    instance: *const c_char,
    is_default: c_int,
    num_options: c_int,
    options: *mut OptionPair,
}

#[repr(C)]
struct OptionPair {
    name: *const c_char,
    value: *const c_char,
}

#[repr(C)]
struct Media {
    name: [c_char; 128],
    width: c_int,
    length: c_int,
    bottom: c_int,
    left: c_int,
    right: c_int,
    top: c_int,
}

type Handle = *mut c_void;
type Connect = unsafe extern "C" fn(
    *const c_char,
    c_int,
    Handle,
    c_int,
    c_int,
    c_int,
    c_int,
    *mut c_int,
) -> Handle;
type MediaLookup =
    unsafe extern "C" fn(Handle, *mut Destination, Handle, c_int, u32, *mut Media) -> c_int;
type FindAttribute =
    unsafe extern "C" fn(Handle, *mut Destination, Handle, *const c_char) -> Handle;

struct Cups {
    server: unsafe extern "C" fn() -> *const c_char,
    port: unsafe extern "C" fn() -> c_int,
    encryption: unsafe extern "C" fn() -> c_int,
    connect: Connect,
    timeout: unsafe extern "C" fn(
        Handle,
        f64,
        Option<unsafe extern "C" fn(Handle, Handle) -> c_int>,
        Handle,
    ),
    close: unsafe extern "C" fn(Handle),
    destinations: unsafe extern "C" fn(Handle, *mut *mut Destination) -> c_int,
    free_destinations: unsafe extern "C" fn(c_int, *mut Destination),
    info: unsafe extern "C" fn(Handle, *mut Destination) -> Handle,
    free_info: unsafe extern "C" fn(Handle),
    media_count: unsafe extern "C" fn(Handle, *mut Destination, Handle, u32) -> c_int,
    media_at: MediaLookup,
    media_default: unsafe extern "C" fn(Handle, *mut Destination, Handle, u32, *mut Media) -> c_int,
    media_label:
        unsafe extern "C" fn(Handle, *mut Destination, Handle, u32, *mut Media) -> *const c_char,
    supported: FindAttribute,
    default: FindAttribute,
    get_range: unsafe extern "C" fn(Handle, c_int, *mut c_int) -> c_int,
    get_string: unsafe extern "C" fn(Handle, c_int, *mut *const c_char) -> *const c_char,
    check_supported: unsafe extern "C" fn(
        Handle,
        *mut Destination,
        Handle,
        *const c_char,
        *const c_char,
    ) -> c_int,
    print_file: unsafe extern "C" fn(
        Handle,
        *const c_char,
        *const c_char,
        *const c_char,
        c_int,
        *const OptionPair,
    ) -> c_int,
    error: unsafe extern "C" fn() -> *const c_char,
    _library: libloading::Library,
}

impl Cups {
    fn get() -> Result<&'static Self, String> {
        static CUPS: OnceLock<Result<Cups, String>> = OnceLock::new();
        CUPS.get_or_init(Self::load).as_ref().map_err(Clone::clone)
    }

    fn load() -> Result<Self, String> {
        // SAFETY: libcups.so.2 provides these signatures and stays loaded for the lifetime of the function pointers.
        let result: Result<Self, libloading::Error> = unsafe {
            (|| {
                let lib = libloading::Library::new("libcups.so.2")?;
                Ok(Self {
                    server: *lib.get(b"cupsServer\0")?,
                    port: *lib.get(b"ippPort\0")?,
                    encryption: *lib.get(b"cupsEncryption\0")?,
                    connect: *lib.get(b"httpConnect2\0")?,
                    timeout: *lib.get(b"httpSetTimeout\0")?,
                    close: *lib.get(b"httpClose\0")?,
                    destinations: *lib.get(b"cupsGetDests2\0")?,
                    free_destinations: *lib.get(b"cupsFreeDests\0")?,
                    info: *lib.get(b"cupsCopyDestInfo\0")?,
                    free_info: *lib.get(b"cupsFreeDestInfo\0")?,
                    media_count: *lib.get(b"cupsGetDestMediaCount\0")?,
                    media_at: *lib.get(b"cupsGetDestMediaByIndex\0")?,
                    media_default: *lib.get(b"cupsGetDestMediaDefault\0")?,
                    media_label: *lib.get(b"cupsLocalizeDestMedia\0")?,
                    supported: *lib.get(b"cupsFindDestSupported\0")?,
                    default: *lib.get(b"cupsFindDestDefault\0")?,
                    get_range: *lib.get(b"ippGetRange\0")?,
                    get_string: *lib.get(b"ippGetString\0")?,
                    check_supported: *lib.get(b"cupsCheckDestSupported\0")?,
                    print_file: *lib.get(b"cupsPrintFile2\0")?,
                    error: *lib.get(b"cupsLastErrorString\0")?,
                    _library: lib,
                })
            })()
        };
        result.map_err(|error| error.to_string())
    }

    fn connect(&self) -> Result<Connection<'_>, String> {
        // SAFETY: libcups supplies the server name and settings, and a null address list requests normal resolution.
        let http = unsafe {
            (self.connect)(
                (self.server)(),
                (self.port)(),
                null_mut(),
                libc::AF_UNSPEC,
                (self.encryption)(),
                1,
                3000,
                null_mut(),
            )
        };
        if http.is_null() {
            return Err("Could not connect to CUPS.".into());
        }
        // SAFETY: The live connection has no timeout callback or associated user data.
        unsafe {
            (self.timeout)(http, 5.0, None, null_mut());
        }
        Ok(Connection { cups: self, http })
    }

    fn error(&self) -> String {
        // SAFETY: The thread-local status string is copied before any subsequent CUPS calls.
        unsafe {
            read_string((self.error)()).unwrap_or_else(|| "CUPS rejected the print job.".into())
        }
    }
}

struct Connection<'a> {
    cups: &'a Cups,
    http: Handle,
}

impl Drop for Connection<'_> {
    fn drop(&mut self) {
        // SAFETY: This connection owns the handle returned by httpConnect2.
        unsafe {
            (self.cups.close)(self.http);
        }
    }
}

/// Familiar names for common PWG sizes; CUPS labels them by dimensions (`8.5 x 11″`).
pub(super) fn common_paper_label(media: &str) -> Option<&'static str> {
    // PWG names end in their dimensions, as in `na_letter_8.5x11in`.
    Some(
        match media.rsplit_once('_').map_or(media, |(name, _)| name) {
            "na_letter" => "Letter",
            "na_legal" => "Legal",
            "na_executive" => "Executive",
            "na_ledger" => "Tabloid",
            "iso_a3" => "A3",
            "iso_a4" => "A4",
            "iso_a5" => "A5",
            "iso_a6" => "A6",
            "iso_b5" => "B5",
            "jis_b5" => "JIS B5",
            "iso_dl" => "DL envelope",
            "na_number-10" => "#10 envelope",
            _ => return None,
        },
    )
}

/// A destination option such as `printer-info`, when present and not empty.
unsafe fn option_value(dest: &Destination, key: &CStr) -> Option<String> {
    let count = usize::try_from(dest.num_options).ok()?;
    if dest.options.is_null() || count == 0 {
        return None;
    }
    // SAFETY: CUPS gives `num_options` pairs of valid C strings for a live destination.
    unsafe {
        let options = std::slice::from_raw_parts(dest.options, count);
        let pair = options
            .iter()
            .find(|pair| !pair.name.is_null() && CStr::from_ptr(pair.name) == key)?;
        read_string(pair.value).filter(|value| !value.trim().is_empty())
    }
}

unsafe fn read_string(value: *const c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }
    // SAFETY: The caller supplies a live, NUL-terminated CUPS string.
    Some(
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned(),
    )
}

pub(super) fn printers() -> Result<Vec<Printer>, String> {
    let cups = Cups::get()?;
    let connection = cups.connect()?;
    let mut destinations = null_mut();
    // SAFETY: CUPS allocates a count-sized destination array which stays alive through capability queries.
    unsafe {
        let count = (cups.destinations)(connection.http, &mut destinations);
        let printers = if count > 0 && !destinations.is_null() {
            (0..count)
                .filter_map(|index| printer(&connection, destinations.add(index as usize)))
                .collect()
        } else {
            Vec::new()
        };
        (cups.free_destinations)(count.max(0), destinations);
        Ok(printers)
    }
}

unsafe fn printer(connection: &Connection<'_>, dest: *mut Destination) -> Option<Printer> {
    let cups = connection.cups;
    let http = connection.http;
    // SAFETY: The caller holds a live destination from this connection. Borrowed IPP attributes and
    // C strings are used before the destination information is freed.
    unsafe {
        if !(*dest).instance.is_null() {
            return None;
        }
        let name = read_string((*dest).name)?;
        let label = option_value(&*dest, c"printer-info").unwrap_or_else(|| name.clone());
        let mut printer = Printer {
            name,
            label,
            is_default: (*dest).is_default != 0,
            papers: Vec::new(),
            default_paper: 0,
            duplex: Vec::new(),
            default_duplex: None,
            color: Vec::new(),
            default_color: None,
            max_copies: 99,
        };
        let info = (cups.info)(http, dest);
        if info.is_null() {
            return Some(printer);
        }
        let mut default_media = std::mem::zeroed::<Media>();
        let default_media = ((cups.media_default)(http, dest, info, 0, &mut default_media) != 0)
            .then(|| read_string(default_media.name.as_ptr()))
            .flatten();
        for index in 0..(cups.media_count)(http, dest, info, 0) {
            let mut media = std::mem::zeroed::<Media>();
            if (cups.media_at)(http, dest, info, index, 0, &mut media) == 0 {
                continue;
            }
            let Some(name) = read_string(media.name.as_ptr()) else {
                continue;
            };
            if media.width <= 0
                || media.length <= 0
                || printer.papers.iter().any(|paper| paper.media == name)
            {
                continue;
            }
            if default_media.as_deref() == Some(&name) {
                printer.default_paper = printer.papers.len();
            }
            let label = common_paper_label(&name)
                .map(str::to_owned)
                .or_else(|| read_string((cups.media_label)(http, dest, info, 0, &mut media)))
                .unwrap_or_else(|| name.clone());
            let mm = |value| f64::from(value) / 100.0;
            printer.papers.push(Paper {
                media: name,
                label,
                width: mm(media.width),
                height: mm(media.length),
                margins: [media.top, media.bottom, media.left, media.right].map(mm),
            });
        }
        let supports = |key: &CStr, value: &str| {
            CString::new(value).is_ok_and(|value| {
                (cups.check_supported)(http, dest, info, key.as_ptr(), value.as_ptr()) != 0
            })
        };
        let default = |key: &CStr| {
            let attr = (cups.default)(http, dest, info, key.as_ptr());
            (!attr.is_null())
                .then(|| read_string((cups.get_string)(attr, 0, null_mut())))
                .flatten()
        };
        printer.duplex = Duplex::ALL
            .iter()
            .copied()
            .filter(|mode| supports(c"sides", mode.ipp()))
            .collect();
        let sides = default(c"sides");
        printer.default_duplex = printer
            .duplex
            .iter()
            .copied()
            .find(|mode| sides.as_deref() == Some(mode.ipp()))
            .or_else(|| printer.duplex.first().copied());

        let color_mode = |color| if color { "color" } else { "monochrome" };
        printer.color = [false, true]
            .into_iter()
            .filter(|color| supports(c"print-color-mode", color_mode(*color)))
            .collect();
        let color = default(c"print-color-mode");
        printer.default_color = printer
            .color
            .iter()
            .copied()
            .find(|mode| color.as_deref() == Some(color_mode(*mode)))
            // Without a reported default, a colour printer prints in colour.
            .or_else(|| printer.color.last().copied());

        let copies = (cups.supported)(http, dest, info, c"copies".as_ptr());
        if !copies.is_null() {
            let mut max = 0;
            (cups.get_range)(copies, 0, &mut max);
            if max > 0 {
                printer.max_copies = max as u32;
            }
        }
        (cups.free_info)(info);
        Some(printer)
    }
}

pub(super) fn submit(
    selection: &Selection,
    title: &str,
    document: OwnedFd,
    prepared: bool,
) -> Result<u32, String> {
    let document = File::from(document);
    use rustix::fs::{OFlags, fcntl_getfl};
    let access = fcntl_getfl(&document).map_err(|e| e.to_string())?;
    if access.contains(OFlags::PATH) || (access & OFlags::RWMODE) == OFlags::WRONLY {
        return Err("The print document is not open for reading.".into());
    }
    if !document.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("The print document must be a regular file.".into());
    }
    // libcups opens the document by path. This path refers only to the descriptor supplied by D-Bus.
    let path = CString::new(format!("/proc/self/fd/{}", document.as_raw_fd()))
        .map_err(|e| e.to_string())?;
    let title = CString::new(if title.is_empty() {
        "Print document"
    } else {
        title
    })
    .map_err(|e| e.to_string())?;
    let queue = CString::new(selection.queue()).map_err(|e| e.to_string())?;
    let options = selection
        .cups_options(prepared)
        .into_iter()
        .map(|(key, value)| Ok((CString::new(key)?, CString::new(value)?)))
        .collect::<Result<Vec<_>, std::ffi::NulError>>()
        .map_err(|e| e.to_string())?;
    let pairs: Vec<_> = options
        .iter()
        .map(|(name, value)| OptionPair {
            name: name.as_ptr(),
            value: value.as_ptr(),
        })
        .collect();
    let cups = Cups::get()?;
    let connection = cups.connect()?;
    // SAFETY: All strings and the descriptor stay alive until the blocking call returns.
    let job = unsafe {
        (cups.print_file)(
            connection.http,
            queue.as_ptr(),
            path.as_ptr(),
            title.as_ptr(),
            pairs.len() as c_int,
            pairs.as_ptr(),
        )
    };
    if job > 0 {
        Ok(job as u32)
    } else {
        Err(cups.error())
    }
}

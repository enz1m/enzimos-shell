use crate::{
    background::{self, Picture},
    handle,
    ui::Images,
};
use enzui_shell::i18n;
use enzui_shell::{
    desktop_preferences::{Wallpaper, WallpaperKind, valid_scene_id},
    session_preferences::LockClock,
};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread::JoinHandle,
    time::{Duration, Instant},
};

struct Frame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    flags: u32,
}
pub(crate) struct Live {
    child: Child,
    reader: Option<JoinHandle<()>>,
    frames: Receiver<Frame>,
    started: Instant,
    received: bool,
    last_frame: Instant,
    light: Option<bool>,
    /// Readable when a frame has arrived, so the loop need not poll for one.
    wake: std::os::unix::net::UnixStream,
}
fn binary() -> Option<PathBuf> {
    std::env::var_os("ENZUI_SHELL_BIN_DIR")
        .map(PathBuf::from)
        .map(|d| d.join("enzwall"))
        .filter(|p| p.is_absolute() && p.is_file())
        .or_else(|| {
            [
                "/usr/lib/enzui/bin/enzwall",
                "/usr/libexec/enzui/enzwall",
                "/usr/bin/enzwall",
            ]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
        })
}
fn scene(wallpaper: &Wallpaper) -> Option<PathBuf> {
    let id = wallpaper.scene.as_deref().filter(|id| valid_scene_id(id))?;
    let root = std::env::var_os("ENZUI_DESKTOP_ROOT")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| "/usr/share/enzui".into());
    let scenes = root.join("enzwall/scenes").canonicalize().ok()?;
    let scene = scenes.join(id).canonicalize().ok()?;
    (scene.parent() == Some(scenes.as_path()) && scene.join("scene.toml").is_file())
        .then_some(scene)
}
impl Live {
    pub(crate) fn start(
        wallpaper: &Wallpaper,
        prefs: &LockClock,
        size: (u32, u32),
        reduced_motion: bool,
    ) -> Option<Self> {
        if wallpaper.kind != WallpaperKind::Live || prefs.background.is_some() {
            return None;
        }
        let scene = scene(wallpaper)?;
        let binary = binary()?.canonicalize().ok()?;
        let shared = scene.parent().and_then(|parent| {
            parent
                .join("_shared/fonts")
                .canonicalize()
                .ok()
                .filter(|p| p.starts_with(parent))
        });
        if !Path::new("/usr/bin/bwrap").is_file() || !Path::new("/usr/bin/prlimit").is_file() {
            return None;
        }
        let scale = (960.0 / size.0.max(1) as f32)
            .min(540.0 / size.1.max(1) as f32)
            .min(1.0);
        let dims = (
            (size.0 as f32 * scale).round().max(16.0) as u32,
            (size.1 as f32 * scale).round().max(16.0) as u32,
        );
        let mut c = Command::new("/usr/bin/bwrap");
        c.env_clear()
            .env("ENZUI_LOCALE", i18n::current().locale().to_string())
            .args([
                "--die-with-parent",
                "--new-session",
                "--unshare-all",
                "--ro-bind",
                "/usr",
                "/usr",
            ]);
        for dir in ["/lib", "/lib64"] {
            if Path::new(dir).exists() {
                c.args(["--ro-bind", dir, dir]);
            }
        }
        c.args([
            "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp", "--dir", "/run", "--dir", "/etc",
        ]);
        for p in ["/etc/fonts", "/etc/localtime", "/etc/ld.so.cache"] {
            if Path::new(p).exists() {
                c.args(["--ro-bind", p, p]);
            }
        }
        if Path::new("/dev/dri").is_dir() {
            c.args(["--dev-bind", "/dev/dri", "/dev/dri"]);
            // Mesa needs read-only sysfs for DRM device discovery; omitting it
            // silently selects software rendering despite an available render node.
            if Path::new("/sys").is_dir() {
                c.args(["--ro-bind", "/sys", "/sys"]);
            }
        }
        if let Some(shared) = shared {
            c.args(["--dir", "/_shared", "--ro-bind"])
                .arg(shared)
                .arg("/_shared/fonts");
        }
        c.args(["--ro-bind"])
            .arg(scene)
            .arg("/scene")
            .args(["--ro-bind"])
            .arg(binary)
            .arg("/enzwall");
        c.args([
            "--setenv",
            "HOME",
            "/tmp",
            "--setenv",
            "XDG_RUNTIME_DIR",
            "/run",
            "--setenv",
            "PATH",
            "/usr/bin",
            "--setenv",
            "EGL_PLATFORM",
            "surfaceless",
            "--setenv",
            "LP_NUM_THREADS",
            "1",
            "--setenv",
            "MESA_SHADER_CACHE_DISABLE",
            "true",
            "--chdir",
            "/scene",
            "/usr/bin/prlimit",
            "--as=8589934592",
            "--nofile=128",
            "--data=536870912",
            "--core=0",
            "--",
            "/enzwall",
            "stream",
            "/scene",
            "--size",
            &format!("{}x{}", dims.0, dims.1),
            "--fps",
            if reduced_motion { "1" } else { "15" },
        ]);
        if prefs.hide_wallpaper_clock || wallpaper.hide_clock {
            c.arg("--hide-clock");
        }
        if prefs.hide_wallpaper_date || wallpaper.hide_date {
            c.arg("--hide-date");
        }
        if reduced_motion {
            c.arg("--static");
        }
        let mut child = c
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        let (tx, frames) = mpsc::sync_channel(1);
        let (wake, signal) = std::os::unix::net::UnixStream::pair().ok()?;
        wake.set_nonblocking(true).ok()?;
        signal.set_nonblocking(true).ok()?;
        let reader = std::thread::spawn(move || {
            use std::io::Write as _;
            let mut header = [0_u8; 20];
            if stdout.read_exact(&mut header).is_err() || &header[..8] != b"ENZWALL1" {
                return;
            }
            let u32at = |i| u32::from_le_bytes(header[i..i + 4].try_into().expect("fixed header"));
            let (w, h, flags) = (u32at(8), u32at(12), u32at(16));
            if (w, h) != dims || flags > 3 {
                return;
            }
            loop {
                let mut rgba = vec![0_u8; w as usize * h as usize * 4];
                if stdout.read_exact(&mut rgba).is_err() {
                    break;
                }
                match tx.try_send(Frame {
                    width: w,
                    height: h,
                    rgba,
                    flags,
                }) {
                    Ok(()) => {
                        let _ = (&signal).write(&[1]);
                    }
                    Err(mpsc::TrySendError::Full(_)) => {}
                    Err(mpsc::TrySendError::Disconnected(_)) => break,
                }
            }
        });
        Some(Self {
            child,
            reader: Some(reader),
            frames,
            started: Instant::now(),
            received: false,
            last_frame: Instant::now(),
            light: None,
            wake,
        })
    }
    /// Becomes readable when a frame arrives.
    pub(crate) fn wakes(&self) -> std::os::fd::BorrowedFd<'_> {
        use std::os::fd::AsFd as _;
        self.wake.as_fd()
    }
    pub(crate) fn frame(&mut self) -> Option<Images> {
        let mut drained = [0_u8; 64];
        while (&self.wake).read(&mut drained).is_ok_and(|n| n > 0) {}
        let frame = self.frames.try_iter().last()?;
        self.received = true;
        self.last_frame = Instant::now();
        let sharp = raster::RgbaImage::from_raw(frame.width, frame.height, frame.rgba)?;
        let soft = background::soften(&sharp);
        let luma = background::luma(&sharp);
        let light = match self.light {
            None => luma > 0.65,
            Some(true) => luma > 0.55,
            Some(false) => luma > 0.75,
        };
        self.light = Some(light);
        Some(Images {
            desktop: None,
            wallpaper: handle(Picture {
                width: frame.width,
                height: frame.height,
                rgba: sharp.into_raw(),
            }),
            soft: handle(soft),
            has_clock: frame.flags & 1 != 0,
            has_date: frame.flags & 2 != 0,
            light,
        })
    }
    pub(crate) fn healthy(&mut self) -> bool {
        self.child.try_wait().is_ok_and(|r| r.is_none())
            && ((self.received && self.last_frame.elapsed() < Duration::from_secs(8))
                || (!self.received && self.started.elapsed() < Duration::from_secs(8)))
    }
}
impl Drop for Live {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

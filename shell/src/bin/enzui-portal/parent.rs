use iced::{Task, window};
use std::sync::Arc;
use wayland_client::{
    Connection, EventQueue, Proxy,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_registry, wl_surface::WlSurface},
};
use wayland_protocols::xdg::{
    dialog::v1::client::{xdg_dialog_v1::XdgDialogV1, xdg_wm_dialog_v1::XdgWmDialogV1},
    foreign::zv2::client::{zxdg_imported_v2::ZxdgImportedV2, zxdg_importer_v2::ZxdgImporterV2},
    shell::client::xdg_toplevel::XdgToplevel,
};

#[derive(Debug)]
struct State;

wayland_client::delegate_noop!(State: ignore ZxdgImporterV2);
wayland_client::delegate_noop!(State: ignore ZxdgImportedV2);
wayland_client::delegate_noop!(State: ignore XdgWmDialogV1);
wayland_client::delegate_noop!(State: ignore XdgDialogV1);
impl wayland_client::Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
    }
}

type WindowObjects = (Connection, WlSurface, u64);

pub(super) struct ParentLease {
    connection: Connection,
    imported: ZxdgImportedV2,
    modal: Option<XdgDialogV1>,
    _queue: EventQueue<State>,
}

impl std::fmt::Debug for ParentLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParentLease")
            .field("imported", &self.imported)
            .finish()
    }
}

impl Drop for ParentLease {
    fn drop(&mut self) {
        if let Some(modal) = &self.modal {
            modal.destroy();
        }
        self.imported.destroy();
        let _ = self.connection.flush();
    }
}

fn attach(
    (connection, surface, _): WindowObjects,
    handle: String,
    toplevel: Option<XdgToplevel>,
) -> Option<ParentLease> {
    let (globals, queue) = registry_queue_init::<State>(&connection).ok()?;
    if !surface.is_alive() {
        return None;
    }
    let qh = queue.handle();
    let importer: ZxdgImporterV2 = globals.bind(&qh, 1..=1, ()).ok()?;
    let imported = importer.import_toplevel(handle, &qh, ());
    importer.destroy();
    imported.set_parent_of(&surface);
    let modal = toplevel.filter(Proxy::is_alive).and_then(|toplevel| {
        let wm_dialog: XdgWmDialogV1 = globals.bind(&qh, 1..=1, ()).ok()?;
        let dialog = wm_dialog.get_xdg_dialog(&toplevel, &qh, ());
        wm_dialog.destroy();
        dialog.set_modal();
        Some(dialog)
    });
    let _ = connection.flush();
    Some(ParentLease {
        connection,
        imported,
        modal,
        _queue: queue,
    })
}

pub(super) fn connect(
    window: window::Id,
    parent: String,
    modal: bool,
) -> Task<Option<Arc<ParentLease>>> {
    if let Some(handle) = parent
        .strip_prefix("wayland:")
        .filter(|handle| !handle.is_empty())
    {
        let handle = handle.to_owned();
        return window::wayland_parent_export(window).then(move |objects| {
            let Some(objects) =
                objects.and_then(|objects| objects.downcast::<WindowObjects>().ok())
            else {
                return Task::done(None);
            };
            let handle = handle.clone();
            if modal {
                window::wayland_toplevel(window).then(move |toplevel| {
                    let toplevel = toplevel
                        .and_then(|toplevel| toplevel.downcast::<XdgToplevel>().ok())
                        .map(|toplevel| *toplevel);
                    attach_task((*objects).clone(), handle.clone(), toplevel)
                })
            } else {
                attach_task(*objects, handle, None)
            }
        });
    }
    #[cfg(feature = "x11")]
    if let Some(xid) = parent
        .strip_prefix("x11:")
        .and_then(|id| u32::from_str_radix(id, 16).ok())
    {
        return window::run(window, move |window| set_x11_parent(window, xid, modal));
    }
    Task::done(None)
}

fn attach_task(
    objects: WindowObjects,
    handle: String,
    toplevel: Option<XdgToplevel>,
) -> Task<Option<Arc<ParentLease>>> {
    Task::future(async move {
        tokio::task::spawn_blocking(move || attach(objects, handle, toplevel))
            .await
            .ok()
            .flatten()
            .map(Arc::new)
    })
}

#[cfg(feature = "x11")]
fn set_x11_parent(
    window: &dyn iced::window::Window,
    parent: u32,
    modal: bool,
) -> Option<Arc<ParentLease>> {
    use iced::window::raw_window_handle::RawWindowHandle;
    use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask};
    use x11rb::wrapper::ConnectionExt as _;
    let child = match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(window) => u32::try_from(window.window).ok()?,
        RawWindowHandle::Xcb(window) => window.window.get(),
        _ => return None,
    };
    let (connection, screen) = x11rb::connect(None).ok()?;
    connection
        .change_property32(
            x11rb::protocol::xproto::PropMode::REPLACE,
            child,
            AtomEnum::WM_TRANSIENT_FOR,
            AtomEnum::WINDOW,
            &[parent],
        )
        .ok()?
        .check()
        .ok()?;
    if modal {
        let state = connection
            .intern_atom(false, b"_NET_WM_STATE")
            .ok()?
            .reply()
            .ok()?
            .atom;
        let modal = connection
            .intern_atom(false, b"_NET_WM_STATE_MODAL")
            .ok()?
            .reply()
            .ok()?
            .atom;
        let event = ClientMessageEvent::new(32, child, state, [1, modal, 0, 1, 0]);
        use x11rb::connection::Connection as _;
        connection
            .send_event(
                false,
                connection.setup().roots.get(screen)?.root,
                EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
                event,
            )
            .ok()?
            .check()
            .ok()?;
    }
    None
}

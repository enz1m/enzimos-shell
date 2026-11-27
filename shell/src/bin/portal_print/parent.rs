use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop, globals};
use wayland_protocols::xdg::{
    dialog::v1::client::{xdg_dialog_v1::XdgDialogV1, xdg_wm_dialog_v1::XdgWmDialogV1},
    foreign::zv2::client::{zxdg_imported_v2::ZxdgImportedV2, zxdg_importer_v2::ZxdgImporterV2},
};

use super::Surface;

pub(crate) struct ParentProtocol {
    queue: EventQueue<State>,
    importer: ZxdgImporterV2,
    dialog_manager: Option<XdgWmDialogV1>,
}

impl ParentProtocol {
    fn new(connection: &Connection) -> Option<Self> {
        let (globals, queue) = globals::registry_queue_init(connection).ok()?;
        let handle = queue.handle();
        let importer = globals.bind(&handle, 1..=1, ()).ok()?;
        let dialog_manager = globals.bind(&handle, 1..=1, ()).ok();
        globals.destroy();
        Some(Self {
            queue,
            importer,
            dialog_manager,
        })
    }
}

#[derive(Debug)]
pub(super) struct Parent {
    imported: ZxdgImportedV2,
    dialog: Option<XdgDialogV1>,
    connection: Connection,
}

struct State;

impl Dispatch<wayland_client::protocol::wl_registry::WlRegistry, globals::GlobalListContents>
    for State
{
    fn event(
        _: &mut Self,
        _: &wayland_client::protocol::wl_registry::WlRegistry,
        _: wayland_client::protocol::wl_registry::Event,
        _: &globals::GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

delegate_noop!(State: ignore ZxdgImporterV2);
delegate_noop!(State: ignore ZxdgImportedV2);
delegate_noop!(State: ignore XdgWmDialogV1);
delegate_noop!(State: ignore XdgDialogV1);

impl Parent {
    pub(super) fn attach(
        protocol: &mut Option<ParentProtocol>,
        surface: Surface,
        parent: &str,
        modal: bool,
    ) -> Option<Self> {
        let handle = parent
            .strip_prefix("wayland-")
            .filter(|handle| !handle.is_empty() && !handle.contains('\0'))?;
        if protocol.is_none() {
            *protocol = ParentProtocol::new(&surface.connection);
        }
        let protocol = protocol.as_mut()?;
        let _ = protocol.queue.dispatch_pending(&mut State);
        let queue = protocol.queue.handle();
        let imported = protocol
            .importer
            .import_toplevel(handle.to_owned(), &queue, ());
        imported.set_parent_of(&surface.surface);
        let dialog = if modal {
            surface
                .toplevel
                .zip(protocol.dialog_manager.as_ref())
                .map(|(toplevel, manager)| {
                    let dialog = manager.get_xdg_dialog(&toplevel, &queue, ());
                    dialog.set_modal();
                    dialog
                })
        } else {
            None
        };
        let parent = Self {
            imported,
            dialog,
            connection: surface.connection,
        };
        let _ = parent.connection.flush();
        Some(parent)
    }
}

impl Drop for Parent {
    fn drop(&mut self) {
        if let Some(dialog) = &self.dialog {
            dialog.destroy();
        }
        self.imported.destroy();
        let _ = self.connection.flush();
    }
}

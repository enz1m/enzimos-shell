use super::{
    model::{LayoutRequest, Live, Message, Pane, Target, ThemeClaim},
    state::{message, requested_layout},
};
use crate::{Message as ShellMessage, Shell, window_settings};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_shell::{
    compositor::PopupPlacement,
    plugins::{
        protocol::{Request, ShellView, Snapshot},
        service::{Command, Event, Owner},
        theme::Colors,
    },
};
use iced::{Task, window};
use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

impl Shell {
    pub(super) fn reconcile_plugin_layout(&mut self) -> Task<ShellMessage> {
        if self.plugin.layout_busy {
            self.plugin.layout_dirty = true;
            return Task::none();
        }
        let Some(settings) = self.settings_store.clone() else {
            return Task::none();
        };
        self.plugin.layout_busy = true;
        self.plugin.layout_dirty = false;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    enzui_shell::plugins::store::Store::for_settings(settings)?
                        .reconcile_layout(false)
                        .map(|_| ())
                })
                .await
                .map_err(|_| tr("shell-plugin-layout-writer-ended").to_owned())?
            },
            |result| message(Message::LayoutSaved(result)),
        )
    }
    pub(super) fn plugin_layout_request(&mut self, request: LayoutRequest) -> Task<ShellMessage> {
        if self.plugin.layout_requests.len() >= 16 {
            self.plugin_reply(
                request.owner,
                request.id,
                false,
                tr("shell-layout-broker-is-busy"),
            );
            return Task::none();
        }
        self.plugin.layout_requests.push_back(request);
        self.plugin_layout_request_next()
    }
    pub(super) fn plugin_layout_request_next(&mut self) -> Task<ShellMessage> {
        if self.plugin.layout_request_busy {
            return Task::none();
        }
        let Some(request) = self.plugin.layout_requests.pop_front() else {
            return Task::none();
        };
        let Some(settings) = self.settings_store.clone() else {
            self.plugin_reply(
                request.owner,
                request.id,
                false,
                tr("shell-layout-broker-is-unavailable"),
            );
            return self.plugin_layout_request_next();
        };
        self.plugin.layout_request_busy = true;
        let owner = request.owner.clone();
        let response_owner = owner.clone();
        let id = request.id;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    enzui_shell::plugins::store::Store::for_settings(settings)?
                        .transform_layout_for(&owner.id, owner.activation, |current| {
                            requested_layout(current, &request)
                        })
                        .map(|_| ())
                })
                .await
                .map_err(|_| tr("shell-layout-broker-writer-ended").to_owned())?
            },
            move |result| {
                message(Message::LayoutRequestSaved {
                    owner: response_owner,
                    id,
                    result,
                })
            },
        )
    }
    pub(super) fn plugin_theme(
        &mut self,
        owner: Owner,
        colors: Colors,
        authority: Arc<AtomicBool>,
    ) -> Result<(), String> {
        if !authority.load(Ordering::Acquire) {
            return Err(tr("shell-plugin-theme-authority-changed").into());
        }
        if colors.is_empty() {
            if self
                .plugin
                .theme
                .as_ref()
                .is_some_and(|claim| claim.owner == owner)
            {
                self.plugin.theme = None;
            }
        } else {
            if self.plugin.theme.as_ref().is_some_and(|claim| {
                claim.owner != owner && claim.authority.load(Ordering::Acquire)
            }) {
                return Err(tr("shell-another-plugin-owns-the-shell-theme").into());
            }
            let _ = colors.apply(&self.base_tokens())?;
            self.plugin.theme = Some(ThemeClaim {
                owner,
                colors,
                authority,
            });
        }
        self.update_frame();
        Ok(())
    }
    pub(crate) fn plugin_command(&mut self, command: Command) {
        if self
            .plugin
            .channel
            .as_ref()
            .is_none_or(|c| c.try_send(command).is_err())
        {
            self.plugin.error = Some(tr("shell-plugin-service-is-unavailable-or-busy").into());
        }
    }
    pub(crate) fn plugin_reply(&mut self, owner: Owner, id: u64, ok: bool, error: &str) {
        self.plugin_command(Command::Reply {
            owner,
            id,
            result: serde_json::json!({"ok":ok,"error":error}),
        });
    }
    pub(crate) fn consume_plugin_gesture(&mut self, owner: &Owner, token: &str) -> bool {
        self.plugin
            .gestures
            .retain(|_, (_, at)| at.elapsed() < Duration::from_secs(2));
        self.plugin
            .gestures
            .remove(token)
            .is_some_and(|(expected, _)| expected == *owner)
    }
    pub(crate) fn plugin_message(&mut self, event: Message) -> Task<ShellMessage> {
        match event {
            Message::Clipboard(event) => self.clipboard_message(event),
            Message::NotificationTick => {
                let error = self.plugin.error.clone();
                let task = self.plugin_notifications_publish();
                self.quiet = self.plugin.error == error && task.units() == 0;
                task
            }
            Message::Event(Event::Connected(channel)) => {
                self.plugin.channel = Some(channel);
                self.plugin.error = None;
                Task::none()
            }
            Message::Event(Event::Available) => {
                self.plugin.error = None;
                Task::none()
            }
            Message::Event(Event::Unavailable(error)) => {
                self.plugin.error = Some(error);
                for live in self.plugin.live.values() {
                    live.authority.store(false, Ordering::Release);
                }
                self.plugin.live.clear();
                self.clipboard_clear();
                self.plugin.notifications.clear();
                self.plugin.catalog_serial = None;
                self.plugin.layout_requests.clear();
                self.plugin.gestures.clear();
                self.plugin.theme = None;
                self.update_frame();
                self.force_close_plugin()
            }
            Message::Event(Event::Catalog(serial)) => {
                if self.plugin.catalog_serial == Some(serial) {
                    self.quiet = true;
                    return Task::none();
                }
                self.plugin.catalog_serial = Some(serial);
                self.reconcile_plugin_layout()
            }
            Message::Event(Event::Started {
                owner,
                manifest,
                authority,
            }) => {
                if !authority.load(Ordering::Acquire) {
                    return Task::none();
                }
                if self
                    .plugin
                    .theme
                    .as_ref()
                    .is_some_and(|claim| claim.owner.id == owner.id)
                {
                    self.plugin.theme = None;
                }
                let live = Live {
                    owner: owner.clone(),
                    manifest,
                    snapshot: Arc::new(Snapshot {
                        revision: 0,
                        elements: Vec::new(),
                    }),
                    authority,
                };
                if let Some(old) = self.plugin.live.insert(owner.id, live) {
                    old.authority.store(false, Ordering::Release);
                    self.clipboard_revoke(&old.owner);
                    self.application_surface_revoke(&old.owner);
                    self.plugin.notifications.revoke(&old.owner);
                }
                self.update_frame();
                Task::none()
            }
            Message::Event(Event::Fault { owner, fault }) => {
                self.plugin.error = Some(
                    tr!("shell-plugin-stopped", "arg0" => owner.id, "arg1" => format!("{:}", fault)),
                );
                Task::none()
            }
            Message::Event(Event::Removed(owner)) => {
                if self
                    .plugin
                    .live
                    .get(&owner.id)
                    .is_some_and(|l| l.owner == owner)
                    && let Some(live) = self.plugin.live.remove(&owner.id)
                {
                    live.authority.store(false, Ordering::Release);
                }
                self.clipboard_revoke(&owner);
                self.application_surface_revoke(&owner);
                self.plugin.notifications.revoke(&owner);
                self.plugin.gestures.retain(|_, (o, _)| *o != owner);
                self.plugin.layout_requests.retain(|r| r.owner != owner);
                if self
                    .plugin
                    .pending
                    .as_ref()
                    .is_some_and(|target| target.owner == owner)
                {
                    self.plugin.pending = None;
                }
                if self
                    .plugin
                    .theme
                    .as_ref()
                    .is_some_and(|claim| claim.owner == owner)
                {
                    self.plugin.theme = None;
                }
                self.update_frame();
                if self.plugin.pane.as_ref().is_some_and(|p| p.owner == owner)
                    || self
                        .plugin
                        .preparing
                        .as_ref()
                        .is_some_and(|(_, t)| t.owner == owner)
                {
                    self.force_close_plugin()
                } else {
                    Task::none()
                }
            }
            Message::Event(Event::Snapshot {
                owner,
                manifest,
                snapshot,
            }) => {
                let Some(authority) = self
                    .plugin
                    .live
                    .get(&owner.id)
                    .filter(|live| live.owner == owner)
                    .map(|live| std::sync::Arc::clone(&live.authority))
                else {
                    return Task::none();
                };
                let mut close = false;
                if let Some(pane) = self.plugin.pane.as_mut()
                    && pane.owner == owner
                    && (pane.visible || !pane.started)
                {
                    if let Some(panel) = snapshot
                        .elements
                        .iter()
                        .find(|e| e.id == pane.element)
                        .and_then(|e| e.panel.as_ref())
                    {
                        if panel.footer.is_some() != pane.view.footer.is_some()
                            || panel.entry_list != pane.view.entry_list
                        {
                            pane.layout_epoch = pane.layout_epoch.wrapping_add(1);
                        }
                        pane.view = Arc::new(panel.clone());
                        pane.revision = snapshot.revision;
                    } else {
                        close = true;
                    }
                }
                self.plugin.live.insert(
                    owner.id.clone(),
                    Live {
                        owner,
                        manifest,
                        snapshot,
                        authority,
                    },
                );
                if close {
                    self.force_close_plugin()
                } else {
                    self.reflow_plugin()
                }
            }
            Message::Event(Event::Request { owner, id, request }) => {
                let Some(authority) = self
                    .plugin
                    .live
                    .get(&owner.id)
                    .filter(|live| live.owner == owner && live.authority.load(Ordering::Acquire))
                    .map(|live| std::sync::Arc::clone(&live.authority))
                else {
                    return Task::none();
                };
                match request {
                    Request::ApplicationDock { element } => {
                        self.application_surface_request(owner, id, element, authority)
                    }
                    Request::ClipboardSubscribe(enabled) => {
                        self.clipboard_subscribe(owner, id, enabled, authority)
                    }
                    Request::NotificationSubscribe(enabled) => {
                        self.plugin_notification_subscribe(owner, id, enabled, authority)
                    }
                    Request::ClipboardRestore { blob, gesture } => {
                        self.clipboard_restore(owner, id, &blob, &gesture, authority)
                    }
                    Request::ReleaseBlobs(handles) => self.clipboard_release(owner, id, &handles),
                    Request::Theme(colors) => {
                        let result = Colors::new(colors)
                            .and_then(|colors| self.plugin_theme(owner.clone(), colors, authority));
                        match result {
                            Ok(()) => self.plugin_reply(owner, id, true, ""),
                            Err(error) => self.plugin_reply(owner, id, false, &error),
                        }
                        Task::none()
                    }
                    Request::Layout {
                        element,
                        section,
                        before,
                        enabled,
                    } => self.plugin_layout_request(LayoutRequest {
                        owner,
                        id,
                        element,
                        section,
                        before,
                        enabled,
                        authority,
                    }),
                    Request::Open { view, gesture }
                        if self.consume_plugin_gesture(&owner, &gesture) =>
                    {
                        self.plugin_reply(owner, id, true, "");
                        let next = match view {
                            ShellView::Applications => ShellMessage::ShowLauncher,
                            ShellView::Desktops => ShellMessage::ToggleOverview,
                            ShellView::Controls => ShellMessage::ToggleControls,
                            ShellView::Settings => ShellMessage::SettingsPage("appearance"),
                        };
                        if let Some(pane) = &self.plugin.pane {
                            self.queue_panel_action(pane.window, next)
                        } else {
                            Task::done(next)
                        }
                    }
                    _ => {
                        self.plugin_reply(
                            owner,
                            id,
                            false,
                            tr("shell-operation-broker-is-not-available"),
                        );
                        Task::none()
                    }
                }
            }
            Message::Toggle(qualified) => {
                self.cancel_panel_action();
                let Some((plugin, element)) = qualified
                    .strip_prefix("plugin:")
                    .and_then(|s| s.split_once('/'))
                else {
                    return Task::none();
                };
                let Some(live) = self.plugin.live.get(plugin) else {
                    return Task::none();
                };
                let target = Target {
                    owner: live.owner.clone(),
                    element: element.to_owned(),
                    placement: None,
                };
                if self.plugin_target(&target).is_none() {
                    return Task::none();
                }
                if let Some(pane) = self.plugin.pane.as_mut()
                    && pane.owner == target.owner
                    && pane.element == target.element
                {
                    if pane.visible
                        || (self.metrics.islands.is_some()
                            && !self.surface_inputs.contains_key(&pane.window))
                    {
                        return self.close_plugin();
                    }
                    pane.visible = true;
                    let compositor = self.compositor.clone();
                    return Task::perform(
                        async move { compositor.focus_plugin_panel().await },
                        ShellMessage::FocusReturned,
                    );
                }
                let close = self.force_close_plugin();
                self.plugin.pending = Some(target);
                Task::batch([
                    close,
                    self.close_controls_for_panel(),
                    self.force_close_overview(),
                    self.force_close_launcher(),
                    self.plugin_prepare_next(),
                ])
            }
            Message::Anchor(serial, anchor) => {
                let Some((current, target)) = self.plugin.preparing.as_ref() else {
                    return Task::none();
                };
                if *current != serial {
                    return Task::none();
                }
                let Some((_, panel)) = self.plugin_target(target) else {
                    return Task::none();
                };
                let Some(anchor) = anchor else {
                    self.plugin.preparing = None;
                    return Task::none();
                };
                let body = iced::Size::new(f32::from(panel.width), f32::from(panel.height));
                self.plugin
                    .preparing
                    .as_mut()
                    .expect("this branch updates the current preparation")
                    .1
                    .placement = Some((anchor, body));
                let (left, right) = self.desktop_edges();
                let placement = PopupPlacement {
                    bar: self.bar,
                    left,
                    right,
                    metrics: self.metrics,
                    anchor: Some(anchor),
                    body,
                };
                self.plugin.placement_busy = Some(serial);
                let compositor = self.compositor.clone();
                Task::perform(
                    async move { compositor.prepare_plugin_panel(placement).await },
                    move |result| message(Message::Prepared(serial, result)),
                )
            }
            Message::Prepared(serial, result) => {
                if self.plugin.placement_busy != Some(serial) {
                    return Task::none();
                }
                self.plugin.placement_busy = None;
                let Some((current, target)) = self.plugin.preparing.as_ref() else {
                    return self.plugin_prepare_next();
                };
                if *current != serial {
                    return self.plugin_prepare_next();
                }
                let Some((live, panel)) = self.plugin_target(target) else {
                    return Task::none();
                };
                let owner = live.owner.clone();
                let manifest = std::sync::Arc::clone(&live.manifest);
                let revision = live.snapshot.revision;
                let element = target.element.clone();
                let Some((anchor, applied_body)) = target.placement else {
                    return Task::none();
                };
                let panel = Arc::new(panel.clone());
                self.plugin.preparing = None;
                match result {
                    Ok(geometry) => {
                        let size = self.popover_window(&geometry);
                        let mut settings =
                            window_settings("dev.enzui.PluginPanel", size.width, size.height);
                        // Fixed Wayland client hints freeze the initial size; compositor rules own the size lock.
                        settings.resizable = true;
                        let (id, open) = window::open(settings);
                        self.plugin.pane = Some(Pane {
                            window: id,
                            owner,
                            element,
                            manifest,
                            view: panel,
                            revision,
                            actual_size: size,
                            geometry,
                            visible: false,
                            started: false,
                            anchor,
                            applied_body,
                            resizing: false,
                            layout_epoch: 0,
                            scroll: Default::default(),
                            geometry_error: None,
                        });
                        open.map(ShellMessage::Opened)
                    }
                    Err(error) => {
                        self.plugin.error = Some(error);
                        Task::none()
                    }
                }
            }
            Message::Reflow(id, owner, body, result) => {
                if self.plugin.reflow != Some(id) {
                    return Task::none();
                }
                self.plugin.reflow = None;
                let bottom_bar = self.bar == enzui_shell::settings::BarEdge::Bottom;
                let Some(pane) = self
                    .plugin
                    .pane
                    .as_mut()
                    .filter(|p| p.window == id && p.owner == owner)
                else {
                    return self.plugin_prepare_next();
                };
                pane.applied_body = body;
                match result {
                    Ok(geometry) => {
                        let m = enzui_shell::reveal::margins(bottom_bar, geometry.attachment);
                        let size = iced::Size::new(
                            geometry.width + m.left + m.right,
                            geometry.height + m.top + m.bottom,
                        );
                        pane.resizing = size != pane.actual_size;
                        pane.geometry = geometry;
                        pane.geometry_error = None;
                        Task::batch([window::resize(id, size), self.reflow_plugin()])
                    }
                    Err(error) => {
                        pane.resizing = false;
                        pane.geometry_error =
                            Some(tr!("shell-panel-resize-failed-reopen-to-retry", "arg0" => error));
                        Task::none()
                    }
                }
            }
            Message::Inspect(id) => {
                if !self
                    .plugin
                    .pane
                    .as_ref()
                    .is_some_and(|p| p.window == id && p.visible && !p.resizing)
                {
                    return Task::none();
                }
                self.queue_panel_action(id, ShellMessage::SettingsPage("plugins"))
            }
            Message::Close => {
                self.cancel_panel_action();
                self.close_plugin()
            }
            Message::Hidden(id) => {
                if self
                    .plugin
                    .pane
                    .as_ref()
                    .is_some_and(|p| p.window == id && !p.visible)
                {
                    self.finish_plugin()
                } else {
                    Task::none()
                }
            }
            Message::Input {
                owner,
                element,
                revision,
                interaction,
            } => {
                if !self.plugin.pane.as_ref().is_some_and(|p| {
                    p.owner == owner
                        && p.element == element
                        && p.revision == revision
                        && p.visible
                        && !p.resizing
                }) {
                    return Task::none();
                }
                let mut random = [0; 16];
                if std::fs::File::open("/dev/urandom")
                    .and_then(|mut f| f.read_exact(&mut random))
                    .is_err()
                {
                    return Task::none();
                }
                self.plugin
                    .gestures
                    .retain(|_, (_, at)| at.elapsed() < Duration::from_secs(2));
                if self.plugin.gestures.len() >= 32 {
                    return Task::none();
                }
                let token = random
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>();
                self.plugin
                    .gestures
                    .insert(token.clone(), (owner.clone(), Instant::now()));
                self.plugin_command(Command::Input {
                    owner,
                    element,
                    revision,
                    action: interaction.id,
                    value: interaction.value,
                    gesture: token,
                });
                Task::none()
            }
            Message::LayoutRequestSaved { owner, id, result } => {
                self.plugin.layout_request_busy = false;
                let reload =
                    if result.is_ok() && !self.settings_save_busy && !self.settings_read_busy {
                        self.read_settings()
                    } else {
                        Task::none()
                    };
                match result {
                    Ok(()) => self.plugin_reply(owner, id, true, ""),
                    Err(error) => self.plugin_reply(owner, id, false, &error),
                }
                Task::batch([reload, self.plugin_layout_request_next()])
            }
            Message::LayoutSaved(result) => {
                self.plugin.layout_busy = false;
                let accepted = match result {
                    Ok(()) if !self.settings_save_busy && !self.settings_read_busy => {
                        self.read_settings()
                    }
                    Ok(()) => Task::none(),
                    Err(error) => {
                        self.plugin.catalog_serial = None;
                        self.plugin.error = Some(error);
                        Task::none()
                    }
                };
                let retry = if self.plugin.layout_dirty {
                    self.reconcile_plugin_layout()
                } else {
                    Task::none()
                };
                Task::batch([accepted, retry])
            }
        }
    }
}

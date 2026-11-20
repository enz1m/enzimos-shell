use super::{
    model::{Live, Message, Target},
    state::{message, widget_id},
};
use crate::{Message as ShellMessage, Shell, region};
use enzui_shell::{
    compositor::PopupPlacement,
    plugins::{
        protocol::Panel,
        service::{self},
        view,
    },
    settings::BarEdge,
};
use iced::{Element, Length, Task, window};

impl Shell {
    pub(crate) fn plugin_subscription(&self) -> iced::Subscription<ShellMessage> {
        if !enzui_shell::desktop_session::mode().managed() {
            return iced::Subscription::none();
        }
        let service = self
            .settings_store
            .as_ref()
            .and_then(|s| s.path().parent())
            .map_or_else(iced::Subscription::none, |root| {
                service::subscription(root.join("plugins")).map(|e| message(Message::Event(e)))
            });
        let phone =
            enzimos_phone_client::socket_path().map_or_else(iced::Subscription::none, |path| {
                iced::Subscription::run_with(path, |path| {
                    crate::phone_clipboard::stream(path.as_path())
                })
                .map(|event| ShellMessage::Plugin(super::model::Message::Clipboard(event)))
            });
        iced::Subscription::batch([
            service,
            phone,
            self.clipboard_subscription(),
            self.plugin_notification_subscription(),
        ])
    }
    pub(crate) fn plugin_bar(&self, qualified: &str) -> Option<Element<'_, ShellMessage>> {
        let (plugin, element) = qualified.strip_prefix("plugin:")?.split_once('/')?;
        let live = self.plugin.live.get(plugin)?;
        let view = live.snapshot.elements.iter().find(|e| e.id == element)?;
        Some(view::bar(
            view,
            &live.manifest.name,
            widget_id(qualified),
            self.metrics,
            message(Message::Toggle(qualified.to_owned())),
        ))
    }
    pub(super) fn plugin_target(&self, target: &Target) -> Option<(&Live, &Panel)> {
        let live = self
            .plugin
            .live
            .get(&target.owner.id)
            .filter(|l| l.owner == target.owner)?;
        let panel = live
            .snapshot
            .elements
            .iter()
            .find(|e| e.id == target.element)?
            .panel
            .as_ref()?;
        Some((live, panel))
    }
    pub(super) fn plugin_prepare_next(&mut self) -> Task<ShellMessage> {
        // Placement targets this shell's panel class, so replacement waits for in-flight placement.
        if self.plugin.retiring.is_some()
            || self.plugin.pane.is_some()
            || self.plugin.reflow.is_some()
            || self.plugin.placement_busy.is_some()
        {
            return Task::none();
        }
        let Some(target) = self.plugin.pending.take() else {
            return Task::none();
        };
        if self.plugin_target(&target).is_none() {
            return Task::none();
        }
        self.plugin.serial = self.plugin.serial.wrapping_add(1);
        let serial = self.plugin.serial;
        let id = widget_id(&format!("plugin:{}/{}", target.owner.id, target.element));
        self.plugin.preparing = Some((serial, target));
        enzui_shell::widget_bounds::bounds(self.panel, id)
            .map(move |anchor| message(Message::Anchor(serial, anchor)))
    }
    pub(super) fn reflow_plugin(&mut self) -> Task<ShellMessage> {
        let Some(pane) = self.plugin.pane.as_mut() else {
            return Task::none();
        };
        let body = iced::Size::new(f32::from(pane.view.width), f32::from(pane.view.height));
        if body == pane.applied_body
            || !pane.started
            || !pane.visible
            || pane.resizing
            || self.plugin.placement_busy.is_some()
            || self.plugin.reflow.is_some()
        {
            return Task::none();
        }
        let id = pane.window;
        let owner = pane.owner.clone();
        let anchor = pane.anchor;
        pane.resizing = true;
        pane.layout_epoch = pane.layout_epoch.wrapping_add(1);
        self.plugin.reflow = Some(id);
        let (left, right) = self.desktop_edges();
        let placement = PopupPlacement {
            bar: self.bar,
            left,
            right,
            metrics: self.metrics,
            anchor: Some(anchor),
            body,
        };
        let compositor = self.compositor.clone();
        Task::perform(
            async move { compositor.prepare_plugin_panel(placement).await },
            move |result| message(Message::Reflow(id, owner.clone(), body, result)),
        )
    }
    pub(crate) fn close_plugin(&mut self) -> Task<ShellMessage> {
        self.plugin.pending = None;
        self.plugin.preparing = None;
        if let Some(pane) = self.plugin.pane.as_mut() {
            pane.visible = false;
            if pane.started {
                return Task::none();
            }
        }
        self.finish_plugin()
    }
    pub(crate) fn force_close_plugin(&mut self) -> Task<ShellMessage> {
        if let Some(pane) = &self.plugin.pane {
            self.cancel_panel_action_for(pane.window);
        }
        self.finish_plugin()
    }
    pub(super) fn finish_plugin(&mut self) -> Task<ShellMessage> {
        self.plugin.pending = None;
        self.plugin.preparing = None;
        self.plugin.serial = self.plugin.serial.wrapping_add(1);
        if let Some(pane) = self.plugin.pane.take() {
            self.plugin.retiring = Some(pane.window);
            self.finish_panel_close(pane.window)
        } else {
            Task::none()
        }
    }
    pub(crate) fn plugin_window(
        &mut self,
        id: window::Id,
        event: &window::Event,
    ) -> Option<Task<ShellMessage>> {
        if self.plugin.retiring == Some(id) && matches!(event, window::Event::Closed) {
            self.plugin.retiring = None;
            return Some(self.plugin_prepare_next());
        }
        if !self.plugin.pane.as_ref().is_some_and(|p| p.window == id) {
            return None;
        }
        Some(match event {
            window::Event::Focused => {
                if let Some(pane) = self.plugin.pane.as_mut()
                    && !pane.started
                {
                    pane.started = true;
                    pane.visible = true;
                }
                self.reflow_plugin()
            }
            window::Event::Resized(size) => {
                let wanted = self
                    .plugin
                    .pane
                    .as_ref()
                    .map(|pane| self.popover_window(&pane.geometry));
                if let (Some(pane), Some(wanted)) = (self.plugin.pane.as_mut(), wanted) {
                    pane.actual_size = *size;
                    if (wanted.width - size.width).abs() < 1.0
                        && (wanted.height - size.height).abs() < 1.0
                    {
                        pane.resizing = false;
                    }
                }
                self.reflow_plugin()
            }
            window::Event::CloseRequested => self.close_plugin(),
            window::Event::Closed => {
                self.plugin.pane = None;
                self.plugin.preparing = None;
                self.plugin_prepare_next()
            }
            _ => Task::none(),
        })
    }
    pub(crate) fn plugin_view(&self, id: window::Id) -> Option<Element<'_, ShellMessage>> {
        let pane = self.plugin.pane.as_ref().filter(|p| p.window == id)?;
        let owner = pane.owner.clone();
        let image_owner = pane.owner.clone();
        let element = pane.element.clone();
        let revision = pane.revision;
        let content = view::panel(
            &pane.view,
            &format!("{} ({})", pane.manifest.name, pane.owner.id),
            format!("plugin-panel:{}:{}:{}", owner.id, owner.run, element),
            view::Appearance {
                theme: self.tokens(),
                metrics: self.metrics,
                scroll: pane.scroll.clone(),
            },
            move |interaction| {
                message(Message::Input {
                    owner: owner.clone(),
                    element: element.clone(),
                    revision,
                    interaction,
                })
            },
            move |handle| self.clipboard_image(&image_owner, handle),
            view::ChromeActions {
                close: message(Message::Close),
                inspect: message(Message::Inspect(id)),
                scroll: ShellMessage::RegionInterior,
            },
        );
        let content = iced::widget::column![
            iced::widget::column(pane.geometry_error.iter().map(|error| {
                enzui_widgets::banner::enz_banner(error.clone())
                    .tone(enzui_widgets::banner::BannerTone::Danger)
                    .into_element()
            })),
            content
        ]
        .height(Length::Fill);
        let content = enzui_widgets::keyed_column::keyed_column(vec![(
            pane.layout_epoch.to_string(),
            content.into(),
        )])
        .width(Length::Fill)
        .height(Length::Fill);
        let content: Element<'_, ShellMessage> = if pane.view.footer.is_some() {
            iced::widget::mouse_area(content)
                .on_press(ShellMessage::RegionInterior)
                .into()
        } else {
            content.into()
        };
        let radius = f32::from(self.metrics.radius);
        let (left, right) = pane.geometry.attachment.insets(radius);
        let bottom = self.bar == BarEdge::Bottom;
        let island = matches!(
            pane.geometry.attachment,
            enzui_shell::reveal::Attachment::Island { .. }
        );
        let join = pane.geometry.attachment.join(radius);
        let content = iced::widget::container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(iced::Padding {
                left,
                right,
                top: if island {
                    if bottom { 0.0 } else { join }
                } else if bottom {
                    radius
                } else {
                    0.0
                },
                bottom: if island {
                    if bottom { join } else { 0.0 }
                } else if bottom {
                    0.0
                } else {
                    radius
                },
            })
            .into();
        Some(region::popup(
            content,
            region::Popup {
                visible: pane.visible,
                bottom,
                hidden: message(Message::Hidden(id)),
                theme: self.tokens().iced_theme(),
                attachment: pane.geometry.attachment,
                radius,
                color: self.tokens().palette.surface,
                expressive: enzui_shell::motion::expressive(
                    self.desired_settings.compositor.animation_style,
                ),
                input: self.surface_inputs.get(&id).cloned(),
                shadow: self.panel_shadow(),
                glass: self.panel_glass(),
            },
        ))
    }
}

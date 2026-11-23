use super::{Message, Shell};
use crate::{control_center, region, sidebar_view, workspace_view};
use enzui_shell::i18n::{self, tr};
use enzui_shell::tr;
use enzui_shell::{
    catalog::{self, APPS},
    desktop_frame,
    settings::BarEdge,
};
use enzui_theme::{EnzTheme, SpaceToken};
use enzui_widgets::{
    clickable::enz_clickable,
    icon::enz_icon,
    responsive_grid::{CellFn, enz_responsive_grid},
    search_field::enz_search_field,
    text::{TextSize, enz_text},
    toolbar_icon::enz_toolbar_icon,
    toolbar_row::enz_toolbar_row,
};
use iced::widget::{column, container, row};
use iced::{Color, Element, Length, Subscription, Task, window};
use std::time::Duration;

impl Shell {
    pub(crate) fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            self.tick_subscription(),
            self.overview_frame_subscription(),
            if self.control_frames > 0 {
                iced::event::listen_raw(|event, _, id| match event {
                    iced::Event::Window(window::Event::RedrawRequested(_)) => {
                        Some(Message::ControlsFrame(id))
                    }
                    _ => None,
                })
            } else {
                Subscription::none()
            },
            self.sharing_watch
                .as_ref()
                .map_or_else(Subscription::none, |watch| {
                    watch.subscription().map(|_| Message::SharingChanged)
                }),
            iced::time::every(self.settings_tick_period()).map(|_| Message::SettingsTick),
            enzui_shell::backup_notices::subscription().map(|()| Message::BackupCheck),
            enzui_shell::desktop_glass::luminance_changes().map(|()| Message::GlassBackdrop(false)),
            window::events().map(|(id, event)| Message::Window(id, event)),
            iced::event::listen_with(|event, status, id| match (event, status) {
                (
                    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                        ..
                    }),
                    iced::event::Status::Ignored,
                ) => Some(Message::Escape(id)),
                (
                    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        key:
                            key @ iced::keyboard::Key::Named(
                                iced::keyboard::key::Named::ArrowUp
                                | iced::keyboard::key::Named::ArrowDown
                                | iced::keyboard::key::Named::ArrowLeft
                                | iced::keyboard::key::Named::ArrowRight,
                            ),
                        ..
                    }),
                    // Also when the search field moved its cursor: in the
                    // emoji picker the arrows choose.
                    _,
                ) => Some(Message::EmojiKey(id, key)),
                (
                    iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        key: key @ iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter),
                        ..
                    }),
                    iced::event::Status::Ignored,
                ) => Some(Message::EmojiKey(id, key)),
                _ => None,
            }),
            self.plugin_subscription(),
            self.notification_subscription(),
            self.assistant_subscription(),
            enzui_shell::network_connectivity::subscription().map(Message::Connectivity),
            if self.desired_settings.controller_notifications {
                enzui_shell::controllers::subscription()
                    .with(self.controllers_generation)
                    .map(|(generation, list)| Message::Controllers(generation, list))
            } else {
                Subscription::none()
            },
            self.activity_subscription(),
            self.command_subscription(),
            self.media_subscription(),
            self.search_subscription(),
            self.dock_subscription(),
            self.compositor
                .desktop_changes()
                .map(|()| Message::DesktopChanged),
            self.compositor
                .windows_feed()
                .map(|()| Message::RegionInterior),
            self.settings_watch
                .as_ref()
                .map_or_else(Subscription::none, |watch| {
                    watch.subscription().map(|_| Message::SettingsChanged)
                }),
            self.session_subscription(),
            self.spread_subscription(),
        ])
    }

    fn session_subscription(&self) -> Subscription<Message> {
        let session = &self.desired_settings.session;
        let idle_config = self.idle_config();
        let idle = enzui_shell::idle::subscription(idle_config)
            .with(idle_config)
            .map(|(config, event)| Message::Idle(config, event));
        // Previews require idle opt-in to avoid host inhibitors; remote sessions have no idle
        // seat.
        if enzui_shell::desktop_session::mode().nested() || enzui_shell::desktop_session::remote() {
            return if std::env::var("ENZUI_PREVIEW_IDLE").as_deref() == Ok("1") {
                idle
            } else {
                Subscription::none()
            };
        }
        // Plugged-in drives belong to the person at the computer: only an
        // installed login mounts them.
        let drives = if enzui_shell::desktop_session::mode()
            == enzui_shell::desktop_session::Mode::Installed
        {
            enzui_shell::drives::subscription().map(Message::Drive)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            idle,
            if enzui_shell::desktop_session::mode() == enzui_shell::desktop_session::Mode::Installed
            {
                enzui_shell::idle_inhibit::subscription().map(Message::IdleInhibited)
            } else {
                Subscription::none()
            },
            enzui_shell::power_policy::subscription(session.lid).map(Message::PowerPolicy),
            drives,
            enzui_shell::privacy::subscription().map(Message::Privacy),
            if self.recording.is_some() {
                iced::time::every(Duration::from_secs(1)).map(|_| Message::RecordingTick)
            } else {
                Subscription::none()
            },
        ])
    }

    pub(crate) fn idle_config(&self) -> enzui_shell::idle::Config {
        if !self.idle_inhibition_ready || self.idle_inhibited {
            enzui_shell::idle::Config::default()
        } else {
            enzui_shell::idle::Config::from_preferences(
                &self.desired_settings.session,
                self.system.battery.as_ref().is_some_and(|b| !b.charging),
            )
        }
    }

    pub(crate) fn icon_tokens(&self) -> enzui_widgets::toolbar_icon::ToolbarIconTokens {
        self.window_icon_tokens(None)
    }

    // The toolbar icons' tokens in window `id`, in its glass's ink when on glass.
    pub(crate) fn window_icon_tokens(
        &self,
        id: Option<window::Id>,
    ) -> enzui_widgets::toolbar_icon::ToolbarIconTokens {
        let mut tokens = enzui_widgets::toolbar_icon::ToolbarIconTokens::from_palette(
            &self.window_tokens(id).palette,
        );
        tokens.icon_size = f32::from(self.metrics.icon);
        tokens.slot_height = f32::from(self.metrics.icon_slot());
        tokens.button_size = tokens.slot_height - 4.0;
        tokens
    }

    pub(crate) fn tokens(&self) -> EnzTheme {
        let base = self.base_tokens();
        let mut theme = self.plugin.theme_tokens(&base).unwrap_or(base);
        enzui_shell::motion::scale(&mut theme, self.desired_settings.compositor.animation_rate);
        theme
    }

    pub(crate) fn themed_surface(&self) -> Option<[u8; 3]> {
        enzui_shell::panel_style::themed_surface(
            self.desired_settings.theme_scheme.as_deref(),
            &self.theme_document,
            self.light,
        )
    }

    /// The chrome's colour when the person chose one (the accent or their
    /// own), opaque.
    pub(crate) fn chrome_color(&self) -> Option<Color> {
        use enzui_shell::chrome_style::ChromeColor;
        let color = self.desired_settings.chrome.color;
        let accent = if color == ChromeColor::Accent {
            self.theme_tokens_in(self.light, None).palette.accent
        } else {
            Color::TRANSPARENT
        };
        color.tint(self.light, accent)
    }

    /// Whether the chrome is light: the theme's mode, or that of the
    /// chrome's own colour, so its text reads (a black frame on a light
    /// theme takes the dark palette).
    pub(crate) fn chrome_light(&self) -> bool {
        self.chrome_color()
            .map_or(self.light, enzui_shell::chrome_style::is_light)
    }

    pub(crate) fn base_tokens(&self) -> EnzTheme {
        let chrome = self.chrome_color();
        let mut theme = self.theme_tokens_in(self.chrome_light(), chrome);
        // Solid chrome's see-through; glass takes its tint instead.
        theme.palette.surface.a = f32::from(self.desired_settings.chrome.opacity) / 100.0;
        theme
    }

    // The shell's theme in `light` or dark mode, its surface `chrome` when
    // one was chosen.
    fn theme_tokens_in(&self, light: bool, chrome: Option<Color>) -> EnzTheme {
        let mut theme = if light {
            EnzTheme::light()
        } else {
            EnzTheme::dark()
        };
        self.theme_document.apply(
            &mut theme,
            if light {
                enzui_theme::EnzThemeMode::Light
            } else {
                enzui_theme::EnzThemeMode::Dark
            },
        );
        // In light mode an ink that is not pitch black (unless the theme file
        // sets its own).
        let mode = if light {
            enzui_theme::EnzThemeMode::Light
        } else {
            enzui_theme::EnzThemeMode::Dark
        };
        if light && self.theme_document.get("palette.text", mode).is_none() {
            theme.palette.text = Color::from_rgb8(0x33, 0x33, 0x38);
        }
        if light
            && self
                .theme_document
                .get("palette.text_muted", mode)
                .is_none()
        {
            theme.palette.text_muted = Color::from_rgb8(0x6B, 0x6B, 0x72);
        }
        // The bar and frame keep the frame's own grey unless the shared
        // theme gives the desktop a surface of its own.
        if let Some(color) = chrome {
            theme.palette.surface = color;
        } else {
            match self.themed_surface() {
                None => {
                    let [r, g, b] = desktop_frame::color(light);
                    theme.palette.surface = Color::from_rgb8(r, g, b);
                }
                // A light scheme's tint is its page colour: the bar and panels take
                // it like the frame does (they stayed pure white beside tinted
                // windows and a tinted frame).
                Some([r, g, b]) if light => {
                    theme.palette.surface = Color::from_rgb8(r, g, b);
                }
                Some(_) => {}
            }
        }
        self.desired_settings.fonts.apply_to_theme(
            &mut theme,
            &self.theme_document,
            if light {
                enzui_theme::EnzThemeMode::Light
            } else {
                enzui_theme::EnzThemeMode::Dark
            },
        );
        // At the default weight, shell text is one step heavier than app body
        // text; a weight the person chose in Settings is kept as chosen.
        if self.desired_settings.fonts.weight == enzui_shell::font_settings::Weight::Regular {
            theme.typography.weight_regular = theme.typography.weight_regular.max(500);
            theme.typography.weight_medium = theme.typography.weight_medium.max(560);
        }
        enzui_shell::settings::apply_text_scale(&mut theme, self.metrics.text_scale());
        theme
    }

    pub(crate) fn theme(&self, id: window::Id) -> iced::Theme {
        let mut theme = self.tokens();
        if self.search.window == Some(id) {
            if self.desired_settings.compositor.animation_style == 0 {
                enzui_shell::motion::search_timing(
                    &mut theme,
                    self.desired_settings.compositor.animation_rate,
                    self.search.preferences.animations,
                );
            } else if !self.search.preferences.animations {
                theme.motion.reduced_motion = true;
            }
        }
        let theme = theme.iced_theme();
        // A window on glass draws its content in the glass's ink.
        match self
            .glass_style(id)
            .filter(|_| self.glass_surface(id).is_some())
        {
            Some(style) => enzui_shell::glass_ink::look(&theme, style, self.window_ink(id)).0,
            None => theme,
        }
    }
    pub(crate) fn style(&self, _: &iced::Theme) -> iced::theme::Style {
        iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: self.tokens().palette.text,
        }
    }
    pub(crate) fn title(&self, id: window::Id) -> String {
        if id == self.panel {
            tr("shell-enzui-desktop")
        } else if Some(id) == self.overview {
            tr("shell-desktops")
        } else if Some(id) == self.control_center {
            tr("shell-control-center-2")
        } else if self.search.window == Some(id) {
            tr("shell-quick-search")
        } else if self.notification.owns_window(id) {
            tr("shell-notifications")
        } else if self.plugin.owns_window(id) {
            tr("shell-plugin-panel-2")
        } else if self.assistant.owns(id) {
            tr("shell-assistant")
        } else {
            tr("shell-applications")
        }
        .into()
    }

    pub(crate) fn view(&self, id: window::Id) -> Element<'_, Message> {
        i18n::localizer().view(|| self.localized_view(id))
    }
    fn localized_view(&self, id: window::Id) -> Element<'_, Message> {
        if self.dock.window == Some(id) {
            return self.dock_view();
        }
        if self.search.window == Some(id) {
            return self.quick_search_view();
        }
        if let Some(view) = self.notification_view(id) {
            return view;
        }
        if let Some(view) = self.assistant_view(id) {
            return view;
        }
        if let Some(view) = self.osd_view(id) {
            return view;
        }
        if let Some(view) = self.switcher_view(id) {
            return view;
        }
        if let Some(view) = self.spread_view(id) {
            return view;
        }
        if let Some(view) = self.emoji_view(id) {
            return view;
        }
        if let Some(view) = self.capture_view(id) {
            return view;
        }
        if let Some(view) = self.osk_view(id) {
            return view;
        }
        if let Some(view) = self.plugin_view(id) {
            return view;
        }
        if Some(id) == self.sidebar_rail {
            self.sidebar.as_ref().map_or_else(
                || iced::widget::space().into(),
                |state| sidebar_view::rail(self, state),
            )
        } else if Some(id) == self.sidebar_pane {
            self.sidebar.as_ref().map_or_else(
                || iced::widget::space().into(),
                |state| sidebar_view::pane(self, state),
            )
        } else if id == self.panel {
            self.panel_view()
        } else if Some(id) == self.overview {
            let island = matches!(
                self.overview_attachment,
                enzui_shell::reveal::Attachment::Island { .. }
            );
            if island {
                region::popup(
                    workspace_view::overview(self),
                    self.popup(
                        id,
                        self.overview_visible,
                        self.overview_attachment,
                        Message::OverviewHidden(id),
                    ),
                )
            } else {
                region::view(
                    workspace_view::overview(self),
                    self.overview_visible,
                    if self.bar == BarEdge::Bottom {
                        enzui_widgets::motion::SlideDirection::Down
                    } else {
                        enzui_widgets::motion::SlideDirection::Up
                    },
                    Message::OverviewHidden(id),
                    &self.tokens().iced_theme(),
                    enzui_shell::reveal::Reveal {
                        shape: enzui_shell::reveal::Shape::Overview {
                            left: self.desktop_edges().0 as f32,
                            right: self.desktop_edges().1 as f32,
                            bottom: self.bar == BarEdge::Bottom,
                        },
                        radius: f32::from(self.metrics.radius),
                        color: self.tokens().palette.surface,
                    },
                    self.surface_inputs.get(&id).cloned(),
                    enzui_shell::motion::expressive(
                        self.desired_settings.compositor.animation_style,
                    ),
                )
            }
        } else if Some(id) == self.control_center {
            let mut popup = self.popup(
                id,
                self.control_visible || self.control_warming,
                self.control_attachment,
                Message::ControlsHidden(id),
            );
            if self.control_warming {
                popup.input = None;
            }
            let content = region::popup(control_center::view(self), popup);
            // Changing the wrapper starts the parked motion at zero; keyed
            // columns preserve child state when their child count is unchanged.
            if self.control_warming {
                container(content)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .into()
            } else {
                content
            }
        } else if Some(id) == self.launcher {
            match self.launcher_geometry.as_ref() {
                Some(geometry) => region::popup(
                    self.launcher_view(),
                    self.popup(
                        id,
                        self.launcher_visible,
                        geometry.attachment,
                        Message::LauncherHidden(id),
                    ),
                ),
                None => self.launcher_view(),
            }
        } else {
            iced::widget::space().into()
        }
    }

    pub(crate) fn popup(
        &self,
        id: iced::window::Id,
        visible: bool,
        attachment: enzui_shell::reveal::Attachment,
        hidden: Message,
    ) -> region::Popup {
        region::Popup {
            visible,
            bottom: self.bar == BarEdge::Bottom,
            hidden,
            theme: self.tokens().iced_theme(),
            attachment,
            radius: f32::from(self.metrics.radius),
            color: self.glass_tint(id, self.tokens().palette.surface),
            expressive: enzui_shell::motion::expressive(
                self.desired_settings.compositor.animation_style,
            ),
            input: self.surface_inputs.get(&id).cloned(),
            shadow: self.panel_shadow(),
            glass: self.panel_glass(),
        }
    }

    // The material of the chrome's glass: the bar's islands, the rail, the
    // assistant button, the level display.
    pub(crate) fn chrome_glass(&self) -> Option<enzui_theme::MaterialStyle> {
        enzui_shell::reveal::chrome_material(self.desired_settings.panel_glass)
    }

    // The material of window `id`'s glass, where islands use it.
    pub(crate) fn glass_style(&self, id: window::Id) -> Option<enzui_theme::MaterialStyle> {
        let chrome = id == self.panel
            || Some(id) == self.sidebar_rail
            || self.assistant.owns_trigger(id)
            || self.owns_osd(id);
        if chrome {
            self.chrome_glass()
        } else {
            self.panel_glass()
        }
        .filter(|_| self.metrics.islands.is_some())
    }

    // The surface of window `id`, while the compositor paints glass on it.
    fn glass_surface(&self, id: window::Id) -> Option<&enzui_shell::surface_input::SurfaceInput> {
        let input = if id == self.panel {
            self.bar_glass.as_ref()
        } else {
            self.glass_windows
                .get(&id)
                .or_else(|| self.surface_inputs.get(&id))
        };
        input.filter(|input| input.has_glass())
    }

    // The ink of window `id`'s glass: `1.0` light to `0.0` dark, or `None`
    // for the theme's own.
    pub(crate) fn window_ink(&self, id: window::Id) -> Option<f32> {
        let style = self.glass_style(id)?;
        self.glass_surface(id)?;
        // Glass in a colour the person chose is tinted that colour and reads
        // in the chrome's own palette (built for that colour), whatever lies
        // behind it: an ink here re-tinted the bar from the ink and left
        // black islands white.
        if self.chrome_color().is_some() {
            return None;
        }
        enzui_shell::glass_ink::ink(style, self.glass_inks.get(&id))
    }

    // The tokens content in window `id` draws with: on glass, the glass
    // palette in the glass's ink, as the window's theme has it.
    pub(crate) fn window_tokens(&self, id: Option<window::Id>) -> EnzTheme {
        let tokens = self.tokens();
        match (
            id.and_then(|id| self.glass_style(id)),
            id.and_then(|id| self.glass_surface(id).map(|_| id)),
        ) {
            (Some(style), Some(id)) => match self.window_ink(id) {
                Some(ink) => tokens.on_material_ink(style, ink),
                None => tokens.on_material(style),
            },
            _ => tokens,
        }
    }

    // The colour window `id`'s glass carries: its ink's, or `fallback`.
    pub(crate) fn glass_tint(&self, id: window::Id, fallback: Color) -> Color {
        if let Some(color) = self
            .chrome_color()
            .filter(|_| self.glass_style(id).is_some())
        {
            return color;
        }
        match (self.glass_style(id), self.window_ink(id)) {
            (Some(style), Some(ink)) => {
                enzui_shell::glass_ink::look(&self.tokens().iced_theme(), style, Some(ink)).1
            }
            _ => fallback,
        }
    }

    // Moves every glass window's ink toward its backdrop, ticking while one
    // settles, waits out a switch or crossfades, and redraws when one moved.
    pub(crate) fn glass_backdrop(&mut self, tick: bool) -> Task<Message> {
        if tick {
            self.glass_ink_ticking = false;
        }
        let now = std::time::Instant::now();
        let ids: Vec<window::Id> = std::iter::once(self.panel)
            .chain(self.glass_windows.keys().copied())
            .chain(self.surface_inputs.keys().copied())
            .collect();
        let (mut more, mut moved) = (false, false);
        let mut live = std::collections::HashSet::new();
        for id in ids {
            let Some(luminance) = self
                .glass_surface(id)
                .map(enzui_shell::surface_input::SurfaceInput::average_luminance)
            else {
                continue;
            };
            live.insert(id);
            let ink = self.glass_inks.entry(id).or_default();
            let before = ink.ink();
            more |= ink.advance(now, luminance);
            moved |= ink.ink() != before;
        }
        self.glass_inks.retain(|id, _| live.contains(id));
        let mut tasks = Vec::new();
        if moved {
            tasks.push(iced_runtime::task::effect(iced_runtime::Action::Window(
                iced_runtime::window::Action::RedrawAll,
            )));
            tasks.push(self.refresh_bar_glass());
        }
        if more && !self.glass_ink_ticking {
            self.glass_ink_ticking = true;
            tasks.push(Task::perform(
                tokio::time::sleep(Duration::from_millis(16)),
                |()| Message::GlassBackdrop(true),
            ));
        }
        Task::batch(tasks)
    }

    // A transient overlay window (level display, window switcher) as one
    // glass panel of `radius` in islands mode, or a solid one.
    pub(crate) fn transient_glass<'a>(
        &self,
        id: window::Id,
        body: impl Into<Element<'a, Message>>,
        radius: f32,
    ) -> Element<'a, Message> {
        self.transient_glass_lifted(id, body, radius, 0.0)
    }

    /// [`Self::transient_glass`] with a panel shadow in the `room` the window
    /// leaves around the pane, so it reads as the foremost layer over a window
    /// of the same colour.
    pub(crate) fn transient_glass_lifted<'a>(
        &self,
        id: window::Id,
        body: impl Into<Element<'a, Message>>,
        radius: f32,
        room: f32,
    ) -> Element<'a, Message> {
        let palette = self.tokens().palette;
        let level = if room > 0.0 {
            self.panel_shadow_level()
        } else {
            0
        };
        let lit =
            0.2126 * palette.surface.r + 0.7152 * palette.surface.g + 0.0722 * palette.surface.b
                > 0.5;
        let glass = enzui_shell::desktop_glass::WindowGlass {
            input: self.glass_windows.get(&id).cloned(),
            style: self.glass_style(id),
            tint: palette.surface,
            light_ink: self.window_ink(id),
            reduced_motion: self.tokens().motion.reduced_motion,
            shadow: enzui_shell::reveal::glass_shadow(level, palette.surface, false),
            pending: self.glass_binding.contains(&id),
        };
        let mut pane = enzui_shell::desktop_glass::glass_pane(body)
            .radius(radius)
            .fallback(palette.surface)
            .outline(palette.border);
        if level > 0 {
            pane = pane.shadow(iced::Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, if lit { 0.18 } else { 0.45 }),
                offset: iced::Vector::new(0.0, 4.0),
                blur_radius: (room - 4.0).max(0.0),
            });
        }
        let pane: Element<'a, Message> = if room > 0.0 {
            container(pane).padding(room).into()
        } else {
            pane.into()
        };
        enzui_shell::desktop_glass::window_glass(pane, &glass)
    }

    pub(crate) fn bind_transient_glass(&mut self, id: window::Id) -> Task<Message> {
        if self.metrics.islands.is_none() || self.panel_glass().is_none() {
            return Task::none();
        }
        self.glass_binding.insert(id);
        enzui_shell::desktop_glass::bind(id, self.surface_globals.clone())
            .map(move |result| Message::GlassBound(id, result))
    }

    pub(crate) fn glass_bound(
        &mut self,
        id: window::Id,
        result: Result<enzui_shell::surface_input::SurfaceInput, String>,
    ) -> Task<Message> {
        self.glass_binding.remove(&id);
        match result {
            Ok(input)
                if self.owns_osd(id)
                    || self.owns_switcher(id)
                    || self.emoji.owns(id)
                    || self.osk.owns(id) =>
            {
                self.surface_globals.get_or_insert_with(|| input.globals());
                self.glass_windows.insert(id, input);
            }
            Ok(_) => {}
            Err(error) => eprintln!("enzui-shell glass: {error}"),
        }
        Task::none()
    }

    pub(crate) fn panel_glass(&self) -> Option<enzui_theme::MaterialStyle> {
        let materials = &enzui_shell::reveal::PANEL_MATERIALS;
        materials[usize::from(self.desired_settings.panel_glass).min(materials.len() - 1)].1
    }

    // The popovers' shadow level: as Settings chose it, short of the large
    // one round a floating island panel, which its window has no room for.
    fn panel_shadow_level(&self) -> u8 {
        if self.metrics.islands.is_some() {
            self.desired_settings
                .panel_shadow
                .min(enzui_shell::reveal::ISLAND_PANEL_SHADOW_MAX)
        } else {
            self.desired_settings.panel_shadow
        }
    }

    // The popovers' shadow as Settings chose it.
    pub(crate) fn panel_shadow(&self) -> enzui_shell::reveal::PanelShadow {
        enzui_shell::reveal::PanelShadow {
            room: true,
            level: self.panel_shadow_level(),
            bar: self.desired_settings.bar_shadow > 0,
        }
    }

    // The window around a shadowed popover's panel.
    pub(crate) fn popover_window(
        &self,
        geometry: &enzui_shell::compositor::ControlGeometry,
    ) -> iced::Size {
        let m = enzui_shell::reveal::margins(self.bar == BarEdge::Bottom, geometry.attachment);
        iced::Size::new(
            geometry.width + m.left + m.right,
            geometry.height + m.top + m.bottom,
        )
    }

    pub(crate) fn applications_button(&self) -> Element<'_, Message> {
        enzui_shell::bar_item::trigger(
            enz_clickable(
                row![
                    // The system's own mark opens its apps.
                    enzui_shell::brand::mark(f32::from(self.metrics.icon) + 2.0).into_element(),
                    enz_text(tr("shell-apps"))
                        .size_px(f32::from(self.metrics.text))
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .into_element(),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
                Message::ShowLauncher,
            ),
            self.metrics,
        )
        .aria_label(tr("shell-applications"))
        .id("applications".into())
        .into_element()
    }

    // "● 1:05" while recording; pressing it stops.
    fn recording_indicator(&self) -> Option<Element<'_, Message>> {
        let recording = self.recording.as_ref()?;
        let elapsed = enzui_shell::recording::elapsed_text(recording.started.elapsed());
        let danger = self.tokens().palette.destructive;
        let dot = container(iced::widget::space())
            .width(10)
            .height(10)
            .style(move |_| container::Style {
                background: Some(danger.into()),
                border: iced::Border {
                    radius: 5.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        let content = row![
            dot,
            enz_text(elapsed.clone())
                .size_px(f32::from(self.metrics.text))
                .weight_rung(enzui_theme::WeightRung::Medium)
                .into_element()
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);
        Some(
            enzui_shell::bar_item::trigger(
                enz_clickable(content, Message::StopRecording),
                self.metrics,
            )
            .aria_label(tr!("shell-recording-the-screen-stop-recording", "arg0" => elapsed))
            .id("stop-recording".into())
            .into_element(),
        )
    }

    pub(crate) fn read_sharing_grant() -> Option<enzui_shell::remote_access::SharingGrant> {
        if enzui_shell::desktop_session::mode() != enzui_shell::desktop_session::Mode::Installed
            || enzui_shell::desktop_session::remote()
        {
            return None;
        }
        enzui_shell::remote_access::sharing_grant()
    }

    fn sharing_indicator(&self) -> Option<Element<'_, Message>> {
        let access = self.remote_sharing?;
        let danger = self.tokens().palette.destructive;
        let dot = container(iced::widget::space())
            .width(10)
            .height(10)
            .style(move |_| container::Style {
                background: Some(danger.into()),
                border: iced::Border {
                    radius: 5.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        let content = row![
            dot,
            enz_text(enzui_shell::i18n::tr("shell-sharing"))
                .size_px(f32::from(self.metrics.text))
                .into_element()
        ]
        .spacing(6)
        .align_y(iced::Alignment::Center);
        let permission = match access {
            enzui_shell::remote_access::SharingGrant::ViewOnly => {
                tr("shell-viewers-can-watch-this-desktop")
            }
            enzui_shell::remote_access::SharingGrant::Control => {
                tr("shell-viewers-can-watch-and-control-this-desktop")
            }
        };
        Some(enzui_shell::bar_item::trigger(enz_clickable(content, Message::SettingsPage("remote/sharing")), self.metrics)
            .aria_label(tr!("shell-desktop-sharing-enabled-open-remote-access-to-stop-sharing", "arg0" => permission.to_owned()))
            .id("desktop-sharing-enabled".into()).into_element())
    }

    pub(crate) fn bar_island_keys(&self) -> Vec<String> {
        use enzui_shell::bar_layout::Section;
        Section::ALL
            .iter()
            .flat_map(|section| self.desired_settings.layout.items(*section))
            .filter(|item| item.enabled())
            .flat_map(|item| {
                // The accessibility menu's button is its own island before the Control Center's.
                let menu = (item.id() == "builtin.controls"
                    && self.desired_settings.accessibility_menu)
                    .then(|| "builtin.accessibility".to_owned());
                // And so is the on-screen keyboard's.
                let keyboard = (item.id() == "builtin.controls"
                    && self.desired_settings.osk_button)
                    .then(|| "builtin.osk".to_owned());
                menu.into_iter()
                    .chain(keyboard)
                    .chain([item.id().to_owned()])
            })
            .chain(
                self.has_visible_activity()
                    .then(|| "builtin.activity".to_owned()),
            )
            .chain(self.remote_sharing.map(|_| "builtin.sharing".to_owned()))
            .collect()
    }

    // One bar element's own view, or `None` when it shows nothing.
    fn bar_item(&self, id: &str) -> Option<Element<'_, Message>> {
        if self
            .dock
            .surface_element
            .as_ref()
            .is_some_and(|(owner, element)| {
                element == id
                    && self.plugin.live.get(&owner.id).is_some_and(|live| {
                        live.owner == *owner
                            && live.authority.load(std::sync::atomic::Ordering::Acquire)
                    })
            })
        {
            return if self.dock_active()
                && self.dock.preferences.position == enzui_shell::dock::Position::Bar
            {
                let width = self
                    .bar_geometry
                    .as_ref()
                    .map_or(1440.0, |(_, g)| g.size.width);
                let icon = f32::from(self.dock.preferences.icon_size)
                    .min(f32::from(self.metrics.bar) - 16.0)
                    .max(12.0);
                let wanted = self.dock.entries.len().max(1) as f32
                    * (icon + 6.0 + f32::from(self.dock.preferences.spacing));
                Some(
                    enzui_widgets::scroll_view::enz_scroll_view(
                        self.dock_icons(crate::dock_ui::Origin::Bar, false),
                    )
                    .axis(enzui_widgets::scroll_view::Axis::Horizontal)
                    .width(wanted.min((width / 3.0 - 180.0).max(48.0)))
                    .height(f32::from(self.metrics.bar))
                    .into_element(),
                )
            } else {
                None
            };
        }
        match id {
            "builtin.applications" => Some(self.applications_button()),
            "builtin.notifications" if enzui_shell::desktop_session::mode().managed() => {
                Some(self.notification_bell())
            }
            "builtin.notifications" => None,
            "builtin.desktops" => Some(workspace_view::indicator(self)),
            "builtin.controls" => {
                let mut corner = row![].spacing(8).align_y(iced::Alignment::Center);
                if let Some(recording) = self.recording_indicator() {
                    corner = corner.push(recording);
                }
                Some(corner.push(control_center::status_button(self)).into())
            }
            "builtin.clock" if enzui_shell::desktop_session::mode().managed() => {
                Some(self.notification_clock())
            }
            "builtin.clock" => Some(
                container(
                    enz_text(&self.clock)
                        .size_px(f32::from(self.metrics.text))
                        .weight_rung(enzui_theme::WeightRung::Medium)
                        .into_element(),
                )
                .padding([
                    0,
                    self.metrics
                        .islands
                        .map_or(0, |_| self.metrics.bar_padding(10)),
                ])
                .into(),
            ),
            _ => self.plugin_bar(id),
        }
    }

    fn osk_item(&self) -> Element<'_, Message> {
        enzui_shell::bar_item::trigger(
            enz_clickable(
                enz_icon("keyboard")
                    .size(f32::from(self.metrics.icon))
                    .into_element(),
                Message::ToggleOsk,
            ),
            self.metrics,
        )
        .aria_label(if self.osk.shown() {
            tr("shell-hide-on-screen-keyboard")
        } else {
            tr("shell-show-on-screen-keyboard")
        })
        .id("on-screen-keyboard".into())
        .into_element()
    }

    pub(crate) fn bar_zone(
        &self,
        section: enzui_shell::bar_layout::Section,
    ) -> Option<Element<'_, Message>> {
        let middle = section == enzui_shell::bar_layout::Section::Middle;
        let activity = middle
            .then(|| self.activity_item())
            .flatten()
            .map(|view| ("builtin.activity", view));
        let sharing = middle
            .then(|| self.sharing_indicator())
            .flatten()
            .map(|view| ("builtin.sharing", view));
        let items: Vec<_> = self
            .desired_settings
            .layout
            .items(section)
            .iter()
            .filter(|i| i.enabled())
            .flat_map(|i| {
                let menu = (i.id() == "builtin.controls"
                    && self.desired_settings.accessibility_menu)
                    .then(|| ("builtin.accessibility", self.accessibility_item()));
                // Its own item, like the accessibility menu: inside the
                // controls' pill it sat a wide, uneven gap from their icons.
                let keyboard = (i.id() == "builtin.controls" && self.desired_settings.osk_button)
                    .then(|| ("builtin.osk", self.osk_item()));
                menu.into_iter()
                    .chain(keyboard)
                    .chain(self.bar_item(i.id()).map(|view| (i.id(), view)))
            })
            .chain(activity)
            .chain(sharing)
            .map(|(id, view)| {
                let island = container(view)
                    .id(iced::advanced::widget::Id::from(format!("bar-island-{id}")))
                    .height(f32::from(self.metrics.bar))
                    .align_y(iced::Alignment::Center);
                match self.metrics.islands {
                    // A glass island's body is the compositor's.
                    Some(style) if !style.joined_bar && !self.bar_glass_on => {
                        let surface = self.tokens().palette.surface;
                        let radius = self.metrics.island_radius();
                        island
                            .style(move |_| container::Style {
                                background: Some(surface.into()),
                                border: iced::Border {
                                    radius: radius.into(),
                                    ..Default::default()
                                },
                                ..Default::default()
                            })
                            .into()
                    }
                    // Glass: a hairline rim keeps the island's shape on any
                    // wallpaper (a dark island over a dark one vanished).
                    Some(style) if !style.joined_bar => {
                        let radius = self.metrics.island_radius();
                        let rim = if self.light {
                            Color::from_rgba(0.0, 0.0, 0.0, 0.08)
                        } else {
                            Color::from_rgba(1.0, 1.0, 1.0, 0.06)
                        };
                        island
                            .style(move |_| container::Style {
                                border: iced::Border {
                                    radius: radius.into(),
                                    width: 1.0,
                                    color: rim,
                                },
                                ..Default::default()
                            })
                            .into()
                    }
                    _ => island.into(),
                }
            })
            .collect();
        (!items.is_empty()).then(|| {
            row(items)
                .spacing(
                    self.metrics
                        .islands
                        .filter(|style| !style.joined_bar)
                        .map_or(18.0, |style| f32::from(style.gap)),
                )
                .height(f32::from(self.metrics.bar))
                .align_y(iced::Alignment::Center)
                .into()
        })
    }

    pub(crate) fn bar_contents(&self, width: f32) -> Element<'_, Message> {
        use enzui_shell::bar_layout::Section;
        use enzui_widgets::scroll_view::{Axis, enz_scroll_view};
        let items = Section::ALL.map(|section| (section, self.bar_zone(section)));
        let count = items
            .iter()
            .filter(|(_, item)| item.is_some())
            .count()
            .max(1) as f32;
        let spacing = enzui_theme::SpacingScale::default();
        let separate = self.metrics.islands.filter(|style| !style.joined_bar);
        let inset = if separate.is_some() {
            enzui_theme::SpaceToken::None
        } else {
            enzui_theme::SpaceToken::Md
        };
        let gap = separate.map_or(enzui_theme::SpaceToken::Sm, |style| {
            enzui_theme::SpaceToken::Custom(f32::from(style.gap))
        });
        let separators = if separate.is_some() { count - 1.0 } else { 2.0 };
        let limit = ((width - 2.0 * inset.resolve(&spacing) - separators * gap.resolve(&spacing))
            / count)
            .max(28.0);
        let mut zones = enz_toolbar_row()
            .center_absolute(true)
            .inset(inset)
            .zone_spacing(gap)
            .spacing_scale(spacing)
            .height(f32::from(self.metrics.bar));
        for (section, item) in items {
            if let Some(item) = item {
                let view = container(
                    enz_scroll_view(item)
                        .axis(Axis::Horizontal)
                        .width(Length::Shrink)
                        .height(f32::from(self.metrics.bar))
                        .id(format!("bar-scroll-{}", section.key()).into())
                        .backdrop(self.tokens().palette.surface)
                        .into_element(),
                )
                .width(Length::Shrink.max(limit))
                .into();
                zones = match section {
                    Section::Start => zones.start(view),
                    Section::Middle => zones.center(view),
                    Section::End => zones.end(view),
                };
            }
        }
        zones.into_element()
    }
    pub(crate) fn panel_view(&self) -> Element<'_, Message> {
        let palette = self.window_tokens(Some(self.panel)).palette;
        let zones = iced::widget::responsive(move |size| self.bar_contents(size.width))
            .height(f32::from(self.metrics.bar));
        let zones = enzui_shell::bar_geometry::observe(
            zones.into(),
            self.bar_island_keys(),
            self.bar_applied_epoch,
            Message::BarGeometry,
        );
        let islands = self.metrics.islands;
        let radius = self.metrics.island_radius();
        // The connected bar on glass: the compositor paints its body, and a
        // hairline rim keeps its shape on any wallpaper, as for the islands.
        let joined_glass = self.bar_glass_on && islands.is_some_and(|style| style.joined_bar);
        let rim = if self.light {
            Color::from_rgba(0.0, 0.0, 0.0, 0.08)
        } else {
            Color::from_rgba(1.0, 1.0, 1.0, 0.06)
        };
        let bar = container(zones)
            .padding([
                0,
                if islands.is_some_and(|style| !style.joined_bar) {
                    0
                } else {
                    12
                },
            ])
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: (islands.is_none_or(|style| style.joined_bar) && !joined_glass)
                    .then_some(palette.surface.into()),
                border: iced::Border {
                    radius: if islands.is_some() { radius } else { 0.0 }.into(),
                    width: if joined_glass { 1.0 } else { 0.0 },
                    color: rim,
                },
                ..Default::default()
            });
        let bar = container(bar)
            .padding([0.0, islands.map_or(0.0, |style| f32::from(style.side_gap))])
            .width(Length::Fill)
            .height(Length::Fill);
        let regions = match islands {
            Some(style) if style.joined_bar => {
                let inset = f32::from(style.side_gap);
                let body = iced::Rectangle::new(
                    iced::Point::new(inset, 0.0),
                    iced::Size::new(
                        (self.bar_size.width - 2.0 * inset).max(0.0),
                        self.bar_size.height,
                    ),
                );
                if self.bar_geometry.is_some() && body.width > 0.0 {
                    vec![(body, radius)]
                } else {
                    Vec::new()
                }
            }
            _ => self
                .bar_geometry
                .as_ref()
                .map_or_else(Vec::new, |(_, geometry)| {
                    geometry
                        .islands
                        .iter()
                        .map(|island| (island.clip, radius))
                        .collect()
                }),
        };
        let bar = enzui_shell::desktop_glass::fixed_glass(
            bar.into(),
            enzui_shell::desktop_glass::WindowGlass {
                input: self.bar_glass.clone(),
                style: self.chrome_glass().filter(|_| self.bar_glass_on),
                tint: self.glass_tint(self.panel, self.tokens().palette.surface),
                light_ink: self.window_ink(self.panel),
                reduced_motion: self.tokens().motion.reduced_motion,
                shadow: enzui_shell::desktop_frame::bar_glass_shadow(
                    self.desired_settings.bar_shadow,
                    self.light,
                    self.bar == BarEdge::Bottom,
                ),
                pending: self.bar_glass_pending,
            },
            regions,
        );
        iced::widget::mouse_area(enzui_widgets::motion::motion(
            bar,
            // With the frame and the rail, as the shell changes style.
            self.chrome_shown(),
            enzui_widgets::motion::MotionEffect::Fade,
        ))
        .on_press(Message::DismissRegions)
        .on_right_press(Message::DismissRegions)
        .on_middle_press(Message::DismissRegions)
        .into()
    }

    pub(crate) fn launcher_view(&self) -> Element<'_, Message> {
        let palette = self.window_tokens(self.launcher).palette;
        let heading = row![
            enz_text(tr("shell-applications"))
                .size(TextSize::Title2)
                .into_element(),
            iced::widget::space().width(Length::Fill),
            enz_toolbar_icon("x", Message::HideLauncher)
                .tokens(self.window_icon_tokens(self.launcher))
                .aria_label(tr("shell-close-launcher"))
                .id("close-launcher".into())
                .into_element(),
        ]
        .align_y(iced::Alignment::Center);
        let filtered: Vec<_> = APPS
            .iter()
            .filter(|app| catalog::offered(app) && catalog::matches(app, &self.query))
            .collect();
        let mut search = enz_search_field(&self.query, Message::Search)
            .placeholder(tr("shell-search-apps"))
            .id("launcher-search")
            .width(Length::Fill);
        if let Some(app) = filtered.first() {
            search = search.on_submit(Message::Launch(app.id));
        }
        let cells: Vec<CellFn<'_, Message>> = filtered
            .iter()
            .map(|&&app| {
                let installed = catalog::executable(&self.root, app.id).is_file();
                let enabled = installed && !self.launching;
                Box::new(move || {
                    // The app's own icon from the theme; a glyph on its colour otherwise.
                    let icon: Element<'_, Message> = match app.themed_icon(64) {
                        Some(path) => {
                            container(enzui_shell::icon_theme::source(&path).into_element(64.0))
                                .center(64)
                                .into()
                        }
                        None => {
                            let color = Color::from_rgb8(app.color[0], app.color[1], app.color[2]);
                            container(
                                enz_icon(app.icon)
                                    .size(32.0)
                                    .tint(Color::WHITE)
                                    .into_element(),
                            )
                            .center(64)
                            .style(move |_| container::Style {
                                background: Some(color.into()),
                                border: iced::Border {
                                    radius: 14.0.into(),
                                    ..Default::default()
                                },
                                ..Default::default()
                            })
                            .into()
                        }
                    };
                    let content = column![
                        icon,
                        enz_text(app.name)
                            .size_px(enzui_theme::text_scale::effective().apply_px(14.0))
                            .into_element(),
                        enz_text(if installed {
                            app.description
                        } else {
                            tr("shell-not-built")
                        })
                        .size_px(enzui_theme::text_scale::effective().apply_px(11.0))
                        .muted_on(palette.surface)
                        .into_element()
                    ]
                    .spacing(8)
                    .align_x(iced::Alignment::Center);
                    enz_clickable(content, Message::Launch(app.id))
                        .aria_label(tr!("shell-open-3", "arg0" => app.name.to_owned()))
                        .description(app.description)
                        .id(format!("app-{}", app.id).into())
                        .enabled(enabled)
                        .width(Length::Fill)
                        .height(144)
                        .padding(8)
                        .radius(12.0)
                        .border_width(0.0)
                        .align_x(iced::Alignment::Center)
                        .into_element()
                }) as CellFn<'_, Message>
            })
            .collect();
        let mut body = column![heading, search.into_element()].spacing(22);
        if cells.is_empty() {
            body = body.push(enz_text(tr("shell-no-matching-applications")).into_element());
        } else {
            body = body.push(
                enz_responsive_grid(cells)
                    .columns_wide(3)
                    .columns_medium(3)
                    .columns_narrow(3)
                    .columns_compact(2)
                    .gap(SpaceToken::Md)
                    .into_element(),
            );
        }
        if let Some(error) = &self.error {
            body = body.push(enz_text(error).into_element());
        }
        let island = self.launcher_geometry.is_some();
        container(body)
            .padding(28)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(move |_| container::Style {
                background: (!island).then_some(palette.surface.into()),
                ..Default::default()
            })
            .into()
    }
}

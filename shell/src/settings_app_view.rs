use super::*;
use enzui_shell::i18n::{self, tr};
use enzui_shell::tr;

impl SettingsApp {
    pub(super) fn publish_text_size(&self) {
        i18n::select(self.settings.language);
        let factor = self.themes.as_ref().map_or_else(
            || self.settings.effective_metrics().text_scale(),
            |themes| themes.text_scale().factor(),
        );
        let (scale, _) = enzui_theme::text_scale::TextScale::admit(factor);
        enzui_theme::text_scale::publish(scale);
    }
    pub(super) fn text_px(&self, px: f32) -> f32 {
        px * self.themes.as_ref().map_or_else(
            || self.settings.effective_metrics().text_scale(),
            |themes| themes.text_scale().factor(),
        )
    }
    pub(super) fn tokens(&self) -> EnzTheme {
        let mode = self.theme_mode();
        let mut theme = self.themes.as_ref().map_or_else(
            || {
                let mut theme = if self.settings.light {
                    EnzTheme::light()
                } else {
                    EnzTheme::dark()
                };
                self.theme.document().apply(&mut theme, mode);
                self.settings
                    .fonts
                    .apply_to_theme(&mut theme, self.theme.document(), mode);
                theme
            },
            |themes| themes.theme().clone(),
        );
        if self.themes.is_none() {
            enzui_shell::settings::apply_text_scale(
                &mut theme,
                self.settings.effective_metrics().text_scale(),
            );
        }
        theme
    }
    pub(super) fn theme(&self) -> iced::Theme {
        self.tokens().iced_theme()
    }
    pub(super) fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(self.tick_period()).map(|_| Message::Tick),
            if self.page == super::Page::Agents && !self.closing {
                iced::time::every(std::time::Duration::from_secs(3))
                    .map(|_| Message::LocalAi(super::local_ai_settings::Message::RefreshStatus))
            } else {
                Subscription::none()
            },
            self.files_watch
                .as_ref()
                .map_or_else(Subscription::none, |watch| {
                    watch.subscription().map(|_| Message::FilesChanged)
                }),
            window::close_requests().map(|_| Message::Close),
            window::open_events().map(Message::Opened),
            self.shortcut_capture_subscription(),
            if self.page == Page::Keyboard {
                enzui_shell::shortcut_map::key_labels_subscription().map(|labels| {
                    Message::Shortcuts(shortcuts_settings::Message::KeyLabels(labels))
                })
            } else {
                Subscription::none()
            },
            self.chrome.subscription().map(Message::WindowState),
            self.microphone_window_events(),
            self.themes
                .as_ref()
                .map_or_else(Subscription::none, |themes| {
                    themes
                        .subscription()
                        .map(|event| Message::SharedTheme(Box::new(event)))
                }),
            window::resize_events().map(|(id, size)| Message::ViewportHeight(id, size.height)),
            iced::event::listen_with_interest(
                iced::event::Interest::KEYBOARD,
                |event, status, window| {
                    if let Some(direction) = enzui_core::focus::traversal_from_event(&event, status)
                    {
                        return Some(Message::Traverse(direction, window));
                    }
                    if status == iced::event::Status::Captured {
                        return None;
                    }
                    use iced::keyboard::{Event, Key};
                    let iced::Event::Keyboard(Event::KeyPressed { key, modifiers, .. }) = event
                    else {
                        return None;
                    };
                    (modifiers == iced::keyboard::Modifiers::CTRL
                        && matches!(key, Key::Character(key) if key.eq_ignore_ascii_case("f")))
                    .then_some(Message::FindSetting)
                },
            ),
            numeric::subscription(),
            navigation::interrupt_subscription(),
            self.agent.subscription(),
            self.file_chooser.subscription(),
            self.process_subscription(),
            self.controller_subscription(),
            self.developer_subscription(),
            self.background_subscription(),
            self.remote_subscription(),
            if self.page == Page::Network {
                enzui_shell::network_connectivity::subscription().map(|_| Message::Connectivity)
            } else {
                Subscription::none()
            },
            if self.page == Page::Appearance && self.wallpaper.preview_started.is_some() {
                window::frames().map(|_| Message::WallpaperFrame)
            } else {
                Subscription::none()
            },
            // Navigation moves at the display's rate, like the other reveals.
            if (self.navigation.moving() && self.navigation_ready() && !self.reading)
                || (self.controls.focus_pending.is_some()
                    || (self.reveal_pending && self.reveal_ready())
                    || self.numeric.focus_pending.is_some()
                    || self.numeric.reveal_pending.is_some())
                    && !self.reading
            {
                window::frames().map(|_| Message::RevealFrame)
            } else {
                Subscription::none()
            },
        ])
    }
    pub(super) fn view(&self) -> Element<'_, Message> {
        i18n::localizer().view(|| self.localized_view())
    }
    fn localized_view(&self) -> Element<'_, Message> {
        let theme = self.tokens();
        let _defaults = enzui_theme::text_defaults::TextDefaultsScope::enter(&theme);
        let _scale =
            enzui_theme::text_scale::TextScaleScope::enter(theme.typography.scale_factor());
        ROW_ICON_SCALE.with(|cell| cell.set(f32::from(self.settings.metrics.icon) / 16.0));
        let page = self.window_view(&theme);
        // Mounted while no dialog is open too, so opening one keeps the
        // page's state (its scroll position) under it.
        #[derive(Clone, Copy)]
        enum Open {
            Permissions,
            Plugin,
            Providers,
            ThemeColor,
            Assistant,
            Startup,
            Background,
            BackupShare,
        }
        let (dialog, label, open) = if let Some(dialog) = self.startup_dialog() {
            (Some(dialog), tr("shell-add-startup-app"), Open::Startup)
        } else if let Some(dialog) = self.background_details_dialog() {
            (
                Some(dialog),
                tr("shell-background-activity"),
                Open::Background,
            )
        } else if let Some(dialog) = self.backup_share_dialog() {
            (Some(dialog), tr("shell-network-share"), Open::BackupShare)
        } else if let Some(dialog) = self.app_permissions_dialog() {
            (Some(dialog), tr("shell-app-permissions"), Open::Permissions)
        } else if let Some(dialog) = self.plugin_details_dialog() {
            (Some(dialog), tr("shell-plugin-details"), Open::Plugin)
        } else if let Some(dialog) = self.search_providers_dialog() {
            (
                Some(dialog),
                tr("shell-apps-that-add-results"),
                Open::Providers,
            )
        } else if let Some(dialog) = self.theme_custom_dialog() {
            (Some(dialog), tr("shell-custom-colour"), Open::ThemeColor)
        } else if let Some(dialog) = self.assistant_dialog() {
            (Some(dialog), tr("shell-assistant"), Open::Assistant)
        } else {
            (None, "", Open::Permissions)
        };
        let generation = self.sandbox_generation();
        let host =
            enzui_widgets::modal_host::enz_modal_host(page, dialog, move |event| match open {
                Open::Permissions => {
                    Message::Sandbox(sandbox_settings::Message::Host(generation, event))
                }
                Open::Plugin => Message::Plugin(plugins_settings::Message::Host(event)),
                Open::Providers => {
                    Message::SearchSettings(quick_search_settings::Message::ProvidersHost(event))
                }
                Open::ThemeColor => Message::ThemeEdit(theme_settings::Edit::Host(event)),
                Open::Assistant => Message::Assistant(assistant_settings::Message::Host(event)),
                Open::Startup => Message::Startup(settings_startup::Message::Host(event)),
                Open::Background => {
                    Message::Background(background_settings::Message::DetailHost(event))
                }
                Open::BackupShare => system_pages::backup_share_host(event),
            })
            .label(label);
        let page = match self
            .sandbox_focus()
            .filter(|_| matches!(open, Open::Permissions))
        {
            Some(focus) => host.scoped_to(focus).into_element(),
            None => host.into_element(),
        };
        let mut notices = Vec::new();
        if let Some(error) = &self.error {
            notices.push(
                enzui_widgets::notice_stack::EnzNotice::new(
                    "settings-error",
                    i18n::label(error).to_owned(),
                    enzui_widgets::toast::ToastTone::Error,
                )
                .on_dismiss(Message::DismissError(error.clone())),
            );
        }
        if let Some(confirm) = self.display_confirm_notice() {
            notices.push(confirm);
        }
        let overlay = (!notices.is_empty())
            .then(|| enzui_widgets::notice_stack::enz_notice_stack(notices).into_element());
        let page = enzui_widgets::toast_host::enz_toast_layer(
            self.background_host(page),
            overlay,
            enzui_widgets::toast_host::ToastOverlayPlacement::default().trailing(),
        );
        self.file_chooser.view(page)
    }
    fn window_view(&self, theme: &EnzTheme) -> Element<'_, Message> {
        use enzui_widgets::{app_shell::ShellTokens, sidebar_row::SidebarRowTokens};
        let scale = theme.typography.scale_factor().factor();
        let row_tokens = SidebarRowTokens::from_palette(&theme.palette);
        let sidebar_theme = theme.clone();
        let top_level_tokens = SidebarRowTokens {
            row_padding_x: 4.0,
            icon_gap: 3.0,
            row_height: (36.0 * scale).max(36.0),
            ..row_tokens
        };
        enz_app_shell(|| self.page_view())
            .layout(ShellLayout::TitlebarSidebarContent)
            .tokens(ShellTokens::default().sidebar_width(theme.sizing.sidebar_width + 24.0))
            .breakpoints(theme.breakpoints)
            .on_window_gesture(Message::Window)
            .maximized(self.chrome.maximized())
            .fullscreen(self.chrome.fullscreen())
            .titlebar(|| {
                enz_title_bar()
                    .center(
                        {
                            let field = enzui_widgets::search_field::enz_titlebar_search(
                                &self.filter,
                                Message::Filter,
                            );
                            use enzui_shell::settings_catalog::SearchDestination;
                            match self.first_search_destination() {
                                Some(SearchDestination::Page(page)) => {
                                    field.on_submit(Message::Page(page))
                                }
                                Some(SearchDestination::Section(route)) => {
                                    field.on_submit(Message::RevealSection(route))
                                }
                                None => field,
                            }
                        }
                        .placeholder(tr("shell-find-a-setting"))
                        .id(SEARCH_FIELD)
                        .into_element(),
                    )
                    .id("settings-window")
                    .into_element()
            })
            .sidebar_titlebar(|| enz_title_bar().title(tr("shell-settings")).into_element())
            .sidebar(move || {
                use enzui_shell::settings_catalog::Category;
                let theme = &sidebar_theme;
                let mut categories = enz_sidebar_content().spacing_scale(theme.spacing);
                let mut group = None;
                let mut folder = None;
                for category in Category::ALL {
                    if !self.layout_preview && category == Category::Notifications
                        || !self.category_available(category)
                    {
                        continue;
                    }
                    if group != Some(category.group()) {
                        if group.is_some() {
                            categories = categories.push(
                                iced::widget::container(
                                    enzui_widgets::divider::enz_divider().into_element(),
                                )
                                .padding([6, 8]),
                            );
                        }
                        group = Some(category.group());
                        categories = categories.push(
                            iced::widget::container(
                                enzui_widgets::section_header::enz_section_header::<Message>(
                                    category.group().title(),
                                )
                                .title_size(self.text_px(11.0))
                                .subtle_on(theme.palette.surface)
                                .into_element(),
                            )
                            .padding(
                                iced::Padding::default()
                                    .left(12.0)
                                    .top(if group == Some(Category::ALL[0].group()) {
                                        0.0
                                    } else {
                                        2.0
                                    })
                                    .bottom(2.0),
                            ),
                        );
                    }
                    let sections = self.sections(category);
                    match category.folder() {
                        // A page with one section has nothing to expand: an expander
                        // would only open onto a row repeating the page's own name.
                        None if sections.len() <= 1 => {
                            categories = categories.push(
                                enz_sidebar_row(
                                    category.title(),
                                    Message::Category(category, true),
                                )
                                .tokens(top_level_tokens)
                                .icon(category.icon())
                                .id(format!("settings-category-{}", category.page()).into())
                                .selected(self.page == category && self.filter.trim().is_empty())
                                .enabled(!self.closing)
                                .into_element(),
                            );
                        }
                        None => {
                            let active = self.page == category;
                            let expanded = self.navigation.expanded == Some(category);
                            categories = categories.push(self.sidebar_expander(
                                category.title(),
                                category.icon(),
                                category.page(),
                                active,
                                expanded,
                                Message::Category(category, true),
                                self.sidebar_sections(sections, 16.0, row_tokens, &theme.palette),
                                theme,
                            ));
                        }
                        Some(current) if folder == Some(current) => {}
                        Some(current) => {
                            folder = Some(current);
                            let mut pages = iced::widget::column![].spacing(2);
                            for page in current
                                .pages()
                                .filter(|page| self.category_available(*page))
                            {
                                let sections = self.sections(page);
                                pages = pages.push(
                                    enz_sidebar_row(page.title(), Message::Page(page))
                                        .tokens(row_tokens)
                                        .icon(page.icon())
                                        .leading_indent(8.0)
                                        .id(format!("settings-category-{}", page.page()).into())
                                        .selected(
                                            self.page == page
                                                && self.filter.trim().is_empty()
                                                && (self.navigation.selected.is_none()
                                                    || sections.len() <= 1),
                                        )
                                        .enabled(!self.closing)
                                        .into_element(),
                                );
                                // A single section would only repeat the page's own row.
                                if self.page == page && sections.len() > 1 {
                                    pages = pages.push(self.sidebar_sections(
                                        sections,
                                        30.0,
                                        row_tokens,
                                        &theme.palette,
                                    ));
                                }
                            }
                            let active = self.page.folder() == Some(current);
                            let expanded = self
                                .navigation
                                .expanded
                                .is_some_and(|open| open.folder() == Some(current));
                            let first = current
                                .pages()
                                .find(|page| self.category_available(*page))
                                .unwrap_or(category);
                            categories = categories.push(self.sidebar_expander(
                                current.title(),
                                current.icon(),
                                current.id(),
                                active,
                                expanded,
                                Message::Category(if active { self.page } else { first }, true),
                                pages,
                                theme,
                            ));
                        }
                    }
                }
                enzui_widgets::scroll_view::enz_scroll_view(categories.into_element())
                    .id("settings-navigation".into())
                    .scrollbar_tokens(theme.scrollbar)
                    .backdrop(theme.palette.surface)
                    .into_element()
            })
            .state(self.nav)
            .sidebar_label(tr("shell-settings-categories"))
            .on_sidebar_state(Message::Sidebar)
            .into()
    }
    fn sidebar_sections(
        &self,
        entries: Vec<enzui_shell::settings_catalog::Section>,
        indent: f32,
        tokens: enzui_widgets::sidebar_row::SidebarRowTokens,
        palette: &enzui_theme::Palette,
    ) -> iced::widget::Column<'_, Message> {
        use enzui_core::semantics::{Role, Semantics};
        use enzui_widgets::{
            enz_text,
            pressable::{PressableStyle, PressableTokens, enz_pressable},
        };
        use iced::{Length, Padding, widget::container};

        let surface = PressableTokens {
            row_chip_selected_bg: tokens.selected_background,
            ..PressableTokens::from_palette(palette)
        };
        let enabled = !self.closing;
        let searching = !self.filter.trim().is_empty();
        iced::widget::column(entries.into_iter().map(|section| {
            let label = tr(section.label);
            let selected = !searching && self.navigation.selected == Some(section.route);
            let mut semantics = Semantics::new(Role::ListItem)
                .label(label)
                .selected(selected);
            if enabled {
                semantics = semantics.default_action();
            }
            let text = enz_text(label)
                .size_px(tokens.label_size)
                .wrap(true)
                .width(Length::Fill)
                .color(if !enabled {
                    tokens.disabled
                } else if selected {
                    tokens.selected_text
                } else {
                    tokens.text
                });
            let row = enz_pressable(text.into_element(), Message::Section(section.route))
                .style(PressableStyle::Row)
                .tokens(surface)
                .padding(
                    Padding::from([5.0, tokens.row_padding_x]).left(indent + tokens.row_padding_x),
                )
                .min_height(tokens.row_height - 2.0 * tokens.fill_inset_y)
                .radius(tokens.row_radius)
                .selected(selected)
                .enabled(enabled)
                .semantics(semantics)
                .id(format!("settings-section-{}", section.route).into());
            container(row.into_element())
                .padding([tokens.fill_inset_y, tokens.outer_gutter])
                .width(Length::Fill)
                .into()
        }))
        .spacing(2)
    }
    #[allow(
        clippy::too_many_arguments,
        reason = "The view composes independent state and actions supplied by its caller."
    )]
    fn sidebar_expander<'a>(
        &'a self,
        title: &'static str,
        icon: &'static str,
        id: &'static str,
        active: bool,
        expanded: bool,
        open: Message,
        content: iced::widget::Column<'a, Message>,
        theme: &EnzTheme,
    ) -> Element<'a, Message> {
        let palette = &theme.palette;
        let scale = theme.typography.scale_factor().factor();
        let active = active && self.filter.trim().is_empty();
        let mut tokens = enzui_widgets::expander::ExpanderTokens::from_palette(palette);
        tokens.header_height = (36.0 * scale).max(36.0);
        tokens.title_size *= scale;
        tokens.subtitle_size *= scale;
        tokens.content_inset = 8.0;
        tokens.content_gap = 4.0;
        tokens.card_radius = enzui_theme::text_defaults::shape_radius(|shape| shape.card);
        tokens.background = if active {
            palette.surface
        } else {
            iced::Color::TRANSPARENT
        };
        tokens.title_color = if active {
            palette.text
        } else {
            palette.text_muted
        };
        enzui_widgets::expander::enz_expander(
            title,
            expanded && self.filter.trim().is_empty(),
            move |expanded| {
                if expanded {
                    open.clone()
                } else {
                    Message::Collapse
                }
            },
        )
        .icon(icon)
        .id(format!("settings-category-{id}"))
        .tokens(tokens)
        .content(content.into())
        .into_element()
    }
    pub(super) fn window_title(&self) -> String {
        tr!("shell-settings-4", "arg0" => (if self.filter.trim().is_empty() {
            self.page.title()
        } else {
            tr("shell-search")
        }).to_owned())
    }
    pub(super) fn page_view(&self) -> Element<'_, Message> {
        if self.picker.is_some() {
            self.wallpaper_picker_view()
        } else {
            self.settings_page()
        }
    }
}

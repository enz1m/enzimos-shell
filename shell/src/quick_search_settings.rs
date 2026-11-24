use super::{Message as ParentMessage, SettingsApp};
use enzui_shell::i18n::tr;
use enzui_shell::tr;
use enzui_shell::{
    search::preferences::{Category, Change, Shortcut, WebProvider},
    settings::Change as SettingChange,
};
use enzui_widgets::{
    button::{ButtonKind, enz_button},
    keyed_column::keyed_column,
    text::{TextSize, enz_text},
};
use iced::{
    Element, Task,
    widget::{column, row},
};

#[derive(Default)]
pub(super) struct State {
    checking: bool,
    // Apps that add search results: (desktop ID, name, made for this
    // desktop), read once.
    providers: std::cell::OnceCell<Vec<(String, String, bool)>>,
    // The dialog listing them, when there are too many for the page.
    providers_open: bool,
}

// More apps than this are chosen in a dialog instead of on the page.
const INLINE_PROVIDERS: usize = 4;

fn provider_apps() -> Vec<(String, String, bool)> {
    use enzui_shell::search::providers;
    let dirs = providers::data_dirs();
    let mut apps: Vec<(String, String, bool)> = providers::discover(&dirs)
        .into_iter()
        .filter_map(|declaration| {
            let name = providers::app_name(&declaration.desktop_id, &dirs)?;
            Some((
                declaration.desktop_id,
                name,
                declaration.protocol == providers::Protocol::Enzui,
            ))
        })
        .collect();
    // Apps made for this desktop first, then GNOME ones, each by name.
    apps.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });
    apps
}
#[derive(Clone, Debug)]
pub(super) enum Message {
    // Choose a folder to search (false) or to leave out (true).
    Choose(bool),
    Add(bool, String),
    Checked(bool, String, Result<(), String>),
    Providers(bool),
    ProvidersHost(enzui_widgets::modal_host::ModalHostEvent),
}
fn change(value: Change) -> ParentMessage {
    ParentMessage::Change(SettingChange::Search(value))
}
impl SettingsApp {
    pub(super) fn search_providers_dialog(&self) -> Option<Element<'_, ParentMessage>> {
        let apps = self.search.providers.get()?;
        if !self.search.providers_open || apps.len() <= INLINE_PROVIDERS {
            return None;
        }
        let p = &self.settings.quick_search;
        let palette = self.tokens().palette;
        let mut rows = column![].spacing(2);
        for (index, (id, name, native)) in apps.iter().enumerate() {
            let key = id.clone();
            let mut row = crate::settings_app::settings_row(name.clone());
            row = row.subtitle(if *native {
                tr("shell-made-for-this-desktop")
            } else {
                tr("shell-gnome-app-answers-while-running")
            });
            rows = rows.push(
                row.id_prefix(format!("control-quick-search-provider-{index}"))
                    .switch(p.provider_on(id, *native), move |on| {
                        change(Change::Provider(key.clone(), on))
                    })
                    .enabled(self.controls_live())
                    .into_element(),
            );
        }
        let header: Element<'_, ParentMessage> = column![
            enz_text(tr("shell-apps-that-add-results"))
                .size_px(self.text_px(18.0))
                .weight_rung(enzui_theme::WeightRung::Semibold)
                .into_element(),
            enz_text(tr(
                "shell-search-sends-what-you-type-to-the-apps-switched-on-here-while-they-run"
            ))
            .size_px(self.text_px(12.0))
            .muted_on(palette.surface)
            .wrap(true)
            .into_element(),
        ]
        .spacing(4)
        .into();
        let close = ParentMessage::SearchSettings(Message::Providers(false));
        let scroll = enzui_widgets::scroll_view::enz_scroll_view(Element::from(
            iced::widget::container(rows).padding(iced::Padding {
                right: 12.0,
                ..Default::default()
            }),
        ))
        .id("quick-search-providers-body".into())
        .scrollbar_tokens(self.tokens().scrollbar)
        .into_element();
        let footer: Element<'_, ParentMessage> = row![
            iced::widget::space::horizontal(),
            enz_button(tr("shell-done"), close.clone())
                .kind(ButtonKind::Primary)
                .id("quick-search-providers-done".into())
                .into_element(),
        ]
        .into();
        let content = enzui_widgets::capped_dialog_body::enz_capped_dialog_body(
            vec![header],
            scroll,
            vec![footer],
            16.0,
            560.0,
        )
        .into_element();
        Some(
            enzui_widgets::modal_surface::enz_modal_surface(content)
                .width(480.0)
                .padding(20.0)
                .on_dismiss(move || close.clone())
                .close_label(tr("shell-close-apps-that-add-results"))
                .into_element(),
        )
    }

    pub(super) fn search_settings_message(&mut self, message: Message) -> Task<ParentMessage> {
        match message {
            Message::Providers(open) => self.search.providers_open = open,
            Message::ProvidersHost(enzui_widgets::modal_host::ModalHostEvent::Dismissed(_)) => {
                self.search.providers_open = false;
            }
            Message::ProvidersHost(event) => {
                return enzui_widgets::modal_host::focus_wrap_operation(event)
                    .map_or_else(Task::none, iced::advanced::widget::operate);
            }
            Message::Choose(excluded) if !self.search.checking && !self.closing => {
                return Task::done(ParentMessage::FileChooser(
                    super::settings_file_chooser::Message::OpenSearchFolder(excluded),
                ));
            }
            Message::Add(excluded, path) if !self.search.checking && !self.closing => {
                let path = path.trim_end_matches('/').to_owned();
                if !std::path::Path::new(&path).is_absolute()
                    || path.is_empty()
                    || path.chars().any(char::is_control)
                {
                    self.error = Some(tr("shell-choose-a-folder-other-than-the-whole-disk").into());
                    return Task::none();
                }
                let p = &self.settings.quick_search;
                let listed = if excluded { &p.exclusions } else { &p.roots };
                if listed.contains(&path) {
                    return Task::none();
                }
                self.search.checking = true;
                let input = path.clone();
                return Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            rustix::fs::openat2(
                                rustix::fs::CWD,
                                input.as_str(),
                                rustix::fs::OFlags::RDONLY
                                    | rustix::fs::OFlags::DIRECTORY
                                    | rustix::fs::OFlags::CLOEXEC,
                                rustix::fs::Mode::empty(),
                                rustix::fs::ResolveFlags::NO_SYMLINKS
                                    | rustix::fs::ResolveFlags::NO_MAGICLINKS,
                            )
                            .map(|_| ())
                            .map_err(|_| {
                                tr("shell-folder-is-unreadable-or-contains-a-symbolic-link")
                                    .to_owned()
                            })
                        })
                        .await
                        .map_err(|_| tr("shell-folder-check-stopped").to_owned())
                        .and_then(|result| result)
                    },
                    move |result| {
                        ParentMessage::SearchSettings(Message::Checked(excluded, path, result))
                    },
                );
            }
            Message::Checked(excluded, path, result) => {
                self.search.checking = false;
                if self.closing {
                    return Task::none();
                }
                match result {
                    Ok(()) => {
                        return self.update(change(if excluded {
                            Change::ExcludedRoot(path, true)
                        } else {
                            Change::Root(path, true)
                        }));
                    }
                    Err(error) => self.error = Some(error),
                }
            }
            _ => {}
        }
        Task::none()
    }
    pub(super) fn quick_search_settings(&self) -> Element<'_, ParentMessage> {
        let p = &self.settings.quick_search;
        let enabled = self.controls_live();
        let mut body = column![
            self.section_card_for(
                "quick-search/enabled",
                column![
                    self.switch_setting_noted(
                        "quick-search/enabled",
                        p.enabled,
                        tr("shell-when-off-the-shortcut-opens-the-classic-app-launcher"),
                        |v| change(Change::Enabled(v))
                    ),
                    self.choice_setting(
                        "quick-search/shortcut",
                        p.shortcut.label(),
                        Shortcut::ALL
                            .into_iter()
                            .map(|value| (value.label().into(), change(Change::Shortcut(value))))
                            .collect()
                    ),
                    self.dependent(p.enabled, || column![
                        self.switch_setting("quick-search/animations", p.animations, |v| change(
                            Change::Animations(v)
                        )),
                        self.switch_setting(
                            "quick-search/background_fade",
                            p.background_fade,
                            |v| change(Change::BackgroundFade(v))
                        ),
                        self.switch_setting("quick-search/compact", p.compact, |v| change(
                            Change::Compact(v)
                        )),
                        self.number_setting("quick-search/rows", p.rows.into(), 4, 12, "", |v| {
                            change(Change::Rows(v as u8))
                        }),
                    ]
                    .spacing(2)),
                ]
                .spacing(2)
            )
            .title(tr("shell-quick-search"))
            .description(tr(
                "shell-find-apps-windows-settings-files-and-more-from-one-box"
            ))
            .into_element(),
        ]
        .spacing(32);
        let mut sources = column![].spacing(2);
        for category in Category::ALL {
            let key =
                enzui_shell::settings_catalog::find(&format!("quick-search/{}", category.key()))
                    .expect("the search category belongs to the settings catalog")
                    .route;
            let description = match category {
                Category::Apps => tr("shell-installed-applications"),
                Category::Windows => tr("shell-open-windows-and-pinned-apps"),
                Category::Actions => tr("shell-settings-and-desktop-actions"),
                Category::Calculator => tr("shell-sums-and-unit-conversions"),
                Category::Files => tr("shell-file-names-in-the-folders-you-choose-below"),
                Category::InApps => tr("shell-messages-events-and-more-from-apps-that-offer-them"),
                Category::Clipboard => tr("shell-text-and-images-you-copied-recently"),
                Category::Web => tr("shell-searches-the-web-only-when-you-open-a-web-result"),
            };
            sources = sources.push(self.dependent(p.enabled, || {
                self.mark_setting(
                    key,
                    crate::settings_app::settings_row(category.label())
                        .icon(enzui_shell::settings_catalog::icon(key).unwrap_or(""))
                        .subtitle(description)
                        .id_prefix(format!("control-{key}"))
                        .switch(p.categories.contains(&category), move |v| {
                            change(Change::Category(category, v))
                        })
                        .enabled(self.controls_live())
                        .into_element(),
                )
            }));
        }
        sources = sources.push(self.dependent(
            p.enabled && p.categories.contains(&Category::Web),
            || {
                self.choice_setting(
                    "quick-search/web-provider",
                    p.web_provider.label(),
                    WebProvider::ALL
                        .into_iter()
                        .map(|value| (value.label().into(), change(Change::WebProvider(value))))
                        .collect(),
                )
            },
        ));
        body = body.push(
            self.section_card_for("quick-search/apps", sources)
                .title(tr("shell-sources"))
                .into_element(),
        );
        let apps = self.search.providers.get_or_init(provider_apps);
        let mut providers = column![].spacing(2);
        let in_dialog = apps.len() > INLINE_PROVIDERS;
        if in_dialog {
            let on = apps
                .iter()
                .filter(|(id, _, native)| p.provider_on(id, *native))
                .count();
            let row = self.dependent(p.enabled && p.categories.contains(&Category::InApps), || {
                crate::settings_app::settings_row(tr("shell-apps-that-add-results"))
                    .icon("blocks")
                    .subtitle(tr!("shell-of-on", "arg0" => format!("{:}", on), "arg1" => format!("{:}", apps.len())))
                    .id_prefix("control-quick-search-providers")
                    .control(
                        enz_button(tr("shell-choose-apps"), ParentMessage::SearchSettings(Message::Providers(true)))
                            .kind(ButtonKind::Quiet)
                            .id("quick-search-providers-choose".into())
                            .enabled(self.controls_live())
                            .into_element(),
                    )
                    .into_element()
            });
            providers = providers.push(self.mark_setting("quick-search/providers", row));
        }
        if apps.is_empty() {
            providers = providers.push(self.mark_setting(
                "quick-search/providers",
                crate::settings_app::settings_row(tr("shell-no-apps-add-results-yet"))
                    .icon("blocks")
                    .subtitle(tr("shell-apps-that-can-search-inside-themselves-like-a-mail-or-calendar-app-appear-he"))
                    .into_element(),
            ));
        }
        for (index, (id, name, native)) in apps.iter().enumerate().filter(|_| !in_dialog) {
            let key = id.clone();
            let row = self.dependent(
                p.enabled && p.categories.contains(&Category::InApps),
                || {
                    let mut row = crate::settings_app::settings_row(name.clone());
                    if !native {
                        row = row.subtitle(tr(
                            "shell-gnome-app-off-until-you-turn-it-on-answers-while-running",
                        ));
                    }
                    row.id_prefix(format!("control-quick-search-provider-{index}"))
                        .switch(p.provider_on(id, *native), move |on| {
                            change(Change::Provider(key.clone(), on))
                        })
                        .enabled(self.controls_live())
                        .into_element()
                },
            );
            providers = providers.push(if index == 0 {
                self.mark_setting("quick-search/providers", row)
            } else {
                row
            });
        }
        body = body.push(
            self.section_card_for("quick-search/providers", providers)
                .title(tr("shell-apps-in-search"))
                .description(tr("shell-apps-switched-on-here-see-what-you-type-in-search-so-they-can-show-matching"))
                .into_element(),
        );
        let files_on = p.enabled && p.categories.contains(&Category::Files);
        let mut folders = column![].spacing(2);
        for (excluded, label, roots, route) in [
            (
                false,
                tr("shell-search-these-folders"),
                &p.roots,
                "quick-search/roots",
            ),
            (
                true,
                tr("shell-leave-these-folders-out"),
                &p.exclusions,
                "quick-search/exclusions",
            ),
        ] {
            let mut listed = vec![(
                "label".to_owned(),
                iced::widget::container(
                    enz_text(label)
                        .size(TextSize::Footnote)
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .muted_on(self.tokens().palette.surface)
                        .into_element(),
                )
                .padding(iced::Padding {
                    top: if excluded { 14.0 } else { 0.0 },
                    bottom: 4.0,
                    left: 10.0,
                    right: 10.0,
                })
                .into(),
            )];
            for path in roots {
                let removal = if excluded {
                    Change::ExcludedRoot(path.clone(), false)
                } else {
                    Change::Root(path.clone(), false)
                };
                let name = std::path::Path::new(path)
                    .file_name()
                    .map_or_else(|| path.clone(), |name| name.to_string_lossy().into_owned());
                listed.push((
                    format!("root:{path}"),
                    self.dependent(files_on, || {
                        crate::settings_app::settings_row(name)
                            .subtitle(path.clone())
                            .icon(if excluded { "folder-x" } else { "folder" })
                            .control(
                                enzui_widgets::toolbar_icon::enz_toolbar_icon("x", change(removal))
                                    .aria_label(tr!("shell-remove-3", "arg0" => path.clone()))
                                    .tooltip(tr("shell-remove"))
                                    .enabled(self.controls_live())
                                    .into_element(),
                            )
                            .into_element()
                    }),
                ));
            }
            let full = roots.len() >= 8;
            listed.push((
                "add".to_owned(),
                self.dependent(files_on && !full && !self.search.checking, || {
                    crate::settings_app::settings_row(if excluded {
                        tr("shell-leave-out-a-folder")
                    } else {
                        tr("shell-add-a-folder")
                    })
                    .icon("plus")
                    .subtitle(if full {
                        tr("shell-up-to-8-folders")
                    } else {
                        ""
                    })
                    .action("", ParentMessage::SearchSettings(Message::Choose(excluded)))
                    .id((if excluded {
                        "search-excluded-folder"
                    } else {
                        "search-include-folder"
                    })
                    .into())
                    .enabled(self.controls_live())
                    .into_element()
                }),
            ));
            folders = folders.push(self.mark_setting(route, keyed_column(listed).spacing(2.0)));
        }
        body = body.push(
            self.section_card_for("quick-search/roots", folders)
                .title(tr("shell-filename-search"))
                .description(tr(
                    "shell-files-finds-names-in-these-folders-hidden-files-are-skipped",
                ))
                .into_element(),
        );
        let clipboard_on = p.enabled && p.categories.contains(&Category::Clipboard);
        body = body.push(
            self.section_card_for(
                "quick-search/recent",
                column![
                    self.switch_setting("quick-search/recent", p.recent, |v| change(
                        Change::Recent(v)
                    )),
                    // Turning it off already clears the counts.
                    self.dependent(p.recent, || self.action_setting(
                        "quick-search/clear-usage",
                        enzui_shell::i18n::tr("shell-clear"),
                        "trash-2",
                        ButtonKind::QuietDestructive,
                        change(Change::ClearUsage),
                    )),
                    self.dependent(clipboard_on, || self.number_setting(
                        "quick-search/clipboard-retention",
                        p.clipboard_minutes.into(),
                        1,
                        1440,
                        " min",
                        |v| change(Change::ClipboardMinutes(v as u16))
                    )),
                    self.dependent(clipboard_on, || self.action_setting(
                        "quick-search/clear-clipboard",
                        enzui_shell::i18n::tr("shell-clear"),
                        "trash-2",
                        ButtonKind::QuietDestructive,
                        change(Change::ClearClipboard)
                    )),
                ]
                .spacing(2),
            )
            .title(enzui_shell::i18n::tr("shell-ranking-privacy"))
            .description(tr(
                "shell-search-learns-which-apps-and-actions-you-open-most-never-what-you-type",
            ))
            .into_element(),
        );
        let mut entries = Vec::new();
        for (key, target) in &p.aliases {
            entries.push((
                format!("alias:{key}:{target}"),
                crate::settings_app::settings_row(target.clone())
                    .subtitle(tr!("shell-quick-key", "arg0" => key.clone()))
                    .icon("keyboard")
                    .control(
                        enzui_widgets::toolbar_icon::enz_toolbar_icon(
                            "x",
                            change(Change::Alias(key.clone(), None)),
                        )
                        .aria_label(tr!("shell-remove-3", "arg0" => target.clone()))
                        .tooltip(tr("shell-remove"))
                        .enabled(enabled)
                        .into_element(),
                    )
                    .into_element(),
            ));
        }
        for target in &p.favorites {
            entries.push((
                format!("favorite:{target}"),
                crate::settings_app::settings_row(target.clone())
                    .subtitle(tr("shell-favorite"))
                    .icon("star")
                    .control(
                        enzui_widgets::toolbar_icon::enz_toolbar_icon(
                            "x",
                            change(Change::Favorite(target.clone(), false)),
                        )
                        .aria_label(tr!("shell-remove-3", "arg0" => target.clone()))
                        .tooltip(tr("shell-remove"))
                        .enabled(enabled)
                        .into_element(),
                    )
                    .into_element(),
            ));
        }
        if entries.is_empty() {
            entries.push((
                "empty".to_owned(),
                crate::settings_app::settings_row(tr("shell-no-quick-keys-or-favorites-yet"))
                    .subtitle(tr("shell-in-search-select-a-result-then-give-it-a-quick-key-or-make-it-a-favorite"))
                    .icon("star")
                    .into_element(),
            ));
        }
        body = body.push(self.mark_setting(
            "quick-search/aliases",
            self.section_card_for("quick-search/aliases", keyed_column(entries).spacing(2.0))
                .title(tr("shell-quick-keys-favorites"))
                .description(tr("shell-results-you-reach-for-often-a-quick-key-selects-a-result-without-opening-it"))
                .into_element(),
        ));
        body.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chosen folder is checked before it is listed: the whole disk is
    /// refused, a folder already listed is not added twice, and a new one
    /// starts the check.
    ///
    /// Sabotage: drop the root or duplicate checks in `Message::Add`.
    #[test]
    fn chosen_folders_are_checked_once_and_never_the_whole_disk() {
        let directory = tempfile::tempdir().unwrap();
        let (mut app, _) = SettingsApp::new(
            crate::settings_app::Store::at(directory.path().join("settings.json")).unwrap(),
        );
        let _ = app.search_settings_message(Message::Add(false, "/".into()));
        assert!(app.error.is_some() && !app.search.checking);
        app.error = None;
        let folder = directory.path().to_str().unwrap().to_owned();
        std::sync::Arc::make_mut(&mut app.settings.quick_search)
            .roots
            .push(folder.clone());
        let _ = app.search_settings_message(Message::Add(false, format!("{folder}/")));
        assert!(
            app.error.is_none() && !app.search.checking,
            "already listed"
        );
        let _ = app.search_settings_message(Message::Add(true, folder));
        assert!(app.search.checking, "a new exclusion is checked");
    }
}

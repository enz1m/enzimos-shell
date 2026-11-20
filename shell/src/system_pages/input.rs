use super::*;
use enzui_shell::i18n::tr;
use enzui_shell::settings::Change;
use enzui_shell::tr;
use enzui_widgets::{
    button::{ButtonKind, ButtonSize, enz_button},
    toolbar_icon::enz_toolbar_icon,
};
use iced::widget::{column, row};

impl SettingsApp {
    pub(super) fn pointer_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let mut rows = self.compositor_rows();
        let mut take = |route: &'static str| {
            rows.remove(route)
                .expect("the input catalog contains each displayed row")
        };
        let mouse = column![
            take("windows/pointer_speed"),
            take("windows/pointer_acceleration"),
            take("windows/left_handed")
        ]
        .spacing(2);
        let scrolling =
            column![take("windows/natural_scroll"), take("windows/scroll_speed")].spacing(2);
        let touchpad = column![
            take("windows/tap_to_click"),
            take("windows/disable_while_typing"),
            take("windows/gestures")
        ]
        .spacing(2);
        let body = body
            .push(
                self.section_card_for("windows/pointer_speed", mouse)
                    .title(tr("shell-mouse"))
                    .description(tr(
                        "shell-turn-acceleration-off-for-consistent-one-to-one-movement",
                    ))
                    .into_element(),
            )
            .push(
                self.section_card_for("windows/natural_scroll", scrolling)
                    .title(tr("shell-scrolling"))
                    .description(tr(
                        "shell-natural-scrolling-moves-content-with-your-fingers-like-a-phone",
                    ))
                    .into_element(),
            )
            .push(
                self.section_card_for("windows/tap_to_click", touchpad)
                    .title(tr("shell-touchpad"))
                    .description(tr("shell-applies-to-built-in-and-external-touchpads"))
                    .into_element(),
            );
        match self.desktop_status_note() {
            Some(note) => body.push(note),
            None => body,
        }
    }
    fn layouts_view<'a>(&'a self) -> Element<'a, Parent> {
        let route = "keyboard/layouts";
        let Some((all, system)) = &self.system.layouts else {
            return self.mark_setting(route, self.loading_row());
        };
        let name = |code: &str| {
            all.iter()
                .find(|(c, _)| c == code)
                .map_or_else(|| code.to_uppercase(), |(_, n)| n.clone())
        };
        let current: Vec<String> = self
            .settings
            .compositor
            .layouts()
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut rows = column![].spacing(2);
        if current.is_empty() {
            let code = system.clone().unwrap_or_else(|| "us".into());
            rows = rows.push(
                self.mark_setting(
                    route,
                    crate::settings_app::settings_row(name(&code))
                        .subtitle(tr("shell-this-computer-s-keyboard-layout"))
                        .icon("keyboard")
                        .into_element(),
                ),
            );
        }
        for (index, code) in current.iter().enumerate() {
            let mut controls = row![].spacing(6).align_y(iced::Alignment::Center);
            if index > 0 {
                let mut reordered = current.clone();
                let moved = reordered.remove(index);
                reordered.insert(0, moved);
                controls = controls.push(
                    enz_button(
                        tr("shell-make-default"),
                        Parent::Change(Change::KeyboardLayouts(reordered)),
                    )
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .id(format!("layout-default-{code}").into())
                    .enabled(self.controls_live())
                    .into_element(),
                );
            }
            // Removing the last one goes back to this computer's layout.
            {
                let mut remaining = current.clone();
                remaining.remove(index);
                controls = controls.push(
                    enz_toolbar_icon("x", Parent::Change(Change::KeyboardLayouts(remaining)))
                        .aria_label(tr!("shell-remove-3", "arg0" => name(code).clone()))
                        .tooltip(tr("shell-remove"))
                        .id(format!("layout-remove-{code}").into())
                        .enabled(self.controls_live())
                        .into_element(),
                );
            }
            let row = crate::settings_app::settings_row(name(code))
                .subtitle(if index == 0 {
                    tr("shell-default").to_owned()
                } else {
                    code.to_uppercase()
                })
                .icon("keyboard")
                .control(controls)
                .into_element();
            rows = rows.push(if index == 0 {
                self.mark_setting(route, row)
            } else {
                row
            });
        }
        if current.len() < enzui_shell::desktop_preferences::MAX_LAYOUTS {
            let base = if current.is_empty() {
                vec![system.clone().unwrap_or_else(|| "us".into())]
            } else {
                current
            };
            match &self.system.layout_query {
                None => {
                    rows = rows.push(
                        crate::settings_app::settings_row(tr("shell-add-a-layout"))
                            .icon("plus")
                            .action(
                                "",
                                Parent::System(Message::LayoutQuery(Some(String::new()))),
                            )
                            .id("keyboard-add-layout".into())
                            .enabled(self.controls_live())
                            .into_element(),
                    );
                }
                Some(query) => rows = rows.push(self.layout_picker(all, &base, query)),
            }
        }
        rows.into()
    }
    fn layout_picker<'a>(
        &'a self,
        all: &'a [(String, String)],
        base: &[String],
        query: &'a str,
    ) -> Element<'a, Parent> {
        let needle = query.trim().to_lowercase();
        let matches: Vec<&(String, String)> = all
            .iter()
            .filter(|(code, label)| {
                code != "custom"
                    && !base.contains(code)
                    && (needle.is_empty()
                        || label.to_lowercase().contains(&needle)
                        || *code == needle)
            })
            .take(6)
            .collect();
        let mut picker = column![
            row![
                enzui_widgets::search_field::enz_search_field(query, |q| Parent::System(
                    Message::LayoutQuery(Some(q))
                ))
                .placeholder(tr("shell-search-by-language-or-country"))
                .id("keyboard-layout-search")
                .into_element(),
                enz_button(
                    tr("shell-cancel"),
                    Parent::System(Message::LayoutQuery(None))
                )
                .kind(ButtonKind::Plain)
                .id("keyboard-layout-cancel".into())
                .into_element(),
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center),
        ]
        .spacing(4);
        if matches.is_empty() {
            picker = picker.push(self.caption(tr("shell-no-layouts-match")));
        }
        for (code, label) in matches {
            let mut next = base.to_vec();
            next.push(code.clone());
            picker = picker.push(
                crate::settings_app::settings_row(label.clone())
                    .subtitle(code.to_uppercase())
                    .action(tr("shell-add-2"), Parent::System(Message::SetLayouts(next)))
                    .id(format!("keyboard-layout-add-{code}").into())
                    .enabled(self.controls_live())
                    .into_element(),
            );
        }
        iced::widget::container(picker).padding([8, 12]).into()
    }
    pub(super) fn keyboard_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let mut rows = self.compositor_rows();
        let mut take = |route: &'static str| {
            rows.remove(route)
                .expect("the input catalog contains each displayed row")
        };
        let typing = column![
            take("windows/repeat_delay"),
            take("windows/repeat_rate"),
            take("windows/caps_lock")
        ]
        .spacing(2);
        let several = self.settings.compositor.layouts().len() > 1;
        // Shown inactive without the OS's Fcitx5 service, so it can be found.
        let available = enzui_shell::input_method::available();
        let on = available && enzui_shell::input_method::wanted(self.settings.input_method);
        let input_method = self.dependent(available, || {
            let note = if available {
                tr("shell-chinese-japanese-korean-and-other-scripts-with-fcitx5-ctrl-space-switches-be")
            } else {
                tr("shell-typing-chinese-japanese-korean-and-other-scripts-needs-fcitx5-which-is-not-i")
            };
            self.switch_setting_noted("keyboard/input_method", on, note, |on| Parent::Change(Change::InputMethod(on)))
        });
        let layouts = column![
            self.layouts_view(),
            // Built inside `dependent` so it is inert with a single layout.
            self.dependent(several, || self.compositor_row(
                enzui_shell::desktop_preferences::OptionKey::LayoutSwitch
            )),
            input_method
        ]
        .spacing(2);
        let body = body
            .push(
                self.section_card_for("keyboard/layouts", layouts)
                    .title(tr("shell-layouts"))
                    .description(tr("shell-the-default-layout-is-used-when-you-start-typing-the-shortcut-moves-to-the-n"))
                    .into_element(),
            )
            .push(
                self.section_card_for("windows/repeat_delay", typing)
                    .title(tr("shell-typing"))
                    .description(tr("shell-keys-repeat-when-held-and-what-caps-lock-does"))
                    .into_element(),
            )
            .push(self.shortcuts_view());
        match self.desktop_status_note() {
            Some(note) => body.push(note),
            None => body,
        }
    }
}

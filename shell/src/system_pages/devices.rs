use super::forms::Form;
use super::*;
use enzui_shell::i18n::tr;
use enzui_shell::system_settings::{Bluetooth, PrinterState, Printers};
use enzui_shell::tr;
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button};
use iced::widget::column;

fn form(message: Form) -> Parent {
    Parent::System(Message::Form(message))
}

impl SettingsApp {
    fn nearby_devices<'a>(&'a self, bluetooth: &Bluetooth, powered: bool) -> Element<'a, Parent> {
        let searching = self.busy_with(|r| matches!(r, Request::BluetoothSearch));
        let writes = self.system_writes();
        let mut rows = column![
            crate::settings_app::settings_row(tr("shell-add-a-device"))
                .subtitle(if searching {
                    tr("shell-searching-put-the-device-in-pairing-mode")
                } else if powered {
                    tr("shell-put-the-device-in-pairing-mode-then-search")
                } else {
                    tr("shell-turn-bluetooth-on-to-add-devices")
                })
                .icon("plus")
                .control(
                    enz_button(
                        if searching {
                            enzui_shell::i18n::tr("shell-searching")
                        } else {
                            enzui_shell::i18n::tr("shell-search")
                        },
                        Self::request(Request::BluetoothSearch)
                    )
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .icon("search")
                    .id("bluetooth-search".into())
                    .enabled(writes && powered && !searching)
                    .into_element(),
                )
                .into_element()
        ]
        .spacing(2);
        if let Some(prompt) = self
            .system
            .forms
            .pair_prompts
            .iter()
            .find(|p| p.needs_answer())
            .or_else(|| self.system.forms.pair_prompts.last())
        {
            rows = rows.push(self.pairing_prompt(prompt));
        }
        for (index, device) in bluetooth.nearby.iter().enumerate() {
            let pairing =
                self.busy_with(|r| matches!(r, Request::BluetoothPair(a) if *a == device.address));
            rows = rows.push(
                crate::settings_app::settings_row(device.name.clone())
                    .subtitle(if pairing {
                        tr("shell-pairing")
                    } else {
                        tr("shell-available")
                    })
                    .icon(device.kind.icon())
                    .control(
                        enz_button(
                            if pairing {
                                tr("shell-pairing")
                            } else {
                                tr("shell-pair")
                            },
                            Self::request(Request::BluetoothPair(device.address.clone())),
                        )
                        .size(ButtonSize::Sm)
                        .id(format!("bluetooth-pair-{index}").into())
                        .enabled(writes && powered && !pairing && !searching)
                        .into_element(),
                    )
                    .into_element(),
            );
        }
        rows.into()
    }
    pub(super) fn bluetooth_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let bluetooth = match self.reading(Panel::Bluetooth) {
            Some(Reading::Bluetooth(b)) => Some(b),
            _ => None,
        };
        let power: Element<'a, Parent> = match &bluetooth {
            None => self.loading_row(),
            Some(Bluetooth { adapter: None, .. }) => self.info_row(
                "bluetooth-off",
                tr("shell-no-bluetooth-adapter"),
                tr("shell-bluetooth-settings-appear-when-this-computer-has-an-adapter"),
            ),
            Some(Bluetooth {
                adapter: Some(adapter),
                ..
            }) => crate::settings_app::settings_row(tr("shell-bluetooth"))
                .subtitle(if adapter.powered {
                    tr!("shell-visible-to-paired-devices-as", "arg0" => adapter.name.clone())
                } else {
                    tr("shell-off").to_owned()
                })
                .icon("bluetooth")
                .id_prefix("control-bluetooth/power")
                .switch(adapter.powered, |on| Self::request(Request::Bluetooth(on)))
                .enabled(self.system_writes())
                .into_element(),
        };
        let mut devices = column![].spacing(2);
        let powered = bluetooth
            .as_ref()
            .and_then(|b| b.adapter.as_ref())
            .is_some_and(|a| a.powered);
        match &bluetooth {
            None => {
                devices = devices.push(self.mark_setting("bluetooth/devices", self.loading_row()));
            }
            Some(b) if b.devices.is_empty() => {
                devices = devices.push(self.mark_setting(
                    "bluetooth/devices",
                    self.info_row(
                        "bluetooth",
                        tr("shell-no-paired-devices"),
                        tr("shell-devices-you-pair-with-this-computer-are-listed-here"),
                    ),
                ));
            }
            Some(b) => {
                for (index, device) in b.devices.iter().enumerate() {
                    let verb = if device.connected {
                        tr("shell-disconnect")
                    } else {
                        tr("shell-connect")
                    };
                    let row = crate::settings_app::settings_row(device.name.clone())
                        .subtitle(if device.connected {
                            tr("shell-connected")
                        } else {
                            tr("shell-not-connected")
                        })
                        .icon(device.kind.icon())
                        .control(
                            iced::widget::row![
                                enz_button(
                                    verb,
                                    Self::request(Request::BluetoothDevice(
                                        device.address.clone(),
                                        !device.connected
                                    ))
                                )
                                .kind(ButtonKind::Secondary)
                                .size(ButtonSize::Sm)
                                .id(format!("bluetooth-device-{index}").into())
                                .enabled(self.system_writes() && powered)
                                .into_element(),
                                enzui_widgets::toolbar_icon::enz_toolbar_icon(
                                    "x",
                                    Self::request(Request::BluetoothRemove(device.address.clone()))
                                )
                                .aria_label(enzui_shell::tr!("shell-forget-2", "arg0" => enzui_shell::i18n::argument(&(device.name))))
                                .tooltip(tr("shell-forget-device"))
                                .id(format!("bluetooth-remove-{index}").into())
                                .enabled(self.system_writes())
                                .into_element(),
                            ]
                            .spacing(6)
                            .align_y(iced::Alignment::Center),
                        )
                        .into_element();
                    devices = devices.push(if index == 0 {
                        self.mark_setting("bluetooth/devices", row)
                    } else {
                        row
                    });
                }
            }
        }
        // The switch needs no heading of its own: the page already names it.
        let body = body
            .push(
                self.section_card_for(
                    "bluetooth/power",
                    self.mark_setting("bluetooth/power", power),
                )
                .into_element(),
            )
            .push(
                self.section_card_for("bluetooth/devices", devices)
                    .title(tr("shell-paired-devices"))
                    .description(tr(
                        "shell-paired-devices-reconnect-when-they-are-nearby-and-switched-on",
                    ))
                    .into_element(),
            );
        match bluetooth.as_ref().filter(|b| b.adapter.is_some()) {
            Some(bluetooth) => body.push(
                self.section_card(self.nearby_devices(bluetooth, powered))
                    .title(tr("shell-nearby-devices"))
                    .into_element(),
            ),
            None => body,
        }
    }
    pub(super) fn printers_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let route = "printers/list";
        let printers = match self.reading(Panel::Printers) {
            Some(Reading::Printers(p)) => Some(p),
            _ => None,
        };
        let mut rows = column![].spacing(2);
        match &printers {
            None => rows = rows.push(self.mark_setting(route, self.loading_row())),
            Some(Printers {
                scheduler: None, ..
            }) => {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "printer",
                        tr("shell-printing-is-not-installed"),
                        tr("shell-install-cups-to-print-and-add-printers"),
                    ),
                ));
            }
            Some(Printers {
                scheduler: Some(false),
                ..
            }) => {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "printer",
                        tr("shell-the-printing-service-is-not-running"),
                        tr("shell-start-cups-cups-service-to-see-and-add-printers"),
                    ),
                ));
            }
            Some(p) if p.printers.is_empty() => {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "printer",
                        tr("shell-no-printers"),
                        tr("shell-search-to-add-a-printer-on-this-network"),
                    ),
                ));
            }
            Some(p) => {
                for (index, printer) in p.printers.iter().enumerate() {
                    let waiting = p
                        .jobs
                        .iter()
                        .filter(|job| job.queue == printer.name)
                        .count();
                    let state = match printer.state {
                        PrinterState::Idle => tr("shell-ready").to_owned(),
                        PrinterState::Printing => tr("shell-printing").to_owned(),
                        PrinterState::Paused if waiting > 0 => {
                            tr!("shell-paused-of-your-jobs-waiting", "arg0" => format!("{:}", waiting))
                        }
                        PrinterState::Paused => tr("shell-paused").to_owned(),
                        PrinterState::Offline => {
                            tr("shell-not-responding-check-that-it-is-on-and-on-this-network")
                                .to_owned()
                        }
                        PrinterState::Stopped => {
                            tr("shell-stopped-by-an-error-on-the-printer").to_owned()
                        }
                    };
                    let paused = printer.state == PrinterState::Paused;
                    let main: Element<'a, Parent> = if printer.default {
                        self.status_badge(tr("shell-default"), BadgeTone::Accent)
                    } else {
                        enz_button(
                            tr("shell-make-default"),
                            Self::request(Request::DefaultPrinter(printer.name.clone())),
                        )
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .id(format!("printer-default-{index}").into())
                        .enabled(self.system_writes())
                        .into_element()
                    };
                    let control = iced::widget::row![
                        main,
                        enzui_widgets::toolbar_icon::enz_toolbar_icon(
                            "file-text",
                            Self::request(Request::PrinterTest(printer.name.clone()))
                        )
                        .aria_label(enzui_shell::tr!("shell-a11y-print-a-test-page-on", "arg0" => enzui_shell::i18n::argument(&(printer.name))))
                        .tooltip(tr("shell-print-test-page"))
                        .id(format!("printer-test-{index}").into())
                        .enabled(self.system_writes())
                        .into_element(),
                        enzui_widgets::toolbar_icon::enz_toolbar_icon(
                            if paused { "play" } else { "pause" },
                            Self::request(Request::PrinterPause(printer.name.clone(), !paused))
                        )
                        .aria_label(format!(
                            "{} {}",
                            if paused { enzui_shell::i18n::tr("shell-resume") } else { enzui_shell::i18n::tr("shell-a11y-pause") },
                            printer.name
                        ))
                        .tooltip(if paused {
                            tr("shell-resume-printing")
                        } else {
                            tr("shell-pause-printing")
                        })
                        .id(format!("printer-pause-{index}").into())
                        .enabled(self.system_writes() && p.admin)
                        .into_element(),
                        enzui_widgets::toolbar_icon::enz_toolbar_icon(
                            "trash-2",
                            Self::request(Request::PrinterRemove(printer.name.clone()))
                        )
                        .aria_label(enzui_shell::tr!("shell-remove-3", "arg0" => enzui_shell::i18n::argument(&(printer.name))))
                        .tooltip(tr("shell-remove-printer"))
                        .id(format!("printer-remove-{index}").into())
                        .enabled(self.system_writes() && p.admin)
                        .into_element(),
                    ]
                    .spacing(6)
                    .align_y(iced::Alignment::Center);
                    let mut tokens = crate::settings_app::settings_row_tokens();
                    let button = enzui_widgets::button::ButtonSize::Sm.tokens();
                    let label_width = enzui_widgets::text_measure::enz_text_measure_width(
                        tr("shell-make-default"),
                        enzui_widgets::text_measure::MeasureTypography::new(TextSize::Body)
                            .font_family(
                                self.tokens()
                                    .font_catalog
                                    .family(enzui_theme::FontRole::Ui)
                                    .to_iced(),
                            )
                            .weight(enzui_theme::text_defaults::weight(
                                enzui_theme::WeightRung::Semibold,
                            ))
                            .pin_size_px(button.text_size),
                    );
                    tokens.inline_field_width = tokens.inline_field_width.max(
                        label_width
                            + button.padding_x * 2.0
                            + 3.0 * enzui_theme::text_scale::ui_px(28.0)
                            + 3.0 * 6.0
                            + 4.0,
                    );
                    let row = crate::settings_app::settings_row(printer.name.replace('_', " "))
                        .tokens(tokens)
                        .subtitle(state)
                        .icon("printer")
                        .control(control)
                        .into_element();
                    rows = rows.push(if index == 0 {
                        self.mark_setting(route, row)
                    } else {
                        row
                    });
                }
            }
        }
        if let Some(p) = printers.as_ref().filter(|p| p.scheduler == Some(true)) {
            if p.admin {
                rows = rows.push(self.add_printer());
            } else {
                rows = rows.push(self.info_row("shield", tr("shell-adding-and-removing-printers-is-not-available"), tr("shell-install-cups-pk-helper-to-manage-printers-with-administrator-approval")));
            }
        }
        let body = body.push(
            self.section_card_for(route, rows)
                .title(tr("shell-your-printers"))
                .description(tr("shell-documents-print-to-the-default-printer-unless-an-app-asks-adding-removing-or"))
                .into_element(),
        );
        let mut jobs = column![].spacing(2);
        match printers.as_ref().filter(|p| p.scheduler == Some(true)) {
            None => {
                jobs = jobs.push(self.mark_setting(
                    "printers/jobs",
                    self.info_row(
                        "file-text",
                        tr("shell-no-print-jobs"),
                        tr("shell-your-documents-appear-here-while-they-wait-to-print"),
                    ),
                ));
            }
            Some(p) => {
                if p.jobs.is_empty() {
                    jobs = jobs.push(self.mark_setting(
                        "printers/jobs",
                        self.info_row(
                            "file-text",
                            tr("shell-nothing-waiting-to-print"),
                            tr("shell-your-documents-appear-here-until-they-have-printed"),
                        ),
                    ));
                }
                for (index, job) in p.jobs.iter().enumerate() {
                    let detail = format!(
                        "{}{}{}",
                        job.queue.replace('_', " "),
                        if job.size.is_empty() { "" } else { " · " },
                        job.size
                    );
                    let row = crate::settings_app::settings_row(tr!("shell-job", "arg0" => job.id.rsplit('-').next().unwrap_or(&job.id).to_owned()))
                        .subtitle(if job.held { tr!("shell-held", "arg0" => detail.clone()) } else { detail })
                        .icon("file-text")
                        .control(
                            enz_button(tr("shell-cancel"), Self::request(Request::JobCancel(job.id.clone())))
                                .kind(ButtonKind::Secondary)
                                .size(ButtonSize::Sm)
                                .id(format!("printer-job-cancel-{index}").into())
                                .enabled(self.system_writes())
                                .into_element(),
                        )
                        .into_element();
                    jobs = jobs.push(if index == 0 {
                        self.mark_setting("printers/jobs", row)
                    } else {
                        row
                    });
                }
            }
        }
        body.push(
            self.section_card_for("printers/jobs", jobs)
                .title(tr("shell-print-jobs"))
                .description(tr("shell-only-your-own-documents-are-shown"))
                .into_element(),
        )
    }
    fn pairing_prompt<'a>(
        &'a self,
        prompt: &enzui_shell::system_settings::pairing::Prompt,
    ) -> Element<'a, Parent> {
        enzui_shell::system_settings::pairing::prompt_view(
            prompt,
            &self.system.forms.pin,
            self.tokens().palette,
            |text| Parent::System(Message::Form(Form::Pin(text))),
            |answer| Parent::System(Message::Pairing(super::Pairing::Answer(answer))),
            |text| self.caption(text),
        )
    }
    fn add_printer<'a>(&self) -> Element<'a, Parent> {
        let forms = &self.system.forms;
        let writes = self.system_writes();
        let mut rows = column![
            crate::settings_app::settings_row(tr("shell-add-a-printer"))
                .subtitle(if forms.printer_search {
                    tr("shell-looking-for-printers-on-this-network")
                } else {
                    tr("shell-finds-printers-on-this-network-and-usb")
                })
                .icon("plus")
                .control(
                    enz_button(
                        if forms.printer_search {
                            enzui_shell::i18n::tr("shell-searching")
                        } else {
                            enzui_shell::i18n::tr("shell-search")
                        },
                        form(Form::SearchPrinters)
                    )
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .icon("search")
                    .id("printer-search".into())
                    .enabled(writes && !forms.printer_search)
                    .into_element(),
                )
                .into_element()
        ]
        .spacing(2);
        match &forms.printers_nearby {
            Some(found) if found.is_empty() && !forms.printer_search => {
                rows = rows.push(self.caption_row(tr(
                    "shell-no-new-printers-found-check-that-the-printer-is-on-and-on-this-network",
                )));
                // On a public network the firewall drops the printers'
                // answers: offer to trust the network this computer is on.
                let public = match self.reading(Panel::Network) {
                    Some(Reading::Network(network)) => network
                        .interfaces
                        .into_iter()
                        .find(|i| {
                            i.state == enzui_shell::system_settings::LinkState::Connected
                                && i.profile.as_ref().is_some_and(|p| !p.trusted)
                        })
                        .map(|i| i.connection),
                    _ => None,
                };
                if let Some(connection) = public {
                    rows = rows.push(
                        crate::settings_app::settings_row(tr!(
                            "shell-trust-network-to-find-printers",
                            "arg0" => connection.clone()
                        ))
                        .subtitle(tr("shell-printers-cannot-answer-on-a-public-network"))
                        .subtitle_wrap(true)
                        .icon("shield")
                        .control(
                            enz_button(
                                tr("shell-trust-and-search"),
                                Self::request(Request::Trust {
                                    connection,
                                    trusted: true,
                                }),
                            )
                            .kind(ButtonKind::Secondary)
                            .size(ButtonSize::Sm)
                            .id("printer-trust-network".into())
                            .enabled(writes)
                            .into_element(),
                        )
                        .into_element(),
                    );
                }
                rows = rows.push(
                    enz_button(
                        tr("shell-printer-network-settings"),
                        Parent::Page(Category::Network),
                    )
                    .kind(ButtonKind::Plain)
                    .size(ButtonSize::Sm)
                    .id("printer-network-settings".into())
                    .into_element(),
                );
            }
            Some(found) => {
                for (index, printer) in found.iter().enumerate() {
                    let adding =
                        self.busy_with(|r| matches!(r, Request::PrinterAdd(p) if p == printer));
                    let place = if printer.uri.starts_with("usb://") {
                        tr("shell-printer-usb").to_owned()
                    } else {
                        printer
                            .uri
                            .split("://")
                            .nth(1)
                            .and_then(|r| r.split(['/', ':']).next())
                            .unwrap_or(&printer.uri)
                            .trim_end_matches(".local")
                            .to_owned()
                    };
                    let subtitle = match (&printer.driver, printer.unsupported) {
                        (_, true) => format!("{place} · {}", tr("shell-printer-no-driver")),
                        (Some((_, model)), _) => format!(
                            "{place} · {}",
                            tr!("shell-printer-driver", "arg0" => model.clone())
                        ),
                        (None, false) => place,
                    };
                    rows = rows.push(
                        crate::settings_app::settings_row(printer.name.clone())
                            .subtitle(subtitle)
                            .subtitle_wrap(true)
                            .icon("printer")
                            .control(
                                enz_button(
                                    if adding {
                                        tr("shell-adding")
                                    } else {
                                        tr("shell-add-2")
                                    },
                                    Self::request(Request::PrinterAdd(printer.clone())),
                                )
                                .size(ButtonSize::Sm)
                                .id(format!("printer-add-{index}").into())
                                .enabled(writes && !adding && !printer.unsupported)
                                .into_element(),
                            )
                            .into_element(),
                    );
                }
            }
            None => {}
        }
        rows.into()
    }
}

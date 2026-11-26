use super::forms::Form;
use super::*;
use enzui_shell::i18n::tr;
use enzui_shell::system_settings::{
    AccessPoint, Details, Interface, InterfaceKind, LinkState, Network,
};
use enzui_shell::tr;
use enzui_widgets::button::{ButtonKind, ButtonSize, enz_button};
use enzui_widgets::icon::enz_icon;
use enzui_widgets::text::enz_text;
use enzui_widgets::text_field::enz_text_field;
use enzui_widgets::toolbar_icon::enz_toolbar_icon;
use iced::{
    Alignment, Length,
    widget::{column, container, row},
};

fn signal_icon(signal: u8) -> &'static str {
    match signal {
        67.. => "wifi",
        34..=66 => "wifi-high",
        1..=33 => "wifi-low",
        0 => "wifi-zero",
    }
}

fn form(message: Form) -> Parent {
    Parent::System(Message::Form(message))
}

impl SettingsApp {
    fn details_rows<'a>(&self, details: &Details) -> Element<'a, Parent> {
        let mut rows = column![].spacing(0);
        let fields = [
            (tr("shell-ipv4-address"), details.ipv4.clone()),
            (tr("shell-ipv6-address"), details.ipv6.clone()),
            (tr("shell-gateway"), details.gateway.clone()),
            (
                (if details.dns.len() > 1 {
                    tr("shell-dns-servers")
                } else {
                    tr("shell-dns-server")
                }),
                (!details.dns.is_empty()).then(|| details.dns.join(", ")),
            ),
            (tr("shell-hardware-address"), details.hardware.clone()),
        ];
        for (title, value) in fields {
            if let Some(value) = value {
                rows = rows.push(self.detail_row(title, value));
            }
        }
        container(rows)
            .padding(iced::Padding::default().left(34.0).bottom(6.0))
            .into()
    }
    fn connection_panel<'a>(
        &'a self,
        interface: &Interface,
        forgettable: bool,
    ) -> Element<'a, Parent> {
        let connection = interface.connection.clone();
        let profile = interface.profile.clone().unwrap_or_default();
        let writes = self.system_writes();
        let trust = {
            let name = connection.clone();
            let choice = |trusted: bool| {
                Self::request(Request::Trust {
                    connection: name.clone(),
                    trusted,
                })
            };
            crate::settings_app::settings_row(tr("shell-network-type"))
                .subtitle(if profile.trusted {
                    tr("shell-trusted-devices-here-can-find-and-share-with-this-computer")
                } else {
                    tr("shell-public-this-computer-stays-hidden-from-other-devices")
                })
                .id_prefix(format!("control-network-trust-{connection}"))
                .value_menu(
                    if profile.trusted {
                        tr("shell-trusted")
                    } else {
                        tr("shell-public")
                    },
                    self.menu.as_deref() == Some("network/trust"),
                    Parent::Menu("network/trust".into(), true),
                    Parent::Menu("network/trust".into(), false),
                    vec![
                        (tr("shell-trusted-home-or-work").into(), choice(true)),
                        (enzui_shell::i18n::tr("shell-public").into(), choice(false)),
                    ],
                )
                .enabled(writes)
                .into_element()
        };
        let dns: Element<'a, Parent> = match &self.system.forms.dns {
            Some((target, text)) if *target == connection => container(
                column![
                    enz_text_field(enzui_shell::i18n::current().text("shell-dns-servers"), "1.1.1.1, 9.9.9.9", text, |t| form(Form::DnsEdit(t)))
                        .description(enzui_shell::i18n::current().text("shell-separate-addresses-with-commas-leave-empty-to-use-the-network-s-own-servers"))
                        .on_submit(form(Form::DnsSave))
                        .id("network-dns".into())
                        .into_element(),
                    row![
                        iced::widget::Space::new().width(Length::Fill),
                        enz_button(tr("shell-cancel"), form(Form::Dns(None))).id("network-dns-cancel".into()).kind(ButtonKind::Plain).into_element(),
                        enz_button(enzui_shell::i18n::tr("shell-save"), form(Form::DnsSave)).id("network-dns-save".into()).enabled(writes).into_element(),
                    ]
                    .spacing(8),
                ]
                .spacing(8),
            )
            .padding([8, 12])
            .into(),
            _ => {
                let current = if profile.dns.is_empty() { tr("shell-automatic").to_owned() } else { profile.dns.join(", ") };
                crate::settings_app::settings_row(tr("shell-dns-servers"))
                    .subtitle(current)
                    .control(
                        enz_button(tr("shell-edit"), form(Form::Dns(Some((connection.clone(), profile.dns.join(", "))))))
                            .kind(ButtonKind::Secondary)
                            .size(ButtonSize::Sm)
                            .id(format!("network-dns-edit-{connection}").into())
                            .enabled(writes)
                            .into_element(),
                    )
                    .into_element()
            }
        };
        let ipv4 = self.ipv4_editor(&connection, &profile, writes);
        let mut actions = row![iced::widget::Space::new().width(Length::Fill)].spacing(8);
        if interface.kind == InterfaceKind::Wifi {
            actions = actions.push(
                enz_button(
                    tr("shell-disconnect"),
                    Self::request(Request::WifiDisconnect),
                )
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .id("network-disconnect".into())
                .enabled(writes)
                .into_element(),
            );
        }
        if forgettable {
            actions = actions.push(
                enz_button(
                    tr("shell-forget-network"),
                    Self::request(Request::Forget(connection.clone())),
                )
                .kind(ButtonKind::Destructive)
                .size(ButtonSize::Sm)
                .id("network-forget".into())
                .enabled(writes)
                .into_element(),
            );
        }
        let mut panel = column![].spacing(2);
        // Trust is a firewalld zone; without firewalld it would change nothing.
        if self.offers(enzui_shell::platform::Feature::NetworkTrust) {
            panel = panel.push(trust);
        }
        let mut panel = panel.push(dns).push(ipv4);
        if let Some(details) = interface.details.as_ref() {
            panel = panel.push(self.details_rows(details));
        }
        container(panel.push(container(actions).padding([4, 12])))
            .padding(iced::Padding::default().left(22.0).bottom(8.0))
            .into()
    }
    fn ipv4_editor<'a>(
        &'a self,
        connection: &str,
        profile: &enzui_shell::system_settings::Profile,
        writes: bool,
    ) -> Element<'a, Parent> {
        let Some(edit) = self
            .system
            .forms
            .ipv4
            .as_ref()
            .filter(|e| e.connection == connection)
        else {
            let current = match &profile.ipv4 {
                None => enzui_shell::i18n::tr("shell-automatic-dhcp").to_owned(),
                Some(m) if m.gateway.is_empty() => {
                    tr!("shell-manual", "arg0" => m.address.clone())
                }
                Some(m) => {
                    tr!("shell-manual-via", "arg0" => m.address.clone(), "arg1" => m.gateway.clone())
                }
            };
            return crate::settings_app::settings_row(tr("shell-ip-address"))
                .subtitle(current)
                .control(
                    enz_button(
                        tr("shell-edit"),
                        form(Form::Ipv4(Some(connection.to_owned()))),
                    )
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .id(format!("network-ipv4-edit-{connection}").into())
                    .enabled(writes)
                    .into_element(),
                )
                .into_element();
        };
        let saving = self.busy_with(|r| matches!(r, Request::Ipv4 { .. }));
        let mut fields = column![
            crate::settings_app::settings_row(tr("shell-set-the-address-manually"))
                .subtitle(if edit.manual {
                    tr("shell-enter-the-address-this-network-assigned-to-you")
                } else {
                    tr("shell-the-network-assigns-an-address-dhcp")
                })
                .id_prefix("control-network-ipv4-manual")
                .switch(edit.manual, |on| form(Form::Ipv4Manual(on)))
                .enabled(!saving)
                .into_element(),
        ]
        .spacing(8);
        if edit.manual {
            let gateway_error = edit.error.as_ref().filter(|e| {
                e.contains("gateway")
                    || e.contains(&format!("“{}”", edit.gateway.trim()))
                        && !edit.gateway.trim().is_empty()
            });
            let mut address = enz_text_field(
                enzui_shell::i18n::current().text("shell-address"),
                "192.168.1.20/24",
                &edit.address,
                |t| form(Form::Ipv4Address(t)),
            )
            .description(
                enzui_shell::i18n::current()
                    .text("shell-the-address-and-its-prefix-length-after-the-slash"),
            )
            .on_submit(form(Form::Ipv4Save))
            .id("network-ipv4-address".into())
            .enabled(!saving);
            let mut gateway = enz_text_field(
                enzui_shell::i18n::current().text("shell-gateway"),
                "192.168.1.1",
                &edit.gateway,
                |t| form(Form::Ipv4Gateway(t)),
            )
            .description(
                enzui_shell::i18n::current()
                    .text("shell-the-router-s-address-leave-empty-on-a-network-without-one"),
            )
            .on_submit(form(Form::Ipv4Save))
            .id("network-ipv4-gateway".into())
            .enabled(!saving);
            match (&edit.error, gateway_error) {
                (_, Some(error)) => gateway = gateway.error(error.clone()),
                (Some(error), None) => address = address.error(error.clone()),
                (None, None) => {}
            }
            fields = fields
                .push(address.into_element())
                .push(gateway.into_element());
        } else if let Some(error) = &edit.error {
            fields = fields.push(self.caption(error.clone()));
        }
        fields = fields.push(
            row![
                iced::widget::Space::new().width(Length::Fill),
                enz_button(tr("shell-cancel"), form(Form::Ipv4(None)))
                    .id("network-ipv4-cancel".into())
                    .kind(ButtonKind::Plain)
                    .enabled(!saving)
                    .into_element(),
                enz_button(
                    if saving {
                        enzui_shell::i18n::tr("shell-applying")
                    } else {
                        enzui_shell::i18n::tr("shell-apply")
                    },
                    form(Form::Ipv4Save)
                )
                .id("network-ipv4-save".into())
                .enabled(writes && !saving)
                .into_element(),
            ]
            .spacing(8),
        );
        container(fields).padding([8, 12]).into()
    }
    fn trust_prompt<'a>(&self, ssid: &str) -> Element<'a, Parent> {
        let palette = self.tokens().palette;
        container(
            column![
                row![
                    enz_icon("shield-check").size(20.0).into_element(),
                    column![
                        enz_text(enzui_shell::tr!("shell-is-a-trusted-network", "arg0" => enzui_shell::i18n::argument(&(ssid)))).weight_rung(enzui_theme::WeightRung::Semibold).into_element(),
                        self.caption(tr("shell-choose-trusted-for-home-or-work-nearby-devices-can-find-this-computer-to-sha")),
                    ]
                    .spacing(4)
                    .width(Length::Fill),
                ]
                .spacing(12),
                row![
                    iced::widget::Space::new().width(Length::Fill),
                    enz_button(tr("shell-public-network"), form(Form::Trust(false))).kind(ButtonKind::Secondary).id("network-trust-public".into()).into_element(),
                    enz_button(tr("shell-trusted-network"), form(Form::Trust(true))).id("network-trust-home".into()).into_element(),
                ]
                .spacing(8),
            ]
            .spacing(12),
        )
        .padding([14, 16])
        .style(move |_| container::Style {
            background: Some(palette.accent_soft.into()),
            border: iced::Border { radius: 12.0.into(), width: 1.0, color: palette.accent },
            ..Default::default()
        })
        .into()
    }
    fn join_form<'a>(&'a self) -> Element<'a, Parent> {
        let Some(join) = &self.system.forms.join else {
            return iced::widget::Space::new().into();
        };
        let joining = self.busy_with(|r| matches!(r, Request::WifiJoin { .. }));
        let mut fields = column![].spacing(8);
        if join.hidden {
            fields = fields.push(
                enz_text_field(
                    enzui_shell::i18n::current().text("shell-network-name"),
                    enzui_shell::i18n::current().text("shell-exact-name-including-capitals"),
                    &join.ssid,
                    |t| form(Form::JoinSsid(t)),
                )
                .id("network-join-ssid".into())
                .enabled(!joining)
                .into_element(),
            );
        }
        if join.enterprise {
            fields = fields.push(
                enz_text_field(
                    enzui_shell::i18n::current().text("shell-username"),
                    enzui_shell::i18n::current().text("shell-your-account-at-this-organisation"),
                    &join.identity,
                    |t| form(Form::JoinIdentity(t)),
                )
                .id("network-join-identity".into())
                .enabled(!joining)
                .into_element(),
            );
        }
        let placeholder = match (join.enterprise, join.hidden) {
            (true, _) => enzui_shell::i18n::current().text("shell-your-account-password"),
            (false, true) => {
                enzui_shell::i18n::current().text("shell-leave-empty-for-an-open-network")
            }
            (false, false) => enzui_shell::i18n::current().text("shell-network-password"),
        };
        let mut password = enz_text_field(
            enzui_shell::i18n::current().text("shell-password"),
            placeholder,
            &join.password,
            |t| form(Form::JoinPassword(t)),
        )
        .secure(!join.revealed)
        .trailing(
            enz_toolbar_icon(
                if join.revealed { "eye-off" } else { "eye" },
                form(Form::JoinReveal),
            )
            .id("network-join-reveal".into())
            .aria_label(if join.revealed {
                enzui_shell::i18n::current().text("shell-hide-password")
            } else {
                enzui_shell::i18n::current().text("shell-show-password")
            })
            .into_element(),
        )
        .on_submit(form(Form::JoinSubmit))
        .id("network-join-password".into())
        .enabled(!joining);
        if let Some(error) = &join.error {
            password = password.error(error.clone());
        }
        let ready = (1..=32).contains(&join.ssid.len())
            && !joining
            && if join.enterprise {
                !join.identity.trim().is_empty() && !join.password.is_empty() && join.server_named()
            } else {
                join.hidden || join.password.len() >= 8
            };
        fields = fields.push(password.into_element());
        if join.enterprise {
            fields = fields.push(self.join_server(join, joining));
        }
        fields = fields.push(
            row![
                iced::widget::Space::new().width(Length::Fill),
                enz_button(tr("shell-cancel"), form(Form::Join(None)))
                    .id("network-join-cancel".into())
                    .kind(ButtonKind::Plain)
                    .enabled(!joining)
                    .into_element(),
                enz_button(
                    if joining {
                        enzui_shell::i18n::tr("shell-connecting")
                    } else {
                        enzui_shell::i18n::tr("shell-connect")
                    },
                    form(Form::JoinSubmit)
                )
                .id("network-join-submit".into())
                .enabled(ready && self.system_writes())
                .into_element(),
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        );
        container(fields)
            .padding(iced::Padding::from([8, 12]).left(56.0))
            .into()
    }
    // The fields that let an enterprise join recognise the real sign-in server.
    fn join_server<'a>(&self, join: &'a super::forms::Join, joining: bool) -> Element<'a, Parent> {
        let checked = !join.unchecked && !joining;
        let mut fields = column![
            enz_text_field(
                tr("shell-sign-in-server-domain"),
                tr("shell-sign-in-server-domain-example"),
                &join.domain,
                |t| form(Form::JoinDomain(t)),
            )
            .description(tr("shell-sign-in-server-domain-detail"))
            .on_submit(form(Form::JoinSubmit))
            .id("network-join-domain".into())
            .enabled(checked)
            .into_element(),
            enz_text_field(
                tr("shell-certificate-authority-file"),
                tr("shell-certificate-authority-file-example"),
                &join.authority,
                |t| form(Form::JoinAuthority(t)),
            )
            .description(tr("shell-certificate-authority-file-detail"))
            .on_submit(form(Form::JoinSubmit))
            .id("network-join-authority".into())
            .enabled(checked)
            .into_element(),
            enzui_widgets::checkbox::enz_checkbox(
                join.unchecked,
                tr("shell-do-not-check-the-sign-in-server"),
                |on| form(Form::JoinUnchecked(on)),
            )
            .into_element(),
        ]
        .spacing(8);
        if join.unchecked {
            fields = fields.push(
                enz_banner(tr("shell-unchecked-sign-in-server-warning"))
                    .tone(BannerTone::Warning)
                    .icon("triangle-alert")
                    .id("network-join-unchecked")
                    .into_element(),
            );
        }
        fields.into()
    }
    fn network_row<'a>(&self, point: &AccessPoint) -> Element<'a, Parent> {
        let subtitle = match (point.active, point.saved, point.secure) {
            (_, false, true) | (true, _, true) if point.enterprise => {
                tr("shell-enterprise-sign-in-with-your-account")
            }
            (true, _, true) => tr("shell-secured"),
            (true, _, false) => tr("shell-open-network"),
            (false, true, _) => tr("shell-saved"),
            (false, false, true) => tr("shell-secured"),
            (false, false, false) => tr("shell-open-network"),
        };
        let joining =
            self.busy_with(|r| matches!(r, Request::WifiJoin { ssid, .. } if *ssid == point.ssid));
        let trailing: Element<'a, Parent> = if point.active {
            let open = self.system.forms.details.as_deref() == Some(point.ssid.as_str());
            row![
                self.status_badge(enzui_shell::i18n::tr("shell-connected"), BadgeTone::Success),
                enz_toolbar_icon(
                    if open { "chevron-up" } else { "chevron-down" },
                    form(Form::Details(Some(point.ssid.clone())))
                )
                .aria_label(if open {
                    tr("shell-hide-network-details")
                } else {
                    tr("shell-show-network-details")
                })
                .tooltip(enzui_shell::i18n::tr("shell-details"))
                .id(format!("wifi-details-{}", point.ssid).into())
                .into_element(),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .into()
        } else {
            // The subtitle already says whether a network is secured.
            let mut trailing = row![].spacing(6).align_y(Alignment::Center);
            let message = if point.saved || !point.secure {
                Self::request(Request::WifiJoin {
                    ssid: point.ssid.clone(),
                    password: None,
                    hidden: false,
                    enterprise: None,
                })
            } else {
                form(Form::Join(Some((point.ssid.clone(), false))))
            };
            trailing = trailing.push(
                enz_button(
                    if joining {
                        tr("shell-connecting")
                    } else {
                        tr("shell-connect")
                    },
                    message,
                )
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .id(format!("wifi-connect-{}", point.ssid).into())
                .enabled(self.system_writes() && !joining)
                .into_element(),
            );
            if point.saved {
                trailing = trailing.push(
                    enz_toolbar_icon("x", Self::request(Request::Forget(point.ssid.clone())))
                        .id(format!("wifi-forget-{}", point.ssid).into())
                        .aria_label(tr!("shell-forget-2", "arg0" => point.ssid.clone()))
                        .tooltip(tr("shell-forget"))
                        .enabled(self.system_writes())
                        .into_element(),
                );
            } else {
                // Keeps every Connect button in one column with the saved
                // networks' Forget control.
                trailing = trailing.push(iced::widget::Space::new().width(28.0));
            }
            trailing.into()
        };
        crate::settings_app::settings_row(point.ssid.clone())
            .subtitle(subtitle)
            .icon(signal_icon(point.signal))
            .control(trailing)
            .into_element()
    }
    fn wifi_card<'a>(&'a self, network: Option<&Network>) -> Element<'a, Parent> {
        let route = "network/wifi";
        let mut rows = column![].spacing(2);
        match network {
            None => rows = rows.push(self.mark_setting(route, self.loading_row())),
            Some(network) if network.networking.is_none() => {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "wifi-off",
                        tr("shell-network-settings-are-unavailable"),
                        tr("shell-they-need-networkmanager-which-is-not-running-on-this-computer"),
                    ),
                ));
            }
            Some(network) if !network.has_wifi() => {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "wifi-off",
                        tr("shell-no-wi-fi-adapter"),
                        tr("shell-wireless-networks-appear-here-when-a-wi-fi-adapter-is-connected"),
                    ),
                ));
            }
            Some(network) => {
                let on = network.wifi_radio == Some(true);
                rows = rows.push(
                    self.mark_setting(
                        route,
                        crate::settings_app::settings_row(tr("shell-wi-fi"))
                            .icon(if on { "wifi" } else { "wifi-off" })
                            .id_prefix(format!("control-{route}"))
                            .switch(on, |on| Self::request(Request::WifiRadio(on)))
                            .enabled(self.system_writes() && network.wifi_radio.is_some())
                            .into_element(),
                    ),
                );
                if let Some(ssid) = self.system.forms.trust_prompt.as_ref().filter(|_| on) {
                    rows = rows.push(container(self.trust_prompt(ssid)).padding([6, 4]));
                }
                if on {
                    if network.access_points.is_empty() {
                        rows = rows.push(self.info_row(
                            "search",
                            tr("shell-no-networks-found"),
                            tr("shell-networks-in-range-will-be-listed-here"),
                        ));
                    }
                    for point in &network.access_points {
                        rows = rows.push(self.network_row(point));
                        let joining_here = self
                            .system
                            .forms
                            .join
                            .as_ref()
                            .is_some_and(|j| !j.hidden && j.ssid == point.ssid);
                        if joining_here {
                            rows = rows.push(self.join_form());
                        }
                        if point.active
                            && self.system.forms.details.as_deref() == Some(point.ssid.as_str())
                        {
                            let wifi = network.interfaces.iter().find(|i| {
                                i.kind == InterfaceKind::Wifi && i.state == LinkState::Connected
                            });
                            if let Some(interface) = wifi {
                                rows = rows.push(self.connection_panel(interface, true));
                            }
                        }
                    }
                    if self.system.forms.join.as_ref().is_some_and(|j| j.hidden) {
                        rows = rows.push(self.join_form());
                    } else {
                        rows = rows.push(
                            crate::settings_app::settings_row(tr("shell-join-a-hidden-network"))
                                .icon("plus")
                                .action("", form(Form::Join(Some((String::new(), true)))))
                                .id("network-join-hidden".into())
                                .enabled(self.system_writes())
                                .into_element(),
                        );
                    }
                }
            }
        }
        self.section_card_for(route, rows)
            .title(tr("shell-wi-fi"))
            .description(tr(
                "shell-wireless-networks-in-range-saved-networks-reconnect-automatically",
            ))
            .into_element()
    }
    fn wired_card<'a>(&'a self, network: Option<&Network>) -> Element<'a, Parent> {
        let route = "network/wired";
        let wired: Vec<&Interface> = network.map_or_else(Vec::new, |n| {
            n.interfaces
                .iter()
                .filter(|i| i.kind == InterfaceKind::Ethernet)
                .collect()
        });
        let mut rows = column![].spacing(2);
        if network.is_none() {
            rows = rows.push(self.mark_setting(route, self.loading_row()));
        } else if wired.is_empty() {
            rows = rows.push(self.mark_setting(
                route,
                self.info_row(
                    "cable",
                    tr("shell-no-wired-adapter"),
                    tr("shell-ethernet-connections-appear-here-when-an-adapter-is-present"),
                ),
            ));
        }
        for (index, interface) in wired.iter().enumerate() {
            let title = if wired.len() > 1 {
                tr!("shell-ethernet", "arg0" => format!("{:}", index + 1))
            } else {
                tr("shell-ethernet-2").to_owned()
            };
            let connected = interface.state == LinkState::Connected;
            let tone = if connected {
                BadgeTone::Success
            } else {
                BadgeTone::Default
            };
            let open = connected
                && self.system.forms.details.as_deref() == Some(interface.connection.as_str());
            let mut trailing = row![self.status_badge(interface.state.label(), tone)]
                .spacing(6)
                .align_y(Alignment::Center);
            if connected && !interface.connection.is_empty() {
                trailing = trailing.push(
                    enz_toolbar_icon(
                        if open { "chevron-up" } else { "chevron-down" },
                        form(Form::Details(Some(interface.connection.clone()))),
                    )
                    .aria_label(if open {
                        tr("shell-hide-connection-details")
                    } else {
                        tr("shell-show-connection-details")
                    })
                    .tooltip(tr("shell-details"))
                    .id(format!("wired-details-{index}").into())
                    .into_element(),
                );
            }
            let row = crate::settings_app::settings_row(title)
                .subtitle(interface.device.clone())
                .icon("ethernet-port")
                .control(trailing)
                .into_element();
            rows = rows.push(if index == 0 {
                self.mark_setting(route, row)
            } else {
                row
            });
            if open {
                rows = rows.push(self.connection_panel(interface, false));
            } else if let Some(details) = interface.details.as_ref() {
                rows = rows.push(self.details_rows(details));
            }
        }
        self.section_card_for(route, rows)
            .title(tr("shell-wired"))
            .description(tr("shell-cable-connections-are-configured-automatically"))
            .into_element()
    }
    fn vpn_card<'a>(&'a self, network: Option<&Network>) -> Element<'a, Parent> {
        let route = "network/vpn";
        let mut rows = column![].spacing(2);
        let tunnels: Vec<&Interface> = network.map_or_else(Vec::new, |n| {
            n.interfaces
                .iter()
                .filter(|i| {
                    i.kind == InterfaceKind::Vpn && !n.vpns.iter().any(|v| v.name == i.connection)
                })
                .collect()
        });
        let writes = self.system_writes();
        match network {
            None => rows = rows.push(self.mark_setting(route, self.loading_row())),
            Some(network) => {
                if network.vpns.is_empty() && tunnels.is_empty() {
                    rows = rows.push(self.mark_setting(
                        route,
                        self.info_row(
                            "shield",
                            tr("shell-no-vpn-connections"),
                            tr("shell-import-a-wireguard-or-openvpn-profile-from-your-provider"),
                        ),
                    ));
                }
                const SHOWN: usize = 4;
                let limit = if self.system.all_vpns {
                    usize::MAX
                } else {
                    SHOWN.max(network.vpns.iter().filter(|v| v.active).count())
                };
                for (index, vpn) in network.vpns.iter().take(limit).enumerate() {
                    let uuid = vpn.uuid.clone();
                    let open = self.system.forms.vpn_open.as_deref() == Some(vpn.uuid.as_str());
                    let connecting =
                        self.busy_with(|r| matches!(r, Request::Vpn(u, _) if *u == vpn.uuid));
                    let kind = vpn.vpn.as_ref().map_or("VPN", |v| v.service.label());
                    let state = if connecting {
                        tr("shell-connecting")
                    } else if vpn.active {
                        tr("shell-connected")
                    } else {
                        tr("shell-off")
                    };
                    let row = crate::settings_app::settings_row(vpn.name.clone())
                        .subtitle(format!("{kind} · {state}"))
                        .icon("shield")
                        .control(
                            row![
                                enz_toolbar_icon(
                                    if open { "chevron-up" } else { "settings-2" },
                                    form(Form::VpnOpen(if open {
                                        None
                                    } else {
                                        Some(vpn.uuid.clone())
                                    }))
                                )
                                .id(format!("vpn-settings-{}", vpn.uuid).into())
                                .aria_label(enzui_shell::tr!("shell-a11y-options-for", "arg0" => enzui_shell::i18n::argument(&(vpn.name))))
                                .tooltip(enzui_shell::i18n::tr("shell-options"))
                                .into_element(),
                                {
                                    let toggle =
                                        enzui_widgets::switch::switch(vpn.active, move |on| {
                                            Self::request(Request::Vpn(uuid.clone(), on))
                                        })
                                        .label(vpn.name.clone());
                                    let toggle = if writes && !connecting {
                                        toggle
                                    } else {
                                        toggle.on_toggle_maybe(None::<fn(bool) -> Parent>)
                                    };
                                    Element::from(toggle)
                                },
                            ]
                            .spacing(10)
                            .align_y(Alignment::Center),
                        )
                        .into_element();
                    rows = rows.push(if index == 0 {
                        self.mark_setting(route, row)
                    } else {
                        row
                    });
                    if open {
                        let details =
                            vpn.vpn
                                .clone()
                                .unwrap_or(enzui_shell::system_settings::Vpn {
                                    service: enzui_shell::system_settings::VpnService::Other,
                                    route_all: true,
                                    metered: false,
                                    split_by_peers: false,
                                    sign_in: None,
                                });
                        let (all, metered) = (vpn.uuid.clone(), vpn.uuid.clone());
                        // An OpenVPN password profile needs its sign-in name in
                        // the profile; the password prompt cannot supply it.
                        let sign_in = details.sign_in.as_ref().map(|current| {
                            let uuid = vpn.uuid.clone();
                            // The field shows only what is being typed; the stored name is the caption.
                            let editing: Option<&str> = self.system.forms.vpn_username.as_ref().filter(|(id, _)| *id == uuid).map(|(_, name)| name.as_str());
                            let changed = editing.is_some_and(|name| Some(name.trim()) != current.as_deref() && enzui_shell::system_settings::valid_vpn_username(name));
                            let field_uuid = uuid.clone();
                            let placeholder = if current.is_some() { tr("shell-type-a-new-name-to-change-it") } else { tr("shell-the-name-your-vpn-provider-gave-you") };
                            let mut block = column![
                                enz_text_field(enzui_shell::i18n::current().text("shell-sign-in-name"), placeholder, editing.unwrap_or(""), move |t| form(Form::VpnUsername(field_uuid.clone(), t)))
                                    .on_submit(form(Form::VpnUsernameSave))
                                    .id(format!("vpn-username-{uuid}").into())
                                    .enabled(writes)
                                    .into_element(),
                            ].spacing(6);
                            if let Some(name) = current {
                                block = block.push(self.caption(tr!("shell-signs-in-as", "arg0" => name.clone())));
                            } else if editing.is_none() {
                                block = block.push(self.caption(tr("shell-this-vpn-asks-for-a-name-and-a-password-add-the-name-before-you-connect")));
                            }
                            if changed {
                                block = block.push(row![
                                    iced::widget::Space::new().width(Length::Fill),
                                    enz_button(enzui_shell::i18n::tr("shell-save"), form(Form::VpnUsernameSave)).kind(ButtonKind::Accent).size(ButtonSize::Sm).id(format!("vpn-username-save-{uuid}").into()).enabled(writes).into_element(),
                                ]);
                            }
                            Element::from(container(block).padding([4, 12]))
                        });
                        let options = column![
                            crate::settings_app::settings_row(tr("shell-send-all-traffic-through-the-vpn"))
                                .subtitle(if details.split_by_peers { tr("shell-only-the-vpn-s-own-networks-use-the-tunnel-as-the-provider-s-profile-sets") }
                                    else if details.route_all { tr("shell-everything-uses-the-tunnel") } else { tr("shell-only-the-vpn-s-own-networks-use-the-tunnel") })
                                .switch(details.route_all, move |on| Self::request(Request::VpnRouteAll { uuid: all.clone(), all: on }))
                                .enabled(writes && !details.split_by_peers)
                                .into_element(),
                            crate::settings_app::settings_row(tr("shell-metered-connection"))
                                .subtitle(tr("shell-apps-avoid-large-downloads-while-it-is-connected"))
                                .switch(details.metered, move |on| Self::request(Request::VpnMetered { uuid: metered.clone(), metered: on }))
                                .enabled(writes)
                                .into_element(),
                            row![
                                if vpn.active { self.caption(tr("shell-turn-the-vpn-off-to-remove-it")) } else { iced::widget::Space::new().into() },
                                iced::widget::Space::new().width(Length::Fill),
                                enz_button(tr("shell-remove-vpn"), Self::request(Request::VpnRemove(vpn.uuid.clone())))
                                    .id(format!("vpn-remove-{}", vpn.uuid).into())
                                    .kind(ButtonKind::DestructiveOutlined)
                                    .size(ButtonSize::Sm)
                                    .enabled(writes && !vpn.active)
                                    .into_element(),
                            ]
                            .padding([4, 12]),
                        ]
                        .spacing(2);
                        let options = match sign_in {
                            Some(sign_in) => column![sign_in, options].spacing(2),
                            None => options,
                        };
                        rows = rows.push(container(options).padding(iced::Padding {
                            left: 28.0,
                            ..iced::Padding::ZERO
                        }));
                    }
                }
                if network.vpns.len() > SHOWN {
                    let hidden = network.vpns.len().saturating_sub(limit);
                    rows = rows.push(
                        crate::settings_app::settings_row(if hidden > 0 {
                            tr!("shell-show-more-2", "arg0" => format!("{:}", hidden))
                        } else {
                            tr("shell-show-fewer").to_owned()
                        })
                        .icon(if hidden > 0 {
                            "chevron-down"
                        } else {
                            "chevron-up"
                        })
                        .action("", Parent::System(Message::ShowAllVpns(hidden > 0)))
                        .id(format!("vpn-show-all-{}", hidden > 0).into())
                        .into_element(),
                    );
                }
                for (index, tunnel) in tunnels.iter().enumerate() {
                    let row = crate::settings_app::settings_row(tunnel.device.clone())
                        .subtitle(tr("shell-managed-by-another-application"))
                        .icon("shield-check")
                        .control(self.status_badge(tunnel.state.label(), BadgeTone::Success))
                        .into_element();
                    rows = rows.push(if index == 0 && network.vpns.is_empty() {
                        self.mark_setting(route, row)
                    } else {
                        row
                    });
                }
                rows = rows.push(self.vpn_manual_form(writes));
                rows = rows.push(match &self.system.forms.vpn_path {
                    None => crate::settings_app::settings_row(tr("shell-import-a-vpn-profile"))
                        .icon("plus")
                        .action(
                            "",
                            Parent::FileChooser(
                                super::super::settings_file_chooser::Message::OpenVpn,
                            ),
                        )
                        .id("network-vpn-import".into())
                        .enabled(writes)
                        .into_element(),
                    Some(path) => {
                        let importing = self.busy_with(|r| matches!(r, Request::VpnImport(_)));
                        container(
                            column![
                                enz_text(path).into_element(),
                                row![
                                    iced::widget::Space::new().width(Length::Fill),
                                    enz_button(tr("shell-cancel"), form(Form::VpnPath(None)))
                                        .id("network-vpn-path-cancel".into())
                                        .kind(ButtonKind::Plain)
                                        .into_element(),
                                    enz_button(
                                        if importing {
                                            enzui_shell::i18n::tr("shell-importing")
                                        } else {
                                            enzui_shell::i18n::tr("shell-import")
                                        },
                                        form(Form::VpnSubmit)
                                    )
                                    .id("network-vpn-submit".into())
                                    .enabled(writes && !importing && !path.trim().is_empty())
                                    .into_element(),
                                ]
                                .spacing(8),
                            ]
                            .spacing(8),
                        )
                        .padding([8, 12])
                        .into()
                    }
                });
            }
        }
        self.section_card_for(route, rows)
            .title("VPN")
            .description(tr("shell-private-tunnels-to-another-network"))
            .into_element()
    }
    fn vpn_manual_form<'a>(&'a self, writes: bool) -> Element<'a, Parent> {
        use super::forms::VpnField;
        use enzui_shell::vpn_manual::Kind;
        let Some(spec) = &self.system.forms.vpn_manual else {
            return crate::settings_app::settings_row(tr("shell-vpn-add-manually"))
                .subtitle(tr("shell-vpn-add-manually-detail"))
                .icon("plus")
                .action("", form(Form::VpnManual(true)))
                .id("network-vpn-manual".into())
                .enabled(writes)
                .into_element();
        };
        let route = "network/vpn-kind";
        let adding = self.busy_with(|r| matches!(r, Request::VpnAdd(_)));
        let fields = spec.kind.fields();
        let choices: Vec<(String, Parent)> = Kind::ALL
            .into_iter()
            .map(|kind| (kind.label().to_owned(), form(Form::VpnManualKind(kind))))
            .collect();
        let mut rows = column![
            crate::settings_app::settings_row(tr("shell-vpn-type"))
                .subtitle(match spec.kind {
                    Kind::Pptp => tr("shell-vpn-pptp-warning"),
                    Kind::AnyConnect | Kind::GlobalProtect | Kind::Fortinet | Kind::Pulse =>
                        tr("shell-vpn-openconnect-detail"),
                    Kind::L2tp => tr("shell-vpn-l2tp-detail"),
                    Kind::Ikev2 => tr("shell-vpn-ikev2-detail"),
                    Kind::Vpnc => tr("shell-vpn-vpnc-detail"),
                })
                .subtitle_wrap(true)
                .icon("shield")
                .value_menu(
                    spec.kind.label().to_owned(),
                    self.menu.as_deref() == Some(route),
                    Parent::Menu(route.into(), true),
                    Parent::Menu(route.into(), false),
                    choices
                )
                .into_element(),
            crate::settings_app::settings_row(tr("shell-vpn-name"))
                .icon("tag")
                .entry(&spec.name, tr("shell-vpn-name-placeholder"), |t| form(
                    Form::VpnManualField(VpnField::Name, t)
                ))
                .into_element(),
            crate::settings_app::settings_row(tr("shell-vpn-server"))
                .icon("server")
                .entry(&spec.gateway, tr("shell-vpn-server-placeholder"), |t| form(
                    Form::VpnManualField(VpnField::Server, t)
                ))
                .into_element(),
        ]
        .spacing(2);
        if fields.method {
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-vpn-ikev2-psk"))
                    .subtitle(tr("shell-vpn-ikev2-psk-detail"))
                    .icon("key-round")
                    .switch(spec.psk, |on| form(Form::VpnManualPsk(on)))
                    .into_element(),
            );
        }
        if fields.group {
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-vpn-group"))
                    .icon("users")
                    .entry(&spec.group, tr("shell-vpn-group-placeholder"), |t| {
                        form(Form::VpnManualField(VpnField::Group, t))
                    })
                    .into_element(),
            );
        }
        if fields.user && !(spec.kind == Kind::Ikev2 && spec.psk) {
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-vpn-user"))
                    .subtitle(tr("shell-vpn-password-asked"))
                    .icon("user")
                    .entry(&spec.user, "", |t| {
                        form(Form::VpnManualField(VpnField::User, t))
                    })
                    .into_element(),
            );
        }
        let secret = match fields.secret {
            Some(enzui_shell::vpn_manual::Secret::PreShared) => Some(tr("shell-vpn-psk")),
            Some(enzui_shell::vpn_manual::Secret::Group) => Some(tr("shell-vpn-group-secret")),
            None if spec.kind == Kind::Ikev2 && spec.psk => Some(tr("shell-vpn-psk")),
            None => None,
        };
        if let Some(label) = secret {
            rows = rows.push(
                crate::settings_app::settings_row(label)
                    .icon("key-round")
                    .password(
                        &spec.secret,
                        "",
                        self.system.forms.vpn_manual_reveal,
                        |t| form(Form::VpnManualField(VpnField::Secret, t)),
                        form(Form::VpnManualReveal),
                    )
                    .into_element(),
            );
        }
        if let Some(problem) = self.system.forms.vpn_manual_error {
            rows = rows.push(enz_banner(problem).tone(BannerTone::Danger).into_element());
        }
        rows = rows.push(
            row![
                iced::widget::Space::new().width(Length::Fill),
                enz_button(tr("shell-cancel"), form(Form::VpnManual(false)))
                    .id("network-vpn-manual-cancel".into())
                    .kind(ButtonKind::Plain)
                    .into_element(),
                enz_button(
                    if adding {
                        tr("shell-vpn-adding")
                    } else {
                        tr("shell-vpn-add")
                    },
                    form(Form::VpnManualSubmit)
                )
                .kind(ButtonKind::Accent)
                .id("network-vpn-manual-submit".into())
                .enabled(writes && !adding)
                .into_element(),
            ]
            .spacing(8)
            .padding([6, 12]),
        );
        container(rows)
            .padding(4)
            .style({
                let palette = self.tokens().palette;
                move |_| container::Style {
                    background: Some(palette.surface_alt.into()),
                    border: iced::Border {
                        radius: 10.0.into(),
                        width: 1.0,
                        color: palette.border_strong,
                    },
                    ..Default::default()
                }
            })
            .into()
    }
    fn internet_card<'a>(&'a self, network: Option<&Network>) -> Element<'a, Parent> {
        use enzui_shell::network_connectivity::State;
        let route = "network/internet";
        let mut rows = column![].spacing(2);
        let writes = self.system_writes();
        match network {
            None => rows = rows.push(self.mark_setting(route, self.loading_row())),
            Some(network) => {
                let connectivity = &network.connectivity;
                let checking = self.busy_with(|r| matches!(r, Request::ConnectivityCheck));
                let (icon, subtitle) = match (connectivity.checks, connectivity.state) {
                    (Some(false), _) => ("globe", tr("shell-not-checked-internet-checks-are-off")),
                    (None, State::Unknown) => (
                        "globe",
                        tr("shell-this-system-does-not-check-internet-access"),
                    ),
                    (_, State::Full) => ("globe", tr("shell-this-computer-reaches-the-internet")),
                    (_, State::Portal) => (
                        "log-in",
                        tr("shell-this-network-asks-you-to-sign-in-before-it-lets-traffic-through"),
                    ),
                    (_, State::Limited) => (
                        "globe-lock",
                        tr(
                            "shell-connected-to-a-network-but-the-internet-does-not-answer-a-vpn-or-dns-setting",
                        ),
                    ),
                    (_, State::None) => ("globe", tr("shell-no-network-connection")),
                    (_, State::Unknown) => ("globe", tr("shell-not-checked-yet")),
                };
                let mut status =
                    crate::settings_app::settings_row(if connectivity.checks == Some(false) {
                        tr("shell-internet")
                    } else {
                        connectivity.state.label()
                    })
                    .subtitle(subtitle)
                    .icon(icon);
                if connectivity.checks == Some(true) {
                    status = status.control(
                        enz_button(
                            if checking {
                                tr("shell-checking")
                            } else {
                                tr("shell-check-again")
                            },
                            Self::request(Request::ConnectivityCheck),
                        )
                        .id("network-connectivity-check".into())
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .enabled(writes && !checking)
                        .into_element(),
                    );
                }
                rows = rows.push(self.mark_setting(route, status.into_element()));
                if connectivity.sign_in().is_some() && !self.system.forms.portal_dismissed {
                    rows = rows.push(
                        container(
                            row![
                                enz_text(tr(
                                    "shell-signing-in-opens-the-network-s-page-in-your-browser"
                                ))
                                .size(enzui_widgets::text::TextSize::Caption)
                                .wrap(true)
                                .into_element(),
                                iced::widget::Space::new().width(Length::Fill),
                                enz_button(tr("shell-not-now"), form(Form::PortalDismiss))
                                    .id("network-portal-dismiss".into())
                                    .kind(ButtonKind::Plain)
                                    .size(ButtonSize::Sm)
                                    .into_element(),
                                enz_button(
                                    tr("shell-sign-in"),
                                    Self::request(Request::PortalSignIn)
                                )
                                .kind(ButtonKind::Accent)
                                .size(ButtonSize::Sm)
                                .id("network-portal-sign-in".into())
                                .enabled(writes)
                                .into_element(),
                            ]
                            .spacing(8)
                            .align_y(Alignment::Center),
                        )
                        .padding([6, 12]),
                    );
                }
                if let Some(enabled) = connectivity.checks {
                    rows = rows.push(
                        crate::settings_app::settings_row(tr("shell-check-for-internet-access"))
                            .icon("radar")
                            .subtitle(tr("shell-contacts-a-fixed-address-now-and-then-to-notice-sign-in-pages-and-outages"))
                            .id_prefix("control-network/internet-checks")
                            .switch(enabled, |on| Self::request(Request::ConnectivityChecks(on)))
                            .enabled(writes)
                            .into_element(),
                    );
                }
            }
        }
        rows = rows.push(match network {
            Some(_) => self.airplane_row(network),
            None => self.mark_control_anchor("network/airplane", iced::widget::Space::new()),
        });
        self.section_card_for(route, rows)
            .title(tr("shell-internet"))
            .into_element()
    }
    fn firewall_card<'a>(&self, network: Option<&Network>) -> Element<'a, Parent> {
        let route = "network/firewall";
        let content: Element<'a, Parent> = match network.map(|n| n.firewall.as_ref()) {
            None => self.loading_row(),
            Some(None) => self.info_row(
                "shield-off",
                tr("shell-no-firewall-is-installed"),
                tr("shell-install-firewalld-to-filter-incoming-connections"),
            ),
            Some(Some(firewall)) => {
                crate::settings_app::settings_row(tr("shell-block-incoming-connections"))
                    .subtitle(if firewall.running {
                        tr("shell-on-public-networks-trusted-networks-still-allow-local-sharing")
                    } else {
                        tr("shell-off-incoming-connections-are-not-filtered")
                    })
                    .icon(if firewall.running {
                        "shield-check"
                    } else {
                        "shield-off"
                    })
                    .id_prefix(format!("control-{route}"))
                    .switch(firewall.running, |on| Self::request(Request::Firewall(on)))
                    .enabled(self.system_writes())
                    .into_element()
            }
        };
        self.section_card_for(route, self.mark_setting(route, content))
            .title(tr("shell-firewall"))
            .description(tr(
                "shell-whether-a-network-is-trusted-is-set-per-connection-under-its-details",
            ))
            .into_element()
    }
    fn airplane_row<'a>(&self, network: Option<&Network>) -> Element<'a, Parent> {
        let route = "network/airplane";
        let content: Element<'a, Parent> = match network {
            Some(network) if network.has_wifi() => {
                crate::settings_app::settings_row(tr("shell-airplane-mode"))
                    .subtitle(tr("shell-turns-off-wi-fi-and-bluetooth"))
                    .icon("plane")
                    .id_prefix(format!("control-{route}"))
                    .switch(network.airplane(), |on| {
                        Self::request(Request::Airplane(on))
                    })
                    .enabled(self.system_writes())
                    .into_element()
            }
            Some(_) => self.info_row(
                "plane",
                tr("shell-airplane-mode"),
                tr("shell-available-when-this-computer-has-a-wireless-radio"),
            ),
            None => self.loading_row(),
        };
        self.mark_setting(route, content)
    }
    pub(super) fn network_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let network = match self.reading(Panel::Network) {
            Some(Reading::Network(network)) => Some(network),
            _ => None,
        };
        let body = body
            .push(self.internet_card(network.as_ref()))
            .push(self.wifi_card(network.as_ref()))
            .push(self.wired_card(network.as_ref()))
            .push(self.vpn_card(network.as_ref()));
        if self.offers(enzui_shell::platform::Feature::FirewallSwitch) {
            body.push(self.firewall_card(network.as_ref()))
        } else {
            body
        }
    }
}

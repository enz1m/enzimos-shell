use super::*;
use iced::widget::column;

impl SettingsApp {
    fn remote_inset<'a>(&self, content: impl Into<Element<'a, Parent>>) -> Element<'a, Parent> {
        container(content)
            .padding(iced::Padding {
                top: 0.0,
                right: 12.0,
                bottom: 8.0,
                left: 44.0,
            })
            .into()
    }

    fn remote_progress<'a>(&'a self, kind: Kind) -> Option<Element<'a, Parent>> {
        let state = &self.system.remote;
        let spinner = |label: &'a str| -> Element<'a, Parent> {
            row![
                enzui_widgets::spinner::enz_spinner()
                    .size(14.0)
                    .into_element(),
                self.caption(label)
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center)
            .into()
        };
        if state.reviewing.is_some_and(|(_, k)| k == kind) {
            return Some(self.remote_inset(spinner(tr("shell-preparing-the-review"))));
        }
        if state.applying.is_some_and(|(_, k)| k == kind) {
            return Some(self.remote_inset(spinner(tr("shell-waiting-for-an-administrator"))));
        }
        match &state.review {
            Some((epoch, k, Ok(review))) if *k == kind => {
                let mut lines = column![].spacing(4);
                for line in &review.consequences {
                    lines = lines.push(self.caption(format!("• {line}")));
                }
                let verb = match &review.change {
                    Change::Ssh(request) if request.enabled => tr("shell-turn-on-ssh"),
                    Change::Ssh(_) => tr("shell-turn-off-ssh"),
                    Change::DisconnectSsh => tr("shell-disconnect"),
                    Change::Share(_) => tr("shell-start-sharing"),
                    Change::StopSharing => tr("shell-stop-sharing"),
                };
                let id = match kind {
                    Kind::Ssh => "remote-ssh",
                    Kind::Sharing => "remote-sharing",
                };
                lines = lines.push(
                    row![
                        iced::widget::Space::new().width(iced::Length::Fill),
                        enz_button(tr("shell-cancel"), message(RemoteMessage::Discard(*epoch)))
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Sm)
                            .id(format!("{id}-review-cancel").into())
                            .into_element(),
                        enz_button(verb, message(RemoteMessage::Apply(*epoch)))
                            .kind(ButtonKind::Accent)
                            .size(ButtonSize::Sm)
                            .id(format!("{id}-apply").into())
                            .enabled(self.controls_live())
                            .into_element(),
                    ]
                    .spacing(8),
                );
                lines = lines.push(self.caption(tr(
                    "shell-an-administrator-approves-this-change-nothing-changes-until-then",
                )));
                Some(self.remote_inset(lines))
            }
            Some((_, k, Err(error))) if *k == kind => {
                Some(self.remote_inset(self.caption(error.as_str())))
            }
            _ => match &state.outcome {
                Some((k, Err(error))) if *k == kind => {
                    Some(self.remote_inset(self.caption(error.as_str())))
                }
                _ => None,
            },
        }
    }

    fn remote_form<'a>(&'a self, draft: &'a Draft) -> Element<'a, Parent> {
        let state = &self.system.remote;
        let editable = !state.busy() && self.controls_live();
        let addresses = state
            .overview
            .as_ref()
            .and_then(Option::as_ref)
            .map(|o| o.addresses.clone())
            .unwrap_or_default();
        let id = match draft.kind {
            Kind::Ssh => "remote-ssh",
            Kind::Sharing => "remote-sharing",
        };
        let mut fields = column![].spacing(8);
        if addresses.is_empty() {
            fields = fields.push(self.caption(tr("shell-this-computer-has-no-network-address-to-listen-on-connect-to-a-network-first")));
        } else {
            let mut choices = column![].spacing(8);
            for (index, entry) in addresses.iter().enumerate() {
                let address = entry.address.clone();
                let label = format!(
                    "{} ({}) — {}",
                    entry.address, entry.interface, entry.network
                );
                if draft.kind == Kind::Ssh {
                    choices = choices.push(
                        enzui_widgets::checkbox::enz_checkbox(
                            draft.addresses.contains(&address),
                            label,
                            move |enabled| {
                                message(RemoteMessage::ListenAddress(address.clone(), enabled))
                            },
                        )
                        .id(format!("{id}-address-{index}").into())
                        .enabled(editable)
                        .into_element(),
                    );
                } else {
                    choices = choices.push(
                        enzui_widgets::radio_button::enz_radio_button(
                            label,
                            true,
                            Some(draft.address.as_ref() == Some(&address)),
                            move |_| message(RemoteMessage::Address(address.clone())),
                        )
                        .id(format!("{id}-address-{index}").into())
                        .enabled(editable)
                        .into_element(),
                    );
                }
            }
            fields = fields
                .push(enz_text(tr("shell-listen-on")).into_element())
                .push(choices)
                .push(self.caption(if draft.kind == Kind::Ssh {
                    tr("shell-ssh-select-addresses")
                } else {
                    tr("shell-only-this-address-of-the-computer-accepts-connections")
                }));
        }
        fields = fields.push(
            enz_text_field(
                enzui_shell::i18n::current().text("shell-allowed-networks"),
                "192.168.1.0/24",
                &draft.networks,
                |t| message(RemoteMessage::Networks(t)),
            )
            .description(enzui_shell::i18n::current().text(
                "shell-only-computers-on-these-networks-can-connect-separate-several-with-commas",
            ))
            .id(format!("{id}-networks").into())
            .enabled(editable)
            .into_element(),
        );
        fields = fields.push(
            enz_text_field(
                enzui_shell::i18n::current().text("shell-port"),
                if draft.kind == Kind::Ssh {
                    "22"
                } else {
                    "5900"
                },
                &draft.port,
                |t| message(RemoteMessage::Port(t)),
            )
            .id(format!("{id}-port").into())
            .enabled(editable)
            .into_element(),
        );
        match draft.kind {
            Kind::Ssh => {
                let mut keys = column![
                    enz_text(tr("shell-keys-that-may-sign-in"))
                        .weight_rung(enzui_theme::WeightRung::Semibold)
                        .into_element()
                ]
                .spacing(6);
                if draft.keys.is_empty() {
                    keys = keys.push(self.caption(tr("shell-no-keys-yet-paste-the-public-key-of-each-computer-you-will-connect-from-its")));
                }
                for (index, key) in draft.keys.iter().enumerate() {
                    let comment = key.split_whitespace().nth(2).unwrap_or("");
                    let label = format!(
                        "{} {comment}",
                        remote::key_fingerprint(key).unwrap_or_default()
                    );
                    keys = keys.push(
                        row![
                            enz_text(label)
                                .size(TextSize::Footnote)
                                .single_line_elided()
                                .width(iced::Length::Fill)
                                .into_element(),
                            enz_button(
                                enzui_shell::i18n::tr("shell-remove"),
                                message(RemoteMessage::RemoveKey(index))
                            )
                            .kind(ButtonKind::Plain)
                            .size(ButtonSize::Sm)
                            .id(format!("remote-ssh-key-remove-{index}").into())
                            .enabled(editable)
                            .into_element(),
                        ]
                        .spacing(8)
                        .align_y(iced::Alignment::Center),
                    );
                }
                keys = keys.push(
                    row![
                        enz_button(
                            tr("shell-ssh-select-key-file"),
                            Parent::FileChooser(
                                crate::settings_app::settings_file_chooser::Message::OpenSshKey
                            )
                        )
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .id("remote-ssh-key-file".into())
                        .enabled(editable && draft.keys.len() < 32)
                        .into_element(),
                        enz_button(
                            tr("shell-ssh-generate-key"),
                            message(RemoteMessage::GenerateKey)
                        )
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .id("remote-ssh-key-generate".into())
                        .enabled(editable && draft.keys.len() < 32)
                        .into_element(),
                    ]
                    .spacing(8),
                );
                if state.key_work.is_some() {
                    keys = keys.push(self.caption(tr("shell-ssh-preparing-key")));
                }
                if let Some(folder) = state.generated_folder.as_ref() {
                    keys = keys
                        .push(self.caption(tr("shell-ssh-generated-key-detail")))
                        .push(
                            row![
                                enz_text(folder.display().to_string())
                                    .size(TextSize::Footnote)
                                    .width(iced::Length::Fill)
                                    .into_element(),
                                enz_button(
                                    tr("shell-copy-folder-path"),
                                    message(RemoteMessage::Copy(folder.display().to_string()))
                                )
                                .kind(ButtonKind::Plain)
                                .size(ButtonSize::Sm)
                                .id("remote-ssh-key-folder-copy".into())
                                .enabled(editable)
                                .into_element(),
                            ]
                            .spacing(8)
                            .align_y(iced::Alignment::Center),
                        );
                }
                let addable = remote::key_fingerprint(draft.key_draft.trim()).is_some();
                let mut field = enz_text_field(
                    enzui_shell::i18n::current().text("shell-add-a-public-key"),
                    "ssh-ed25519 AAAA… you@laptop",
                    &draft.key_draft,
                    |t| message(RemoteMessage::KeyDraft(t)),
                )
                .on_submit(message(RemoteMessage::AddKey))
                .id("remote-ssh-key".into())
                .enabled(editable);
                if !draft.key_draft.trim().is_empty() && !addable {
                    field = field.error(tr("shell-this-is-not-an-openssh-public-key-line"));
                }
                keys = keys.push(field.into_element());
                keys = keys.push(row![
                    iced::widget::Space::new().width(iced::Length::Fill),
                    enz_button(tr("shell-add-key"), message(RemoteMessage::AddKey))
                        .kind(ButtonKind::Secondary)
                        .size(ButtonSize::Sm)
                        .id("remote-ssh-key-add".into())
                        .enabled(editable && addable)
                        .into_element(),
                ]);
                fields = fields
                    .push(keys)
                    .push(
                        crate::settings_app::settings_row(tr("shell-ssh-allow-password"))
                            .subtitle(tr("shell-ssh-password-detail"))
                            .id_prefix("remote-ssh-password-auth")
                            .switch(draft.allow_password, |on| {
                                message(RemoteMessage::PasswordAuth(on))
                            })
                            .enabled(editable)
                            .into_element(),
                    )
                    .push(
                        crate::settings_app::settings_row(tr("shell-start-with-the-computer"))
                            .subtitle(tr("shell-listen-from-startup-before-anyone-signs-in-here"))
                            .id_prefix("control-remote-ssh-autostart")
                            .switch(draft.autostart, |on| message(RemoteMessage::Autostart(on)))
                            .enabled(editable)
                            .into_element(),
                    );
            }
            Kind::Sharing => {
                fields = fields.push(
                    crate::settings_app::settings_row(tr("shell-allow-control"))
                        .subtitle(if draft.control {
                            tr("shell-viewers-can-use-the-mouse-and-keyboard")
                        } else {
                            tr("shell-viewers-can-only-watch")
                        })
                        .id_prefix("control-remote-sharing-control")
                        .switch(draft.control, |on| message(RemoteMessage::Control(on)))
                        .enabled(editable)
                        .into_element(),
                );
            }
        }
        if state
            .review
            .as_ref()
            .is_none_or(|(_, k, result)| *k != draft.kind || result.is_err())
            && state.reviewing.is_none()
            && state.applying.is_none()
        {
            fields = fields.push(
                row![
                    iced::widget::Space::new().width(iced::Length::Fill),
                    enz_button(tr("shell-cancel"), message(RemoteMessage::CancelEdit))
                        .kind(ButtonKind::Plain)
                        .id(format!("{id}-cancel").into())
                        .into_element(),
                    enz_button(
                        enzui_shell::i18n::tr("shell-review-2"),
                        message(RemoteMessage::ReviewDraft)
                    )
                    .id(format!("{id}-review").into())
                    .enabled(
                        editable && self.system.backend.writes_allowed && draft.address.is_some()
                    )
                    .into_element(),
                ]
                .spacing(8),
            );
        }
        container(fields).padding([8, 12]).into()
    }

    pub(in crate::settings_app) fn remote_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let state = &self.system.remote;
        let (ssh_route, sharing_route) = ("remote/ssh", "remote/sharing");
        let overview = match &state.overview {
            Some(Some(overview)) => overview,
            missing => {
                let row = |route: &'static str| {
                    match missing {
                    None => self.mark_setting(route, self.loading_row()),
                    Some(_) => self.mark_setting(route, self.info_row("screen-share", tr("shell-remote-access-is-not-available"), tr("shell-this-system-does-not-include-the-enzimos-remote-access-service"))),
                }
                };
                return body
                    .push(
                        self.section_card_for(ssh_route, column![row(ssh_route)])
                            .title(tr("shell-remote-sign-in-ssh"))
                            .into_element(),
                    )
                    .push(
                        self.section_card_for(sharing_route, column![row(sharing_route)])
                            .title(tr("shell-desktop-sharing"))
                            .into_element(),
                    );
            }
        };
        let idle = !state.busy() && self.controls_live() && self.system.backend.writes_allowed;
        let draft_open = |kind: Kind| state.draft.as_ref().filter(|d| d.kind == kind);
        let small = |label: &'a str,
                     msg: RemoteMessage,
                     id: &'static str,
                     enabled: bool|
         -> Element<'a, Parent> {
            enz_button(label, message(msg))
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .id(id.into())
                .enabled(enabled)
                .into_element()
        };

        let mut ssh_rows = column![].spacing(2);
        match &overview.ssh {
            Err(error) => {
                ssh_rows = ssh_rows.push(self.mark_setting(
                    ssh_route,
                    self.info_row(
                        "triangle-alert",
                        tr("shell-remote-sign-in-could-not-be-read"),
                        error.clone(),
                    ),
                ));
            }
            Ok(ssh) if ssh.other_account => {
                ssh_rows = ssh_rows.push(self.mark_setting(
                    ssh_route,
                    self.info_row(
                        "user",
                        tr("shell-another-account-set-up-remote-sign-in"),
                        tr("shell-only-that-account-can-change-it-while-it-is-set-up"),
                    ),
                ));
            }
            Ok(ssh) => {
                let on = ssh.setup.as_ref().is_some_and(|s| s.enabled);
                let subtitle = match (&ssh.setup, ssh.listening) {
                    (Some(setup), true) => {
                        tr!("shell-on-port-from", "arg0" => setup.addresses.join(", "), "arg1" => format!("{:}", setup.endpoint.port), "arg2" => setup.endpoint.networks.join(", "))
                    }
                    (Some(setup), false) if setup.enabled => match &ssh.failure {
                        Some(detail) => {
                            tr!("shell-turned-on-but-it-did-not-start-change-reviews-it-again", "arg0" => detail.clone())
                        }
                        None => tr("shell-turned-on-but-not-listening-now-change-reviews-it-again")
                            .into(),
                    },
                    _ => tr("shell-off-nothing-on-this-computer-accepts-ssh-connections").into(),
                };
                let mut controls = row![].spacing(8);
                if on {
                    controls = controls.push(small(
                        tr("shell-turn-off-2"),
                        RemoteMessage::ReviewOff(Kind::Ssh),
                        "remote-ssh-off",
                        idle,
                    ));
                }
                if draft_open(Kind::Ssh).is_none() {
                    controls = controls.push(small(
                        if on {
                            tr("shell-change")
                        } else {
                            tr("shell-set-up-2")
                        },
                        RemoteMessage::Edit(Kind::Ssh),
                        "remote-ssh-edit",
                        idle && overview.capabilities.ssh,
                    ));
                }
                let mut tokens = crate::settings_app::settings_row_tokens();
                tokens.inline_field_width = 240.0;
                let row = crate::settings_app::settings_row(tr("shell-ssh-and-sftp"))
                    .tokens(tokens)
                    .subtitle(subtitle)
                    .subtitle_wrap(true)
                    .icon("terminal")
                    .control(Element::from(controls))
                    .into_element();
                ssh_rows = ssh_rows.push(self.mark_setting(ssh_route, row));
                if let Some(setup) = ssh.setup.as_ref().filter(|s| s.enabled) {
                    if let Some(host) = &ssh.host_key {
                        ssh_rows = ssh_rows
                            .push(self.detail_row(tr("shell-this-computer-s-key"), host.clone()));
                    }
                    ssh_rows = ssh_rows.push(self.detail_row(
                        tr("shell-ssh-authentication"),
                        if setup.allow_password {
                            tr("shell-ssh-key-or-password")
                        } else {
                            tr("shell-ssh-key-only")
                        },
                    ));
                    for key in &setup.keys {
                        ssh_rows = ssh_rows.push(self.detail_row(
                            tr("shell-allowed-key"),
                            format!("{} ({})", key.fingerprint, key.kind),
                        ));
                    }
                    ssh_rows = ssh_rows.push(self.detail_row(
                        tr("shell-starts"),
                        if setup.autostart {
                            tr("shell-with-the-computer")
                        } else {
                            tr("shell-only-until-the-computer-restarts")
                        },
                    ));
                }
                if !ssh.sessions.is_empty() {
                    let sessions = ssh.sessions.join(", ");
                    let row = crate::settings_app::settings_row(if ssh.sessions.len() == 1 { tr("shell-1-session-connected").to_owned() } else { tr!("shell-sessions-connected", "arg0" => format!("{:}", ssh.sessions.len())) })
                        .subtitle(tr!("shell-from", "arg0" => sessions))
                        .subtitle_wrap(true)
                        .icon("users")
                        .control(small(tr("shell-disconnect-2"), RemoteMessage::ReviewDisconnect, "remote-ssh-disconnect", idle))
                        .into_element();
                    ssh_rows = ssh_rows.push(row);
                }
                if let Some(draft) = draft_open(Kind::Ssh) {
                    ssh_rows = ssh_rows.push(self.remote_form(draft));
                }
            }
        }
        if let Some(progress) = self.remote_progress(Kind::Ssh) {
            ssh_rows = ssh_rows.push(progress);
        }

        let mut sharing_rows = column![].spacing(2);
        match &overview.sharing {
            Err(error) => {
                sharing_rows = sharing_rows.push(self.mark_setting(
                    sharing_route,
                    self.info_row(
                        "screen-share",
                        tr("shell-desktop-sharing-is-not-available"),
                        error.clone(),
                    ),
                ));
            }
            Ok(sharing) => {
                let active = sharing.listening
                    || matches!(
                        sharing.state.as_str(),
                        "active" | "activating" | "reloading"
                    );
                let subtitle = match (&sharing.setup, sharing.listening) {
                    (Some(setup), true) => {
                        tr!("shell-sharing-port", "arg0" => setup.endpoint.address.clone(), "arg1" => format!("{:}", setup.endpoint.port), "arg2" => (if setup.view_only { tr("shell-view-only") } else { tr("shell-viewers-can-control") }).to_owned())
                    }
                    _ if active => tr("shell-starting").into(),
                    _ => {
                        tr("shell-off-sharing-lasts-until-you-stop-it-lock-the-screen-or-sign-out")
                            .into()
                    }
                };
                let mut controls = row![].spacing(8);
                if active || sharing.cleanup_pending {
                    controls = controls.push(small(
                        tr("shell-stop-sharing-2"),
                        RemoteMessage::ReviewOff(Kind::Sharing),
                        "remote-sharing-stop",
                        idle,
                    ));
                } else if draft_open(Kind::Sharing).is_none() {
                    controls = controls.push(small(
                        tr("shell-share"),
                        RemoteMessage::Edit(Kind::Sharing),
                        "remote-sharing-edit",
                        idle,
                    ));
                }
                let row = crate::settings_app::settings_row(tr("shell-share-this-desktop"))
                    .subtitle(subtitle)
                    .subtitle_wrap(true)
                    .icon("screen-share")
                    .control(Element::from(controls))
                    .into_element();
                sharing_rows = sharing_rows.push(self.mark_setting(sharing_route, row));
                if sharing.listening {
                    let viewers = sharing.viewers.map_or_else(
                        || tr("shell-viewer-count-unavailable").to_owned(),
                        |count| tr!("shell-authenticated-viewers", "arg0" => count.to_string()),
                    );
                    sharing_rows = sharing_rows.push(self.caption_row(viewers));
                }
                if sharing.cleanup_pending {
                    sharing_rows = sharing_rows.push(self.caption_row(tr("shell-sharing-ended-but-its-network-opening-is-still-being-closed-stop-sharing-to")));
                }
                if sharing.listening
                    && let Some(credentials) = &state.credentials
                {
                    let password = if state.show_password {
                        credentials.password.clone()
                    } else {
                        "•".repeat(12)
                    };
                    sharing_rows = sharing_rows.push(
                        crate::settings_app::settings_row(tr("shell-sharing-password"))
                            .subtitle(password)
                            .icon("key-round")
                            .control(Element::from(
                                row![
                                    small(
                                        if state.show_password {
                                            enzui_shell::i18n::tr("shell-hide")
                                        } else {
                                            enzui_shell::i18n::tr("shell-show")
                                        },
                                        RemoteMessage::ShowPassword(!state.show_password),
                                        "remote-sharing-password-show",
                                        true
                                    ),
                                    small(
                                        enzui_shell::i18n::tr("shell-copy"),
                                        RemoteMessage::Copy(credentials.password.clone()),
                                        "remote-sharing-password-copy",
                                        true
                                    ),
                                ]
                                .spacing(8),
                            ))
                            .into_element(),
                    );
                    sharing_rows = sharing_rows.push(
                        self.detail_row(tr("shell-certificate"), credentials.fingerprint.clone()),
                    );
                    sharing_rows = sharing_rows.push(self.caption_row(tr("shell-give-the-viewer-this-password-separately-from-your-account-password-their-vn")));
                }
                if let Some(draft) = draft_open(Kind::Sharing) {
                    sharing_rows = sharing_rows.push(self.remote_form(draft));
                }
            }
        }
        if let Some(progress) = self.remote_progress(Kind::Sharing) {
            sharing_rows = sharing_rows.push(progress);
        }

        body.push(
            self.section_card_for(ssh_route, ssh_rows)
                .title(tr("shell-remote-sign-in-ssh"))
                .description(tr("shell-open-a-terminal-and-copy-files-sftp-from-another-computer-on-your-network-si"))
                .into_element(),
        )
        .push(
            self.section_card_for(sharing_route, sharing_rows)
                .title(tr("shell-desktop-sharing"))
                .description(tr("shell-let-someone-on-your-network-see-this-screen-and-use-it-if-you-allow-over-an"))
                .into_element(),
        )
    }
}

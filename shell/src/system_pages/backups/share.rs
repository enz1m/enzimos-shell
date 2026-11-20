//! The network share (NAS) copy of encrypted backups: its row, the set-up
//! dialog, testing and removal.
use super::*;
use enzui_core::form::SecretText;
use enzui_core::menu_model::{MenuEntryModel, MenuModel, MenuState};
use enzui_shell::i18n::tr;
use enzui_shell::system_settings::backup_remote::{self, RemoteShare, ShareInput, Tested};
use enzui_widgets::banner::{BannerTone, enz_banner};
use enzui_widgets::menu_button::{MenuButtonEvent, apply_event, enz_kebab_menu_button};
use enzui_widgets::modal_host::ModalHostEvent;
use enzui_widgets::secure_field::enz_secure_field;
use iced::widget::column;

pub(super) struct Share {
    form: Option<Form>,
    testing: bool,
    tested: Option<Result<Tested, String>>,
    confirm_remove: bool,
    removing: bool,
    notice: Option<String>,
    menu: MenuModel,
    menu_state: MenuState,
}

impl Default for Share {
    fn default() -> Self {
        Self {
            form: None,
            testing: false,
            tested: None,
            confirm_remove: false,
            removing: false,
            notice: None,
            menu: MenuModel::new(vec![
                MenuEntryModel::command("change", tr("shell-share-change")),
                MenuEntryModel::command("remove", tr("shell-share-remove")).destructive(true),
            ]),
            menu_state: MenuState::closed(),
        }
    }
}

struct Form {
    input: ShareInput,
    password: SecretText,
    repository: SecretText,
    repeat: SecretText,
    revealed: [bool; 3],
    more: bool,
    working: bool,
    error: Option<String>,
    needs_repository: bool,
    change: bool,
}

impl Form {
    fn ready(&self) -> bool {
        !self.working
            && self.input.complete()
            && backup_remote::validate_share_password(&self.password).is_ok()
            && (!self.needs_repository
                || (backup_setup_password_ok(&self.repository)
                    && self.repository.expose_secret() == self.repeat.expose_secret()))
    }
}

fn backup_setup_password_ok(password: &SecretText) -> bool {
    enzui_shell::system_settings::backup_setup::validate_password(password).is_ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::settings_app) enum Field {
    Server,
    Share,
    Folder,
    User,
    Domain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::settings_app) enum Secret {
    Share,
    Repository,
    Repeat,
}

#[derive(Clone, Debug)]
pub(in crate::settings_app) enum ShareMessage {
    Open,
    Field(Field, String),
    Secret(Secret, SecretText),
    Reveal(Secret),
    More(bool),
    Submit,
    Done(Result<String, String>),
    Close,
    Host(ModalHostEvent),
    Test,
    Tested(Result<Tested, String>),
    Menu(MenuButtonEvent),
    Remove(bool),
    ConfirmRemove,
    Removed(Result<(), String>),
}

fn send(message: ShareMessage) -> Parent {
    super::send(BackupsMessage::Share(message))
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|name| name.trim().to_owned())
        .unwrap_or_default()
}

impl Share {
    pub(super) fn busy(&self) -> bool {
        self.form.as_ref().is_some_and(|form| form.working) || self.testing || self.removing
    }
    pub(super) fn leave(&mut self) {
        if !self.busy() {
            self.form = None;
        }
        self.confirm_remove = false;
    }
}

impl SettingsApp {
    pub(super) fn backup_share_message(&mut self, message: ShareMessage) -> Task<Parent> {
        let backend = self.system.backend.clone();
        let remote = self
            .safety_now()
            .map(|safety| safety.remote)
            .unwrap_or_default();
        let demo = backend.demo;
        let share = &mut self.system.backups.share;
        match message {
            ShareMessage::Open => {
                if share.busy() {
                    return Task::none();
                }
                share.notice = None;
                share.confirm_remove = false;
                let host = if demo {
                    "enzimos-laptop".into()
                } else {
                    hostname()
                };
                share.form = Some(Form {
                    input: ShareInput::suggested(&remote, &host),
                    password: SecretText::new(String::new()),
                    repository: SecretText::new(String::new()),
                    repeat: SecretText::new(String::new()),
                    revealed: [false; 3],
                    more: !remote.domain.is_empty(),
                    working: false,
                    error: None,
                    needs_repository: remote.needs_repository_password(),
                    change: remote.configured(),
                });
                return iced::widget::operation::focus(iced::widget::Id::new(
                    "backup-share-server",
                ));
            }
            ShareMessage::Field(field, text) => {
                if let Some(form) = share.form.as_mut().filter(|form| !form.working) {
                    let text: String = text
                        .chars()
                        .filter(|c| !c.is_control())
                        .take(1024)
                        .collect();
                    let slot = match field {
                        Field::Server => &mut form.input.server,
                        Field::Share => &mut form.input.share,
                        Field::Folder => &mut form.input.folder,
                        Field::User => &mut form.input.user,
                        Field::Domain => &mut form.input.domain,
                    };
                    *slot = text;
                    form.error = None;
                }
            }
            ShareMessage::Secret(which, secret) => {
                if let Some(form) = share.form.as_mut().filter(|form| !form.working) {
                    match which {
                        Secret::Share => form.password = secret,
                        Secret::Repository => form.repository = secret,
                        Secret::Repeat => form.repeat = secret,
                    }
                    form.error = None;
                }
            }
            ShareMessage::Reveal(which) => {
                if let Some(form) = share.form.as_mut() {
                    let index = which as usize;
                    form.revealed[index] = !form.revealed[index];
                }
            }
            ShareMessage::More(open) => {
                if let Some(form) = share.form.as_mut() {
                    form.more = open;
                }
            }
            ShareMessage::Submit => {
                let Some(form) = share.form.as_mut().filter(|form| form.ready()) else {
                    return Task::none();
                };
                form.working = true;
                form.error = None;
                let input = form.input.clone();
                let password = form.password.clone();
                let repository = form.needs_repository.then(|| form.repository.clone());
                let needs = form.needs_repository;
                return Task::perform(
                    async move {
                        backend
                            .remote_share_setup(input, password, repository, needs)
                            .await
                    },
                    |result| send(ShareMessage::Done(result)),
                );
            }
            ShareMessage::Done(result) => {
                match result {
                    Ok(repository) => {
                        share.form = None;
                        share.tested = None;
                        share.notice = Some(
                            if repository == "adopted" {
                                tr("shell-share-ready-adopted")
                            } else {
                                tr("shell-share-ready-created")
                            }
                            .into(),
                        );
                    }
                    Err(error) => {
                        if let Some(form) = share.form.as_mut() {
                            form.working = false;
                            form.error = Some(error);
                        }
                    }
                }
                // Also after a late or failed answer: the status says what happened.
                return self.read_system(true);
            }
            ShareMessage::Close | ShareMessage::Host(ModalHostEvent::Dismissed(_)) => {
                if !share.form.as_ref().is_some_and(|form| form.working) {
                    share.form = None;
                }
            }
            ShareMessage::Host(event) => {
                return enzui_widgets::modal_host::focus_wrap_operation(event)
                    .map_or_else(Task::none, iced::advanced::widget::operate);
            }
            ShareMessage::Test => {
                if share.busy() {
                    return Task::none();
                }
                share.testing = true;
                share.tested = None;
                share.notice = None;
                return Task::perform(async move { backend.remote_share_test().await }, |result| {
                    send(ShareMessage::Tested(result))
                });
            }
            ShareMessage::Tested(result) => {
                share.testing = false;
                share.tested = Some(result);
            }
            ShareMessage::Menu(event) => {
                let activated = apply_event(&mut share.menu, &mut share.menu_state, &event);
                match activated
                    .as_ref()
                    .map(enzui_core::menu_model::EntryId::as_str)
                {
                    Some("change") => return self.backup_share_message(ShareMessage::Open),
                    Some("remove") => return self.backup_share_message(ShareMessage::Remove(true)),
                    _ => {}
                }
            }
            ShareMessage::Remove(ask) => {
                if !share.busy() {
                    share.confirm_remove = ask;
                    share.notice = None;
                }
            }
            ShareMessage::ConfirmRemove => {
                if share.busy() {
                    return Task::none();
                }
                share.removing = true;
                return Task::perform(
                    async move { backend.remote_share_remove().await },
                    |result| send(ShareMessage::Removed(result)),
                );
            }
            ShareMessage::Removed(result) => {
                share.removing = false;
                share.confirm_remove = false;
                share.tested = None;
                share.notice = Some(match result {
                    Ok(()) => tr("shell-share-removed").into(),
                    Err(error) => error,
                });
                return self.read_system(true);
            }
        }
        Task::none()
    }

    /// The network share rows of the Encrypted backups card.
    pub(super) fn backup_share_rows<'a>(&'a self, safety: &Safety) -> Element<'a, Parent> {
        let share = &self.system.backups.share;
        let remote: &RemoteShare = &safety.remote;
        let writes = self.system_writes();
        let small = |label: &'static str, message: ShareMessage, id: &'static str| {
            enz_button(label, send(message))
                .kind(ButtonKind::Secondary)
                .size(ButtonSize::Sm)
                .id(id.into())
                .enabled(writes && !share.busy())
                .into_element()
        };
        let mut rows = column![].spacing(2);
        let row = match remote.state.as_str() {
            "configured" => {
                let menu = enz_kebab_menu_button(&share.menu, &share.menu_state, |event| {
                    send(ShareMessage::Menu(event))
                })
                .enabled(writes && !share.busy())
                .alignment(enzui_core::placement::Alignment::End)
                .menu_id("backup-share-menu".into())
                .into_element();
                crate::settings_app::settings_row(tr("shell-network-share"))
                    .subtitle(format!("{} · {}", remote.address(), remote.account()))
                    .icon("network")
                    .id_prefix("backup-share")
                    .control(
                        row![
                            enz_button(
                                if share.testing {
                                    tr("shell-share-testing")
                                } else {
                                    tr("shell-share-test")
                                },
                                send(ShareMessage::Test),
                            )
                            .kind(ButtonKind::Secondary)
                            .size(ButtonSize::Sm)
                            .id("backup-share-test".into())
                            .enabled(writes && !share.busy())
                            .into_element(),
                            menu,
                        ]
                        .spacing(4)
                        .align_y(iced::Alignment::Center),
                    )
                    .into_element()
            }
            "reconfigure-as-smb" => {
                crate::settings_app::settings_row(tr("shell-share-set-up-again"))
                    .subtitle(tr("shell-share-reconfigure-detail"))
                    .subtitle_wrap(true)
                    .icon("triangle-alert")
                    .control(small(
                        tr("shell-share-set-up"),
                        ShareMessage::Open,
                        "backup-share-setup",
                    ))
                    .into_element()
            }
            state => crate::settings_app::settings_row(tr("shell-network-share"))
                .subtitle(if state == "invalid" {
                    tr("shell-share-invalid")
                } else {
                    tr("shell-share-not-set-up")
                })
                .icon("network")
                .control(small(
                    tr("shell-share-set-up"),
                    ShareMessage::Open,
                    "backup-share-setup",
                ))
                .into_element(),
        };
        rows = rows.push(row);
        if remote.configured()
            && let Some(tested) = &share.tested
        {
            let (icon, title, detail) = match tested {
                Ok(tested) => (
                    if tested.ok() {
                        "circle-check"
                    } else {
                        "circle-alert"
                    },
                    tested.summary(),
                    test_details(tested),
                ),
                Err(error) => ("circle-alert", error.clone(), String::new()),
            };
            rows = rows.push(
                iced::widget::container(self.info_row(icon, title, detail)).padding(
                    iced::Padding {
                        left: 34.0,
                        ..iced::Padding::ZERO
                    },
                ),
            );
        }
        if share.confirm_remove && remote.configured() {
            rows = rows.push(
                crate::settings_app::settings_row(tr("shell-share-remove-title"))
                    .subtitle(tr("shell-share-remove-detail"))
                    .subtitle_wrap(true)
                    .icon("trash-2")
                    .accessory_width(enzui_theme::text_scale::ui_px(220.0))
                    .control(
                        row![
                            enz_button(tr("shell-cancel"), send(ShareMessage::Remove(false)))
                                .kind(ButtonKind::Secondary)
                                .size(ButtonSize::Sm)
                                .enabled(!share.removing)
                                .into_element(),
                            enz_button(
                                if share.removing {
                                    tr("shell-share-removing")
                                } else {
                                    tr("shell-share-remove-confirm")
                                },
                                send(ShareMessage::ConfirmRemove),
                            )
                            .kind(ButtonKind::Destructive)
                            .size(ButtonSize::Sm)
                            .id("backup-share-remove".into())
                            .enabled(writes && !share.removing)
                            .into_element(),
                        ]
                        .spacing(6),
                    )
                    .into_element(),
            );
        }
        if let Some(notice) = &share.notice {
            rows = rows.push(
                iced::widget::container(self.caption(notice.clone())).padding(iced::Padding {
                    top: 2.0,
                    right: 10.0,
                    bottom: 6.0,
                    left: 44.0,
                }),
            );
        }
        rows.into()
    }

    /// The set-up dialog, while open.
    pub(in crate::settings_app) fn backup_share_dialog<'a>(
        &'a self,
    ) -> Option<Element<'a, Parent>> {
        if self.page != Category::Backups {
            return None;
        }
        let form = self.system.backups.share.form.as_ref()?;
        let current = enzui_shell::i18n::current();
        let enabled = !form.working;
        let field =
            |field: Field, label: String, placeholder: String, value: &'a str, id: &'static str| {
                text_input(field, label, placeholder, value, id, enabled)
            };
        let secret = |which: Secret, label: String, value: &'a str, id: &'static str| {
            secret_input(
                which,
                label,
                value,
                form.revealed[which as usize],
                id,
                enabled,
            )
        };
        let mut body = column![
            self.caption(if form.needs_repository {
                tr("shell-share-note-own-password")
            } else {
                tr("shell-share-note")
            }),
            field(
                Field::Server,
                current.text("shell-share-server"),
                "nas.local".into(),
                &form.input.server,
                "backup-share-server"
            ),
            field(
                Field::Share,
                current.text("shell-share-share"),
                "backups".into(),
                &form.input.share,
                "backup-share-share"
            ),
            field(
                Field::Folder,
                current.text("shell-share-folder"),
                "enzimos/laptop".into(),
                &form.input.folder,
                "backup-share-folder"
            ),
            field(
                Field::User,
                current.text("shell-share-user"),
                String::new(),
                &form.input.user,
                "backup-share-user"
            ),
            secret(
                Secret::Share,
                current.text("shell-share-password"),
                form.password.expose_secret(),
                "backup-share-password"
            )
            .into_element(),
        ]
        .spacing(12);
        if form.needs_repository {
            let entered = !form.repository.expose_secret().is_empty();
            let invalid = entered && !backup_setup_password_ok(&form.repository);
            let mismatch = !invalid
                && !form.repeat.expose_secret().is_empty()
                && form.repeat.expose_secret() != form.repository.expose_secret();
            let field = secret(
                Secret::Repository,
                current.text("shell-share-repository-password"),
                form.repository.expose_secret(),
                "backup-share-repository",
            )
            .description(current.text("shell-share-repository-password-detail"));
            body = body.push(
                if invalid {
                    field.error(
                        current.text("shell-enter-a-single-line-password-of-1-4096-utf-8-bytes"),
                    )
                } else {
                    field
                }
                .into_element(),
            );
            let repeat = secret(
                Secret::Repeat,
                current.text("shell-repeat-the-new-password"),
                form.repeat.expose_secret(),
                "backup-share-repeat",
            );
            body = body.push(
                if mismatch {
                    repeat.error(current.text("shell-the-passwords-don-t-match"))
                } else {
                    repeat
                }
                .into_element(),
            );
        }
        body = body.push(
            enz_button(
                tr("shell-share-more-options"),
                send(ShareMessage::More(!form.more)),
            )
            .icon(if form.more {
                "chevron-down"
            } else {
                "chevron-right"
            })
            .kind(ButtonKind::Ghost)
            .size(ButtonSize::Sm)
            .id("backup-share-more".into())
            .into_element(),
        );
        if form.more {
            body = body.push(
                enz_text_field(
                    current.text("shell-share-domain"),
                    "WORKGROUP",
                    &form.input.domain,
                    |text| send(ShareMessage::Field(Field::Domain, text)),
                )
                .description(current.text("shell-share-domain-detail"))
                .id("backup-share-domain".into())
                .enabled(enabled)
                .into_element(),
            );
        }
        if form.working {
            body = body.push(
                enz_banner(tr("shell-share-working"))
                    .tone(BannerTone::Info)
                    .icon("loader")
                    .body(tr("shell-share-working-detail"))
                    .into_element(),
            );
        } else if let Some(error) = &form.error {
            body = body.push(
                enz_banner(tr("shell-share-failed"))
                    .tone(BannerTone::Danger)
                    .body(error.clone())
                    .id("backup-share-error")
                    .into_element(),
            );
        }
        let close = send(ShareMessage::Close);
        let header = enz_text(if form.change {
            tr("shell-share-change-title")
        } else {
            tr("shell-share-set-up-title")
        })
        .size_px(self.text_px(18.0))
        .weight_rung(enzui_theme::WeightRung::Semibold)
        .into_element();
        let footer: Element<'a, Parent> = row![
            iced::widget::space::horizontal(),
            enz_button(tr("shell-cancel"), close.clone())
                .kind(ButtonKind::Secondary)
                .enabled(!form.working)
                .into_element(),
            enz_button(
                if form.working {
                    tr("shell-share-setting-up")
                } else {
                    tr("shell-share-set-up-button")
                },
                send(ShareMessage::Submit),
            )
            .kind(ButtonKind::Primary)
            .id("backup-share-submit".into())
            .enabled(self.system_writes() && form.ready())
            .into_element(),
        ]
        .spacing(8)
        .into();
        let scroll = enzui_widgets::scroll_view::enz_scroll_view(Element::from(
            iced::widget::container(body).padding(iced::Padding {
                right: 12.0,
                ..iced::Padding::ZERO
            }),
        ))
        .id("backup-share-body".into())
        .scrollbar_tokens(self.tokens().scrollbar)
        .into_element();
        let content = enzui_widgets::capped_dialog_body::enz_capped_dialog_body(
            vec![header],
            scroll,
            vec![footer],
            16.0,
            (self.viewport.height - 64.0).clamp(160.0, 720.0),
        )
        .into_element();
        Some(
            enzui_widgets::modal_surface::enz_modal_surface(content)
                .width(520.0)
                .padding(20.0)
                .on_dismiss(move || close.clone())
                .close_label(tr("shell-cancel"))
                .into_element(),
        )
    }
}

fn text_input<'a>(
    field: Field,
    label: String,
    placeholder: String,
    value: &'a str,
    id: &'static str,
    enabled: bool,
) -> Element<'a, Parent> {
    enz_text_field(label, placeholder, value, move |text| {
        send(ShareMessage::Field(field, text))
    })
    .id(id.into())
    .enabled(enabled)
    .into_element()
}

fn secret_input<'a>(
    which: Secret,
    label: String,
    value: &'a str,
    revealed: bool,
    id: &'static str,
    enabled: bool,
) -> enzui_widgets::secure_field::EnzSecureField<'a, Parent> {
    enz_secure_field(
        value,
        move |text| send(ShareMessage::Secret(which, SecretText::new(text))),
        revealed,
        send(ShareMessage::Reveal(which)),
    )
    .label(label)
    .id(id.into())
    .enabled(enabled)
}

/// Reachable, signed in, ready: each check the test made, in words.
fn test_details(tested: &Tested) -> String {
    let mark = |ok: bool, yes: &'static str, no: &'static str| if ok { yes } else { no };
    [
        mark(
            tested.reachable,
            tr("shell-share-check-reachable"),
            tr("shell-share-check-unreachable"),
        ),
        mark(
            tested.signed_in,
            tr("shell-share-check-signed-in"),
            tr("shell-share-check-not-signed-in"),
        ),
        mark(
            tested.repository == "ready",
            tr("shell-share-check-ready"),
            tr("shell-share-check-not-ready"),
        ),
    ]
    .join(" · ")
}

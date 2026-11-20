use super::{ImagePurpose, Message as Parent, SettingsApp};
use enzui_shell::i18n::tr;
use enzui_shell::{
    settings_catalog::Category,
    user_accounts::{self, Account, Accounts, Action, SecretText},
};
use enzui_widgets::{
    banner::{BannerTone, enz_banner},
    button::{ButtonKind, enz_button},
    secure_field::enz_secure_field,
    text::enz_text,
    text_field::enz_text_field,
};
use iced::{
    Alignment, Element, Length, Task,
    widget::{column, container, row},
};

#[derive(Default)]
pub(super) struct Users {
    pub unlocked: bool,
    data: Option<Accounts>,
    busy: bool,
    last_read: Option<std::time::Instant>,
    error: Option<String>,
    notice: Option<String>,
    pending: Option<Pending>,
    selected: Option<u32>,
    name: Option<String>,
    new: Option<NewUser>,
    password: Option<(u32, PasswordFields)>,
    confirm: Option<u32>,
}
#[derive(Clone)]
enum Pending {
    Create(String),
    Rename,
    Profile,
}
struct PasswordFields {
    value: SecretText,
    confirmation: SecretText,
    revealed: [bool; 2],
}
impl Default for PasswordFields {
    fn default() -> Self {
        Self {
            value: SecretText::new(String::new()),
            confirmation: SecretText::new(String::new()),
            revealed: [false; 2],
        }
    }
}
impl PasswordFields {
    fn valid(&self) -> bool {
        user_accounts::valid_password(self.value.expose_secret()) && self.value == self.confirmation
    }
    fn edit(&mut self, confirmation: bool, value: SecretText) {
        if value.expose_secret().len() <= user_accounts::MAX_PASSWORD_BYTES {
            if confirmation {
                self.confirmation = value;
            } else {
                self.value = value;
            }
        }
    }
}
#[derive(Default)]
struct NewUser {
    name: String,
    display: String,
    admin: bool,
    passwords: PasswordFields,
}
#[derive(Clone, Debug)]
pub(super) enum Message {
    Refresh,
    Loaded(Result<Accounts, String>),
    Unlock,
    Unlocked(Result<(), String>),
    Select(u32),
    Name(String),
    /// Typing in your own profile's name field, whichever account is being managed.
    OwnName(u32, String),
    SaveName,
    Picture(u32),
    Apply(Action),
    New(bool),
    NewName(String),
    NewDisplay(String),
    NewAdmin(bool),
    NewSecret(bool, SecretText),
    NewReveal(bool),
    PasswordDialog(Option<u32>),
    PasswordSecret(bool, SecretText),
    PasswordReveal(bool),
    SavePassword,
    Create,
    Remove(Option<u32>),
    ConfirmRemove,
}
fn msg(value: Message) -> Parent {
    Parent::Users(value)
}
impl SettingsApp {
    pub(super) fn users_leave(&mut self) {
        self.users.unlocked = false;
        self.users.new = None;
        self.users.password = None;
        self.users.last_read = None;
    }
    fn password_fields<'a>(
        &'a self,
        fields: &'a PasswordFields,
        creating: bool,
    ) -> Element<'a, Parent> {
        let changed = move |confirmation: bool, value: String| {
            let secret = SecretText::new(value);
            msg(if creating {
                Message::NewSecret(confirmation, secret)
            } else {
                Message::PasswordSecret(confirmation, secret)
            })
        };
        let reveal = move |confirmation| {
            msg(if creating {
                Message::NewReveal(confirmation)
            } else {
                Message::PasswordReveal(confirmation)
            })
        };
        let prefix = if creating {
            "user-new"
        } else {
            "user-password"
        };
        row![
            container(
                enz_secure_field(
                    fields.value.expose_secret(),
                    move |v| changed(false, v),
                    fields.revealed[0],
                    reveal(false)
                )
                .label(enzui_shell::i18n::current().text("shell-password"))
                .placeholder(enzui_shell::i18n::current().text("shell-at-least-8-characters"))
                .id(format!("{prefix}-secret").into())
                .enabled(!self.users.busy && self.system_writes_allowed() && !self.closing)
                .into_element()
            )
            .width(Length::Fill),
            container(
                enz_secure_field(
                    fields.confirmation.expose_secret(),
                    move |v| changed(true, v),
                    fields.revealed[1],
                    reveal(true)
                )
                .label(enzui_shell::i18n::current().text("shell-confirm-password"))
                .placeholder(enzui_shell::i18n::current().text("shell-enter-the-same-password"))
                .id(format!("{prefix}-confirmation").into())
                .enabled(!self.users.busy && self.system_writes_allowed() && !self.closing)
                .into_element()
            )
            .width(Length::Fill),
        ]
        .spacing(12)
        .into()
    }
    pub(super) fn users_pending(&self) -> bool {
        self.users.pending.is_some()
    }
    fn user_section<'a>(
        &self,
        route: &str,
        content: impl Into<Element<'a, Parent>>,
    ) -> super::sections::SectionCard<'a, Parent> {
        self.section_card_for(route, self.anchor_setting(route, content))
    }
    pub(super) fn users_ready(&self) -> bool {
        self.users.data.is_some() || self.users.error.is_some()
    }
    pub(super) fn read_users(&mut self) -> Task<Parent> {
        if self.page != Category::Users
            || self.users.busy
            || self
                .users
                .last_read
                .is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(30))
        {
            return Task::none();
        }
        self.users.busy = true;
        let demo = self.system_backend().demo;
        let demo_data = demo.then(|| self.users.data.clone().unwrap_or_else(sample));
        Task::perform(
            async move {
                if let Some(data) = demo_data {
                    Ok(data)
                } else {
                    user_accounts::load().await
                }
            },
            |r| msg(Message::Loaded(r)),
        )
    }
    pub(super) fn users_message(&mut self, event: Message) -> Task<Parent> {
        if matches!(
            &event,
            Message::Name(_)
                | Message::OwnName(..)
                | Message::NewName(_)
                | Message::NewDisplay(_)
                | Message::NewSecret(..)
                | Message::PasswordSecret(..)
        ) {
            self.highlighted = None;
            self.reveal_pending = false;
        }
        match event {
            Message::Loaded(result) => {
                let mutation = self.users.pending.is_some();
                if mutation && let Err(error) = &result {
                    self.error = Some(error.clone());
                    self.closing = false;
                }
                self.users.busy = false;
                self.users.last_read = Some(std::time::Instant::now());
                match result {
                    Ok(data) => {
                        if self
                            .users
                            .selected
                            .is_none_or(|uid| !data.users.iter().any(|u| u.uid == uid))
                        {
                            self.users.selected = initial_selection(&data);
                            self.users.name = None;
                            self.users.password = None;
                        }
                        if !data.administrator {
                            self.users.unlocked = false;
                            self.users.new = None;
                            self.users.password = None;
                        }
                        if let Some(action) = self.users.pending.take() {
                            match action {
                                Pending::Create(name) => {
                                    self.users.selected = data
                                        .users
                                        .iter()
                                        .find(|u| u.name == name)
                                        .map(|u| u.uid)
                                        .or(self.users.selected);
                                    self.users.new = None;
                                    self.users.name = None;
                                    self.users.password = None;
                                    self.highlighted = Some("users/profile");
                                    self.reveal_pending = true;
                                }
                                Pending::Rename => self.users.name = None,
                                _ => {}
                            }
                        }
                        self.users.data = Some(data);
                        self.users.error = None;
                    }
                    Err(e) => {
                        self.users.error = Some(e);
                        self.users.notice = None;
                        self.users.pending = None;
                    }
                }
            }
            Message::Refresh => {
                self.users.last_read = None;
                return self.read_users();
            }
            Message::Unlock if !self.users.busy && self.system_writes_allowed() => {
                self.users.busy = true;
                let demo = self.system_backend().demo;
                return Task::perform(
                    async move {
                        if demo {
                            Ok(())
                        } else {
                            user_accounts::unlock().await
                        }
                    },
                    |r| msg(Message::Unlocked(r)),
                );
            }
            Message::Unlocked(result) => {
                self.users.busy = false;
                match result {
                    Ok(()) => {
                        self.users.unlocked = true;
                        self.users.error = None;
                        if self.highlighted == Some("users/new") {
                            self.reveal_pending = true;
                        }
                    }
                    Err(e) => {
                        self.users.error = Some(e);
                        self.users.notice = None;
                        self.users.pending = None;
                    }
                }
            }
            Message::Select(uid) => {
                self.users.selected = Some(uid);
                self.users.new = None;
                self.users.password = None;
                self.users.name = None;
                self.users.confirm = None;
                self.users.notice = None;
            }
            Message::Name(name) => self.users.name = Some(name.chars().take(160).collect()),
            Message::OwnName(uid, name) => {
                if self.users.selected != Some(uid) {
                    self.users.selected = Some(uid);
                    self.users.password = None;
                    self.users.confirm = None;
                }
                self.users.name = Some(name.chars().take(160).collect());
            }
            Message::SaveName => {
                if let (Some(uid), Some(name)) = (self.users.selected, self.users.name.clone()) {
                    return self.users_message(Message::Apply(Action::Rename(uid, name)));
                }
            }
            Message::Picture(uid) => return self.start_image_picker(ImagePurpose::Avatar(uid)),
            Message::Apply(action)
                if !self.users.busy
                    && !self.wallpaper.busy
                    && !self.closing
                    && self.system_writes_allowed() =>
            {
                let Some(data) = &self.users.data else {
                    return Task::none();
                };
                let own = matches!(&action,Action::Rename(uid,_)|Action::Avatar(uid,_) if *uid==data.current);
                if !own && !(data.administrator && self.users.unlocked) {
                    self.users.error = Some(tr("shell-unlock-administrator-settings-first").into());
                    return Task::none();
                }
                self.users.pending = Some(match &action {
                    Action::Create { name, .. } => Pending::Create(name.clone()),
                    Action::Rename(..) => Pending::Rename,
                    _ => Pending::Profile,
                });
                if matches!(&action, Action::Create { .. })
                    && let Some(new) = &mut self.users.new
                {
                    new.passwords = Default::default();
                }
                if matches!(&action, Action::Password(..)) {
                    self.users.password = None;
                }
                self.users.busy = true;
                self.users.error = None;
                let notice = match &action {
                    Action::Create { .. } => {
                        tr("shell-account-created-the-user-can-sign-in-with-the-chosen-password")
                    }
                    Action::Remove(_) => tr("shell-account-removed-home-files-were-kept"),
                    _ => tr("shell-profile-updated"),
                };
                self.users.notice = Some(notice.into());
                let demo = self.system_backend().demo;
                let sample = self.users.data.clone();
                return Task::perform(
                    async move {
                        if demo {
                            simulate(sample.unwrap_or_default(), action)
                        } else {
                            user_accounts::change(action).await
                        }
                    },
                    |r| msg(Message::Loaded(r)),
                );
            }
            Message::New(open) => {
                self.users.new = open.then(NewUser::default);
                self.users.password = None;
                self.users.confirm = None;
                if open {
                    self.highlighted = Some("users/new");
                    self.reveal_pending = true;
                }
            }
            Message::NewSecret(confirm, value) => {
                if let Some(new) = &mut self.users.new {
                    new.passwords.edit(confirm, value);
                }
            }
            Message::NewReveal(confirm) => {
                if let Some(new) = &mut self.users.new {
                    new.passwords.revealed[usize::from(confirm)] ^= true;
                }
            }
            Message::PasswordDialog(uid) => {
                self.users.password = uid.map(|id| (id, PasswordFields::default()));
                self.users.new = None;
                if uid.is_some() {
                    self.highlighted = Some("users/profile");
                    self.reveal_pending = true;
                }
            }
            Message::PasswordSecret(confirm, value) => {
                if let Some((_, fields)) = &mut self.users.password {
                    fields.edit(confirm, value);
                }
            }
            Message::PasswordReveal(confirm) => {
                if let Some((_, fields)) = &mut self.users.password {
                    fields.revealed[usize::from(confirm)] ^= true;
                }
            }
            Message::SavePassword => {
                if let Some((uid, fields)) = &self.users.password
                    && fields.valid()
                {
                    return self.users_message(Message::Apply(Action::Password(
                        *uid,
                        fields.value.clone(),
                    )));
                }
            }
            Message::NewName(name) => {
                if let Some(n) = &mut self.users.new {
                    n.name = name.chars().take(32).collect();
                }
            }
            Message::NewDisplay(name) => {
                if let Some(n) = &mut self.users.new {
                    n.display = name.chars().take(160).collect();
                }
            }
            Message::NewAdmin(on) => {
                if let Some(n) = &mut self.users.new {
                    n.admin = on;
                }
            }
            Message::Create if !self.users.busy => {
                if let Some(n) = &self.users.new
                    && n.passwords.valid()
                {
                    return self.users_message(Message::Apply(Action::Create {
                        name: n.name.clone(),
                        display: n.display.clone(),
                        admin: n.admin,
                        password: n.passwords.value.clone(),
                    }));
                }
            }
            Message::Remove(uid) => self.users.confirm = uid,
            Message::ConfirmRemove => {
                if let Some(uid) = self.users.confirm.take() {
                    return self.users_message(Message::Apply(Action::Remove(uid)));
                }
            }
            _ => {}
        }
        if self.closing
            && !self.users_pending()
            && !self.busy
            && !self.wallpaper.pending()
            && !self.plugins.busy()
        {
            return iced::exit();
        }
        Task::none()
    }
    fn avatar<'a>(&self, account: &Account) -> Element<'a, Parent> {
        account
            .icon
            .as_deref()
            .and_then(|path| enzui_shell::profile_image::avatar(std::path::Path::new(path)))
            .map_or_else(
                || {
                    iced::widget::svg(iced::widget::svg::Handle::from_memory(
                        enzui_shell::lock_fonts::monogram(&account.display).into_bytes(),
                    ))
                    .width(64)
                    .height(64)
                    .into()
                },
                |handle| {
                    iced::widget::image(handle)
                        .width(64)
                        .height(64)
                        .border_radius(32.0)
                        .content_fit(iced::ContentFit::Cover)
                        .into()
                },
            )
    }
    /// Picture, name and the actions that change them, side by side.
    fn identity_editor<'a>(
        &'a self,
        account: &'a Account,
        edit: bool,
        own: bool,
    ) -> Element<'a, Parent> {
        let u = &self.users;
        let drafting = u.selected == Some(account.uid);
        let name = u
            .name
            .as_deref()
            .filter(|_| drafting)
            .unwrap_or(&account.display);
        let uid = account.uid;
        let field = enz_text_field(
            enzui_shell::i18n::current().text("shell-full-name"),
            enzui_shell::i18n::current().text("shell-name"),
            name,
            move |s| {
                msg(if own {
                    Message::OwnName(uid, s)
                } else {
                    Message::Name(s)
                })
            },
        )
        .id(if own {
            "user-full-name"
        } else {
            "user-managed-name"
        }
        .into())
        .enabled(edit)
        .into_element();
        let mut actions = row![
            enz_button(
                tr("shell-change-picture"),
                msg(Message::Picture(account.uid))
            )
            .icon("image")
            .kind(ButtonKind::Secondary)
            .size(enzui_widgets::button::ButtonSize::Sm)
            .enabled(edit && !self.wallpaper.pending())
            .into_element(),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        // Only offered when there is a picture to drop.
        if account.icon.is_some() {
            actions = actions.push(
                enz_button(
                    tr("shell-use-initials"),
                    msg(Message::Apply(Action::Avatar(account.uid, String::new()))),
                )
                .kind(ButtonKind::Plain)
                .size(enzui_widgets::button::ButtonSize::Sm)
                .enabled(edit)
                .into_element(),
            );
        }
        actions = actions.push(iced::widget::Space::new().width(Length::Fill));
        // Save appears once the name is actually changed.
        let changed = drafting
            && u.name
                .as_deref()
                .is_some_and(|draft| draft.trim() != account.display && !draft.trim().is_empty());
        if changed {
            actions = actions.push(
                enz_button(tr("shell-save-name"), msg(Message::SaveName))
                    .size(enzui_widgets::button::ButtonSize::Sm)
                    .id("user-save-name".into())
                    .enabled(edit)
                    .into_element(),
            );
        }
        row![
            container(self.avatar(account)).padding(iced::Padding {
                top: 4.0,
                right: 4.0,
                bottom: 0.0,
                left: 4.0,
            }),
            column![field, actions]
                .spacing(10)
                .width(Length::Fill.max(520.0)),
        ]
        .spacing(16)
        .align_y(Alignment::Start)
        .into()
    }
    fn profile_editor<'a>(
        &'a self,
        data: &'a Accounts,
        account: &'a Account,
        writable: bool,
    ) -> Element<'a, Parent> {
        let mut controls =
            column![self.identity_editor(account, writable && account.local, true)].spacing(12);
        if !account.local {
            controls = controls.push(
                enz_text(tr("shell-this-account-is-managed-by-your-organization")).into_element(),
            );
        }
        // An unlocked administrator may still hand the role on.
        if data.administrator && self.users.unlocked && account.local {
            controls = controls.push(
                super::settings_row(tr("shell-administrator"))
                    .subtitle(tr("shell-can-install-software-and-manage-other-accounts"))
                    .icon("shield-check")
                    .switch(account.admin, move |v| {
                        msg(Message::Apply(Action::Administrator(account.uid, v)))
                    })
                    .enabled(writable && (data.may_disable(account.uid) || !account.admin))
                    .into_element(),
            );
        }
        controls.into()
    }
    fn manage_editor<'a>(
        &'a self,
        data: &'a Accounts,
        account: &'a Account,
        writable: bool,
    ) -> Element<'a, Parent> {
        let u = &self.users;
        let admin = data.administrator && u.unlocked;
        let edit = writable && admin && account.local;
        let mut controls = column![self.identity_editor(account, edit, false)].spacing(12);
        if data.administrator && !u.unlocked && account.local {
            controls = controls.push(
                super::settings_row(tr("shell-unlock-to-manage-account"))
                    .subtitle(tr("shell-unlock-to-manage-account-detail"))
                    .icon("lock-keyhole")
                    .control(
                        enz_button(tr("shell-unlock"), msg(Message::Unlock))
                            .kind(ButtonKind::Secondary)
                            .size(enzui_widgets::button::ButtonSize::Sm)
                            .enabled(writable)
                            .into_element(),
                    )
                    .into_element(),
            );
        }
        if !account.local {
            controls = controls.push(
                enz_text(tr("shell-this-account-is-managed-by-your-organization")).into_element(),
            );
        }
        if admin && account.local {
            let mut rows = column![
                super::settings_row(tr("shell-administrator"))
                    .subtitle(tr("shell-can-install-software-and-manage-other-accounts"))
                    .icon("shield-check")
                    .switch(account.admin, move |v| {
                        msg(Message::Apply(Action::Administrator(account.uid, v)))
                    })
                    .enabled(writable && (data.may_disable(account.uid) || !account.admin))
                    .into_element(),
                super::settings_row(tr("shell-allow-sign-in"))
                    .subtitle(if account.password_mode == 0 {
                        tr("shell-disabling-this-account-keeps-its-files")
                    } else {
                        tr("shell-set-a-regular-password-before-changing-sign-in-availability")
                    })
                    .icon("log-in")
                    .switch(!account.locked, move |v| {
                        msg(Message::Apply(Action::Locked(account.uid, !v)))
                    })
                    .enabled(
                        writable
                            && account.password_mode == 0
                            && (data.may_disable(account.uid) || account.locked),
                    )
                    .into_element(),
            ]
            .spacing(2);
            let editing_password = u
                .password
                .as_ref()
                .is_some_and(|(uid, _)| *uid == account.uid);
            rows = rows.push(
                super::settings_row(tr("shell-password"))
                    .subtitle(tr(
                        "shell-setting-a-password-enables-sign-in-for-this-account",
                    ))
                    .icon("key-round")
                    .control(
                        enz_button(
                            tr("shell-set-password"),
                            msg(Message::PasswordDialog(Some(account.uid))),
                        )
                        .kind(ButtonKind::Secondary)
                        .size(enzui_widgets::button::ButtonSize::Sm)
                        .enabled(writable && !editing_password)
                        .into_element(),
                    )
                    .into_element(),
            );
            controls = controls.push(rows);
            if let Some((_, fields)) = u.password.as_ref().filter(|(uid, _)| *uid == account.uid) {
                controls = controls.push(self.password_fields(fields, false)).push(
                    row![
                        iced::widget::Space::new().width(Length::Fill),
                        enz_button(tr("shell-cancel"), msg(Message::PasswordDialog(None)))
                            .kind(ButtonKind::Secondary)
                            .into_element(),
                        enz_button(tr("shell-save-password"), msg(Message::SavePassword))
                            .enabled(writable && fields.valid())
                            .into_element()
                    ]
                    .spacing(8),
                );
            }
            controls = controls.push(
                super::settings_row(tr("shell-remove-account-2"))
                    .subtitle(tr("shell-remove-account-detail"))
                    .icon("user-x")
                    .control(
                        enz_button(
                            tr("shell-remove-2"),
                            msg(Message::Remove(Some(account.uid))),
                        )
                        .kind(ButtonKind::DestructiveOutlined)
                        .size(enzui_widgets::button::ButtonSize::Sm)
                        .enabled(writable && data.removable(account.uid) && u.confirm.is_none())
                        .into_element(),
                    )
                    .into_element(),
            );
        }
        if u.confirm == Some(account.uid) {
            controls = controls.push(enz_banner(tr("shell-remove-this-account-its-home-files-will-remain-on-disk-this-cannot-be-undone")).tone(BannerTone::Warning).into_element());
            controls = controls.push(
                row![
                    iced::widget::Space::new().width(Length::Fill),
                    enz_button(tr("shell-cancel"), msg(Message::Remove(None)))
                        .kind(ButtonKind::Secondary)
                        .into_element(),
                    enz_button(tr("shell-remove-account-2"), msg(Message::ConfirmRemove))
                        .kind(ButtonKind::Destructive)
                        .enabled(writable && admin && data.removable(account.uid))
                        .into_element()
                ]
                .spacing(8),
            );
        }
        controls = controls.push(row![
            iced::widget::Space::new().width(Length::Fill),
            enz_button(tr("shell-done"), msg(Message::Select(data.current)))
                .kind(ButtonKind::Secondary)
                .size(enzui_widgets::button::ButtonSize::Sm)
                .id("user-manage-done".into())
                .into_element(),
        ]);
        controls.into()
    }
    pub(super) fn users_page(&self) -> Element<'_, Parent> {
        let u = &self.users;
        let mut body = column![].spacing(32);
        if let Some(error) = &u.error {
            body = body.push(enz_banner(error).tone(BannerTone::Danger).into_element());
        } else if let Some(notice) = &u.notice {
            body = body.push(enz_banner(notice).tone(BannerTone::Info).into_element());
        }
        let Some(data) = &u.data else {
            let notice = if u.busy {
                tr("shell-reading-local-accounts")
            } else {
                tr("shell-your-os-needs-accountsservice-to-manage-local-users")
            };
            return body
                .push(self.user_section("users/profile",enz_text(tr("shell-select-an-available-account-to-edit-its-profile")).into_element()).title(tr("shell-profile")).into_element())
                .push(self.user_section("users/accounts",column![enz_text(notice).into_element(),enz_button(enzui_shell::i18n::tr("shell-retry"),msg(Message::Refresh)).enabled(!u.busy).into_element()].spacing(10)).title(tr("shell-accounts")).into_element())
                .push(self.user_section("users/new",enz_text(tr("shell-account-creation-needs-the-system-account-service-and-administrator-authoriz")).into_element()).title(tr("shell-add-user")).into_element()).into();
        };
        let writable =
            self.system_writes_allowed() && !u.busy && !self.wallpaper.busy && !self.closing;
        let admin = data.administrator && u.unlocked;
        let own_account = data.users.iter().find(|a| a.uid == data.current);
        // Your own profile first: your name and picture.
        if let Some(account) = own_account {
            body = body.push(
                self.user_section(
                    "users/profile",
                    self.profile_editor(data, account, writable),
                )
                .title(tr("shell-your-profile"))
                .description(tr("shell-your-profile-description"))
                .into_element(),
            );
        }
        // Everyone on this computer, and adding someone.
        let mut people = column![].spacing(2);
        for account in &data.users {
            let own = account.uid == data.current;
            let subtitle = enzui_shell::tr!(
                "account-summary",
                "name" => account.name.as_str(),
                "role" => if account.admin { tr("shell-administrator") } else { tr("shell-standard") },
                "own" => if own { "yes" } else { "no" },
                "disabled" => if account.locked { "yes" } else { "no" },
            );
            let row = super::settings_row(account.display.clone())
                .subtitle(subtitle)
                .icon(if account.admin {
                    "shield-check"
                } else {
                    "user-round"
                })
                .id_prefix(format!("user-account-{}", account.uid));
            // Your own account is edited in Your profile above.
            let row = if own {
                row
            } else {
                row.action(tr("shell-manage"), msg(Message::Select(account.uid)))
                    .enabled(!u.busy)
            };
            people = people.push(row.into_element());
        }
        if !(admin && u.new.is_some()) {
            let locked = data.administrator && !u.unlocked;
            let mut add = super::settings_row(tr("shell-add-a-person"))
                .subtitle(if admin {
                    tr("shell-each-person-gets-their-own-files-apps-and-desktop")
                } else if data.administrator {
                    tr("shell-unlock-administrator-settings-to-add-another-person")
                } else {
                    tr("shell-only-an-administrator-can-add-accounts")
                })
                .icon("user-plus");
            if data.administrator {
                add = add.control(
                    enz_button(
                        if locked {
                            tr("shell-unlock")
                        } else {
                            tr("shell-add-user-2")
                        },
                        msg(if locked {
                            Message::Unlock
                        } else {
                            Message::New(true)
                        }),
                    )
                    .icon(if locked { "lock-keyhole" } else { "" })
                    .kind(ButtonKind::Secondary)
                    .size(enzui_widgets::button::ButtonSize::Sm)
                    .id("user-add".into())
                    .enabled(writable)
                    .into_element(),
                );
            }
            people = people.push(self.anchor_setting("users/new", add.into_element()));
        }
        body = body.push(
            self.user_section("users/accounts", people)
                .title(tr("shell-accounts"))
                .description(tr("shell-accounts-description"))
                .into_element(),
        );
        if admin && let Some(n) = &u.new {
            body = body.push(
                self.user_section(
                    "users/new",
                    column![
                        enz_text_field(
                            enzui_shell::i18n::current().text("shell-full-name"),
                            enzui_shell::i18n::current().text("shell-alex-morgan"),
                            &n.display,
                            |v| msg(Message::NewDisplay(v))
                        )
                        .id("user-new-display".into())
                        .enabled(writable)
                        .into_element(),
                        enz_text_field(
                            enzui_shell::i18n::current().text("shell-username"),
                            "alex",
                            &n.name,
                            |v| msg(Message::NewName(v))
                        )
                        .description(
                            enzui_shell::i18n::current()
                                .text("shell-lowercase-letters-numbers-hyphens-or-underscores")
                        )
                        .id("user-new-name".into())
                        .enabled(writable)
                        .into_element(),
                        self.password_fields(&n.passwords, true),
                        super::settings_row(enzui_shell::i18n::tr("shell-administrator"))
                            .subtitle(tr("shell-can-install-software-and-manage-other-accounts"))
                            .switch(n.admin, |v| msg(Message::NewAdmin(v)))
                            .enabled(writable)
                            .into_element(),
                        row![
                            iced::widget::Space::new().width(Length::Fill),
                            enz_button(tr("shell-cancel"), msg(Message::New(false)))
                                .kind(ButtonKind::Secondary)
                                .into_element(),
                            enz_button(tr("shell-add-user"), msg(Message::Create))
                                .enabled(
                                    writable
                                        && user_accounts::valid_name(&n.name)
                                        && !n.display.trim().is_empty()
                                        && n.passwords.valid()
                                )
                                .into_element()
                        ]
                        .spacing(8),
                    ]
                    .spacing(12),
                )
                .title(tr("shell-add-a-person"))
                .into_element(),
            );
        }
        // Someone else's account, opened from the list.
        if let Some(account) = data
            .users
            .iter()
            .find(|a| Some(a.uid) == u.selected && a.uid != data.current)
        {
            body = body.push(
                self.user_section("users/manage", self.manage_editor(data, account, writable))
                    .title(&enzui_shell::tr!("shell-manage-account-named", "arg0" => account.display.clone()))
                    .into_element(),
            );
        }
        container(body).width(Length::Fill).into()
    }
}
/// The account a fresh Users page shows: your own, else the first.
fn initial_selection(data: &Accounts) -> Option<u32> {
    data.users
        .iter()
        .find(|account| account.uid == data.current)
        .or_else(|| data.users.first())
        .map(|account| account.uid)
}
fn sample() -> Accounts {
    let user = |uid, name: &str, display: &str, admin| Account {
        uid,
        name: name.into(),
        display: display.into(),
        admin,
        locked: false,
        password_mode: 0,
        icon: None,
        local: true,
    };
    Accounts {
        users: vec![
            user(1000, "alex", tr("shell-alex-morgan"), true),
            user(1001, "sam", tr("shell-sam-rivera"), false),
        ],
        current: 1000,
        sessions: vec![1000],
        administrator: true,
    }
}
fn simulate(mut data: Accounts, action: Action) -> Result<Accounts, String> {
    data.validate_action(&action)?;
    match action {
        Action::Create {
            name,
            display,
            admin,
            ..
        } => {
            let uid = data.users.iter().map(|u| u.uid).max().unwrap_or(1000) + 1;
            data.users.push(Account {
                uid,
                name,
                display,
                admin,
                locked: false,
                password_mode: 0,
                icon: None,
                local: true,
            });
        }
        Action::Remove(uid) => data.users.retain(|a| a.uid != uid),
        action => {
            for a in &mut data.users {
                match &action {
                    Action::Rename(uid, name) if a.uid == *uid => a.display.clone_from(name),
                    Action::Avatar(uid, path) if a.uid == *uid => {
                        a.icon = (!path.is_empty()).then(|| path.clone());
                    }
                    Action::Administrator(uid, v) if a.uid == *uid => a.admin = *v,
                    Action::Locked(uid, v) if a.uid == *uid => a.locked = *v,
                    Action::Password(uid, _) if a.uid == *uid => {
                        a.password_mode = 0;
                        a.locked = false;
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn exits(task: Task<Parent>) -> bool {
        use iced::futures::StreamExt;
        iced::futures::executor::block_on(async {
            let Some(mut stream) = iced_runtime::task::into_stream(task) else {
                return false;
            };
            while let Some(action) = stream.next().await {
                if matches!(action, iced_runtime::Action::Exit) {
                    return true;
                }
            }
            false
        })
    }
    #[test]
    fn settings_users_behaviour() {
        // A fresh Users page opens on your own account, wherever it sorts.
        //
        // Sabotage: select `data.users.first()` again; Sam (uid 999) is shown.
        {
            let mut data = sample();
            data.users.insert(
                0,
                Account {
                    uid: 999,
                    name: "sam".into(),
                    display: "Sam".into(),
                    admin: false,
                    locked: false,
                    password_mode: 0,
                    icon: None,
                    local: true,
                },
            );
            assert_eq!(initial_selection(&data), Some(data.current));
            data.current = 4242;
            assert_eq!(initial_selection(&data), Some(999));
        }
        // Secure drafts are redacted and cleared when leaving users.
        {
            let temp = tempfile::tempdir().unwrap();
            let (mut app, _) = SettingsApp::new(
                enzui_shell::settings::Store::at(temp.path().join("settings.json")).unwrap(),
            );
            app.users.new = Some(NewUser::default());
            let _ = app.users_message(Message::NewSecret(
                false,
                SecretText::new("test-only-private-draft".into()),
            ));
            assert!(
                app.users
                    .new
                    .as_ref()
                    .unwrap()
                    .passwords
                    .value
                    .expose_secret()
                    .contains("private")
            );
            assert!(
                !format!(
                    "{:?}",
                    Message::NewSecret(false, SecretText::new("test-only-private-draft".into()))
                )
                .contains("private")
            );
            app.users_leave();
            assert!(app.users.new.is_none());
            assert!(app.users.password.is_none());
        }
        // Closing waits for account mutations and a failed change preserves drafts.
        {
            let temp = tempfile::tempdir().unwrap();
            let (mut app, _) = SettingsApp::new(
                enzui_shell::settings::Store::at(temp.path().join("settings.json")).unwrap(),
            );
            app.users.pending = Some(Pending::Create("sam".into()));
            app.users.busy = true;
            let close = app.update(Parent::Close);
            assert!(app.closing);
            assert!(!exits(close));
            let complete = app.users_message(Message::Loaded(Ok(sample())));
            assert!(!app.users_pending());
            assert!(exits(complete));
            app.closing = false;
            app.users.pending = Some(Pending::Rename);
            app.users.name = Some("Edited".into());
            app.users.busy = true;
            let _ = app.update(Parent::Close);
            assert!(app.closing);
            let failed = app.users_message(Message::Loaded(Err("Authorization cancelled".into())));
            assert!(!exits(failed));
            assert!(!app.closing);
            assert_eq!(app.users.name.as_deref(), Some("Edited"));
            assert!(app.error.as_deref().unwrap().contains("cancelled"));
        }
    }
}

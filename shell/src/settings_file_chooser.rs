use super::{Message as Parent, SettingsApp};
use enzui_shell::i18n::tr;
use enzui_widgets::file_chooser::{
    self, CancelReason, ChooserOptions, DbusBackend, FallbackBackend, FileFilter, FileRequest,
    FileResult, PendingRequest, PortalBackend, PortalOperation, PortalOutcome, PortalParent,
};
use iced::{Subscription, Task};

#[derive(Default)]
pub(super) struct State {
    backend: Option<FallbackBackend>,
    pending: Option<PendingRequest>,
    generation: u64,
    purpose: Purpose,
}
/// What the file being chosen is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Purpose {
    #[default]
    Vpn,
    SshKey,
    Model,
    Phone,
    // A folder Search looks in, or leaves out when true.
    SearchFolder(bool),
}
impl Purpose {
    fn page(self) -> super::Page {
        match self {
            Self::Vpn => super::Page::Network,
            Self::SshKey => super::Page::Remote,
            Self::Model => super::Page::Agents,
            Self::Phone => super::Page::Phone,
            Self::SearchFolder(_) => super::Page::Search,
        }
    }
}
#[derive(Clone, Debug)]
pub(super) enum Message {
    OpenVpn,
    OpenSshKey,
    OpenModel,
    OpenPhoneFiles,
    OpenSearchFolder(bool),
    Wake,
    Selected(u64, PortalOutcome<FileResult>),
    Fallback(u64, file_chooser::fallback::FallbackEvent),
}
impl State {
    fn cancel(&mut self, reason: CancelReason) {
        self.generation = self.generation.saturating_add(1);
        if let Some(mut pending) = self.pending.take() {
            pending.cancel_handle().cancel(reason);
            if let Some(backend) = self.backend.as_mut() {
                pending.settle(backend);
            }
        }
    }
    pub(super) fn subscription(&self) -> Subscription<Parent> {
        file_chooser::completion_subscription(
            self.generation,
            self.pending.as_ref(),
            self.backend
                .as_ref()
                .map(|backend| backend as &dyn PortalBackend),
        )
        .map(|_| Parent::FileChooser(Message::Wake))
    }
    pub(super) fn view<'a>(&'a self, page: iced::Element<'a, Parent>) -> iced::Element<'a, Parent> {
        match &self.backend {
            Some(backend) => {
                let generation = self.generation;
                backend.view(page, move |event| {
                    Parent::FileChooser(Message::Fallback(generation, event))
                })
            }
            None => page,
        }
    }
}
impl Drop for State {
    fn drop(&mut self) {
        self.cancel(CancelReason::ParentClosed);
    }
}
impl SettingsApp {
    pub(super) fn file_chooser_leave(&mut self) {
        self.file_chooser.cancel(CancelReason::Application);
    }
    pub(super) fn file_chooser_message(&mut self, message: Message) -> Task<Parent> {
        match message {
            Message::Fallback(generation, event) => {
                if generation != self.file_chooser.generation || self.closing {
                    return Task::none();
                }
                if let Some(backend) = self.file_chooser.backend.as_mut() {
                    let (answered, focus) = backend.handle(event);
                    return if answered {
                        Task::batch([focus, self.file_chooser_message(Message::Wake)])
                    } else {
                        focus
                    };
                }
            }
            Message::OpenVpn
            | Message::OpenSshKey
            | Message::OpenModel
            | Message::OpenPhoneFiles
            | Message::OpenSearchFolder(_) => {
                let purpose = match message {
                    Message::OpenSshKey => Purpose::SshKey,
                    Message::OpenModel => Purpose::Model,
                    Message::OpenPhoneFiles => Purpose::Phone,
                    Message::OpenSearchFolder(excluded) => Purpose::SearchFolder(excluded),
                    _ => Purpose::Vpn,
                };
                if self.closing
                    || self.page != purpose.page()
                    || (matches!(purpose, Purpose::Vpn | Purpose::SshKey)
                        && !self.system_writes_allowed())
                    || self.file_chooser.pending.is_some()
                {
                    return Task::none();
                }
                if self.file_chooser.backend.is_none() {
                    let portal = DbusBackend::new()
                        .ok()
                        .map(|backend| Box::new(backend) as Box<dyn PortalBackend>);
                    self.file_chooser.backend = Some(FallbackBackend::new(portal));
                }
                let Some(backend) = self.file_chooser.backend.as_mut() else {
                    self.error = Some(tr("shell-file-chooser-unavailable").into());
                    return Task::none();
                };
                self.file_chooser.generation = self.file_chooser.generation.saturating_add(1);
                self.file_chooser.purpose = purpose;
                let options = if let Purpose::SearchFolder(excluded) = purpose {
                    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
                    let options = ChooserOptions::new().title(if excluded {
                        tr("shell-leave-out-a-folder")
                    } else {
                        tr("shell-add-a-folder")
                    });
                    match home {
                        Some(home) => options.current_folder(home),
                        None => options,
                    }
                } else if purpose == Purpose::Phone {
                    ChooserOptions::new().title(tr("shell-phone-send-files"))
                } else if purpose == Purpose::SshKey {
                    ChooserOptions::new()
                        .title(tr("shell-ssh-select-key-file"))
                        .filter(FileFilter::extensions(
                            format!("{} (.pub)", tr("shell-ssh-select-key-file")),
                            ["pub"],
                        ))
                } else if purpose == Purpose::Model {
                    ChooserOptions::new().title(tr("shell-add-a-model")).filter(
                        FileFilter::extensions(
                            format!("{} (.gguf, .bin)", tr("shell-models-on-this-computer")),
                            ["gguf", "bin"],
                        ),
                    )
                } else {
                    ChooserOptions::new()
                        .title(tr("shell-import-a-vpn-profile"))
                        .filter(FileFilter::extensions(
                            "VPN (.conf, .ovpn, .pcf)",
                            ["conf", "ovpn", "pcf"],
                        ))
                };
                let operation = match purpose {
                    Purpose::Phone => PortalOperation::OpenMultiple,
                    Purpose::SearchFolder(_) => PortalOperation::OpenDirectory,
                    _ => PortalOperation::OpenFile,
                };
                let request = FileRequest::new(operation, PortalParent::None).with_options(options);
                match request.start(backend, |_| true) {
                    Ok(pending) => self.file_chooser.pending = Some(pending),
                    Err(_) => self.error = Some(tr("shell-file-chooser-unavailable").into()),
                }
            }
            Message::Wake => {
                if let (Some(pending), Some(backend)) = (
                    self.file_chooser.pending.as_mut(),
                    self.file_chooser.backend.as_mut(),
                ) {
                    let generation = self.file_chooser.generation;
                    let task = file_chooser::pump(pending, backend, move |outcome| {
                        Parent::FileChooser(Message::Selected(generation, outcome))
                    });
                    if !pending.is_unsettled() {
                        self.file_chooser.pending = None;
                    }
                    return task;
                }
            }
            Message::Selected(generation, outcome) => {
                if generation != self.file_chooser.generation
                    || self.closing
                    || self.page != self.file_chooser.purpose.page()
                {
                    return Task::none();
                }
                if let Some(error) = file_chooser::failure_text(&outcome) {
                    self.error = Some(error);
                    return Task::none();
                }
                match outcome {
                    PortalOutcome::Completed(result) => {
                        let paths = result.local_paths();
                        if self.file_chooser.purpose == Purpose::Phone {
                            return self.phone_message(
                                super::system_pages::phone_link::PhoneMessage::Files(paths),
                            );
                        }
                        if let [path] = paths.as_slice() {
                            match self.file_chooser.purpose {
                                Purpose::SshKey => return self.remote_key_file(path.clone()),
                                Purpose::Model => return self.local_ai_add_model(path.clone()),
                                Purpose::SearchFolder(excluded) => {
                                    let Some(path) = path.to_str() else {
                                        self.error =
                                            Some(tr("shell-file-chooser-local-file").into());
                                        return Task::none();
                                    };
                                    return self.search_settings_message(
                                        super::quick_search_settings::Message::Add(
                                            excluded,
                                            path.to_owned(),
                                        ),
                                    );
                                }
                                Purpose::Vpn | Purpose::Phone => {}
                            }
                            let Some(path) = path.to_str() else {
                                self.error = Some(tr("shell-file-chooser-local-file").into());
                                return Task::none();
                            };
                            return self.system_message(
                                super::system_pages::Message::VpnProfileSelected(path.into()),
                            );
                        }
                        self.error = Some(tr("shell-file-chooser-local-file").into());
                    }
                    PortalOutcome::Cancelled(_) => {}
                    _ => self.error = Some(tr("shell-file-chooser-unavailable").into()),
                }
            }
        }
        Task::none()
    }
}

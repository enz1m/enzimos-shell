//! Accessible lock and greeter controls; the password field never exposes its value.

use crate::ui::{Target, Ui};
use accesskit::{
    Action, ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, Node, NodeId,
    Role, TreeId, TreeInfo, TreeUpdate,
};
use enzui_shell::i18n::{self, tr};
use enzui_shell::tr;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

const ROOT: NodeId = NodeId(1);
const CLOCK: NodeId = NodeId(2);
const NAME: NodeId = NodeId(3);
const FIELD: NodeId = NodeId(4);
const STATUS: NodeId = NodeId(5);

fn node_of(target: Target) -> NodeId {
    match target {
        Target::Field => FIELD,
        Target::PreviousUser => NodeId(10),
        Target::NextUser => NodeId(11),
        Target::Keyboard => NodeId(12),
        Target::Session => NodeId(13),
        Target::Restart => NodeId(14),
        Target::PowerOff => NodeId(15),
        Target::SwitchUser => NodeId(16),
        Target::Sleep => NodeId(17),
        Target::PowerMenu => NodeId(18),
    }
}

#[derive(Clone, PartialEq)]
struct Snapshot {
    greeter: bool,
    clock: String,
    name: String,
    question: String,
    secret: bool,
    status: String,
    focus: Target,
    controls: Vec<(Target, String)>,
}

impl Snapshot {
    fn of(ui: &Ui) -> Self {
        let (question, secret) = ui.question();
        let controls = ui
            .targets()
            .into_iter()
            .filter(|t| *t != Target::Field)
            .map(|target| {
                let label = match target {
                    Target::PreviousUser => tr("shell-previous-account").to_owned(),
                    Target::NextUser => tr("shell-next-account").to_owned(),
                    Target::Keyboard if ui.keyboard => tr("shell-hide-on-screen-keyboard").to_owned(),
                    Target::Keyboard => tr("shell-show-on-screen-keyboard").to_owned(),
                    Target::Session => tr!("shell-session-press-to-choose-the-next-one", "arg0" => ui.session_name().unwrap_or("none").to_owned()),
                    Target::Restart => tr("shell-restart").to_owned(),
                    Target::Sleep => tr("shell-sleep").to_owned(),
                    Target::PowerMenu => if ui.power_menu { tr("shell-cancel-power-options") } else { tr("shell-power-options") }.to_owned(),
                    Target::PowerOff => tr("shell-shut-down-2").to_owned(),
                    Target::SwitchUser => tr("shell-switch-user").to_owned(),
                    Target::Field => String::new(),
                };
                (target, label)
            })
            .collect();
        let now = chrono::Local::now();
        Self {
            greeter: ui.greeter.is_some(),
            clock: [
                ui.clock.show_time.then(|| i18n::time(&now, ui.clock_24h)),
                ui.clock
                    .show_date
                    .then(|| i18n::date(now.date_naive(), true)),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", "),
            name: ui.display_name().to_owned(),
            question,
            secret,
            status: ui.status(),
            focus: ui.target,
            controls,
        }
    }

    fn tree(&self) -> TreeUpdate {
        let mut root = Node::new(Role::Window);
        root.set_label(if self.greeter {
            tr("shell-sign-in")
        } else {
            tr("shell-screen-locked")
        });
        let mut clock = Node::new(Role::Label);
        clock.set_value(self.clock.clone());
        let mut name = Node::new(Role::Label);
        name.set_value(if self.greeter {
            tr!("shell-account", "arg0" => self.name.clone())
        } else {
            tr!("shell-locked-by", "arg0" => self.name.clone())
        });
        // Screen readers speak a change of the focused control's
        // description, not of an unfocused label, so the message is both.
        let described = |node: &mut Node| {
            if !self.status.is_empty() {
                node.set_description(self.status.clone());
            }
        };
        let mut field = Node::new(if self.secret {
            Role::PasswordInput
        } else {
            Role::TextInput
        });
        field.set_label(self.question.clone());
        if self.focus == Target::Field {
            described(&mut field);
        }
        field.add_action(Action::Focus);
        // The message is announced through the focused control's description;
        // a live region as well would make screen readers say it twice.
        let mut status = Node::new(Role::Label);
        status.set_value(self.status.clone());
        let mut children = vec![CLOCK, NAME, FIELD, STATUS];
        let mut nodes = vec![
            (CLOCK, clock),
            (NAME, name),
            (FIELD, field),
            (STATUS, status),
        ];
        for (target, label) in &self.controls {
            let mut button = Node::new(Role::Button);
            button.set_label(label.clone());
            if self.focus == *target {
                described(&mut button);
            }
            button.add_action(Action::Focus);
            button.add_action(Action::Click);
            children.push(node_of(*target));
            nodes.push((node_of(*target), button));
        }
        root.set_children(children);
        nodes.insert(0, (ROOT, root));
        let mut tree = TreeInfo::new(ROOT);
        tree.toolkit_name = Some("enzui lock".into());
        TreeUpdate {
            nodes,
            tree: Some(tree),
            tree_id: TreeId::ROOT,
            focus: node_of(self.focus),
        }
    }
}

struct Activation(Arc<AtomicBool>);
impl ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        // Built on the lock's own thread at its next frame.
        self.0.store(true, Ordering::Release);
        None
    }
}
struct Actions(mpsc::Sender<ActionRequest>);
impl ActionHandler for Actions {
    fn do_action(&mut self, request: ActionRequest) {
        let _ = self.0.send(request);
    }
}
struct Deactivation(Arc<AtomicBool>);
impl DeactivationHandler for Deactivation {
    fn deactivate_accessibility(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

pub(crate) struct Access {
    adapter: accesskit_unix::Adapter,
    requested: Arc<AtomicBool>,
    actions: mpsc::Receiver<ActionRequest>,
    last: Option<Snapshot>,
}

/// What an assistive technology asked for.
pub(crate) enum Request {
    Focus(Target),
    Press(Target),
}

impl Access {
    pub(crate) fn new() -> Self {
        let requested = Arc::new(AtomicBool::new(false));
        let (sender, actions) = mpsc::channel();
        let mut adapter = accesskit_unix::Adapter::new(
            Activation(std::sync::Arc::clone(&requested)),
            Actions(sender),
            Deactivation(std::sync::Arc::clone(&requested)),
        );
        // The lock surface holds the keyboard for as long as it exists.
        adapter.update_window_focus_state(true);
        Self {
            adapter,
            requested,
            actions,
            last: None,
        }
    }

    // Sends the tree when it changed, or when a screen reader asked for it.
    pub(crate) fn update(&mut self, ui: &Ui) {
        let snapshot = Snapshot::of(ui);
        let asked = self.requested.swap(false, Ordering::AcqRel);
        if !asked && self.last.as_ref() == Some(&snapshot) {
            return;
        }
        let tree = snapshot.tree();
        if asked {
            // The window had focus before any screen reader listened; toggling
            // it announces the lock as the active window.
            self.adapter.update_window_focus_state(false);
        }
        self.adapter.update_if_active(|| tree);
        if asked {
            self.adapter.update_window_focus_state(true);
        }
        self.last = Some(snapshot);
    }

    pub(crate) fn requests(&self, ui: &Ui) -> Vec<Request> {
        let targets = ui.targets();
        self.actions
            .try_iter()
            .filter_map(|request| {
                let target = targets
                    .iter()
                    .copied()
                    .find(|t| node_of(*t) == request.target_node)?;
                match request.action {
                    Action::Focus => Some(Request::Focus(target)),
                    Action::Click if target != Target::Field => Some(Request::Press(target)),
                    _ => None,
                }
            })
            .collect()
    }
}

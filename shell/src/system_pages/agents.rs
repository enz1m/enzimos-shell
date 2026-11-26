use super::{Parent, SettingsApp};
use enzui_shell::i18n::tr;
use enzui_shell::system_settings::{Agents, Panel, Reading, Request};
use enzui_shell::tr;
use enzui_widgets::{
    badge::BadgeTone,
    button::{ButtonKind, ButtonSize, enz_button},
};
use iced::{Element, widget::column};

fn when(unix_ms: u64, clock_24h: bool) -> String {
    i64::try_from(unix_ms)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|t| enzui_shell::i18n::local_timestamp(t, clock_24h))
        .unwrap_or_default()
}

// "search_messages" reads as "Search messages"; a namespaced id such as
// "settings.set_system" drops its namespace ("Set system"), which the row's
// app already names.
fn function_label(id: &str) -> String {
    let id = id.rsplit_once('.').map_or(id, |(_, name)| name);
    let words = id.replace(['_', '-'], " ");
    let mut chars = words.trim().chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

// `correlated`: the event is its call's newest, so a dispatched call is
// still running; an uncorrelated one only records that it started.
fn outcome(outcome: &str, correlated: bool) -> (&'static str, BadgeTone) {
    match outcome {
        "completed" => (tr("shell-done"), BadgeTone::Success),
        "dispatched" if correlated => (tr("shell-running"), BadgeTone::Accent),
        "dispatched" => (tr("shell-started"), BadgeTone::Default),
        "permission_required" => (tr("shell-asked"), BadgeTone::Default),
        "permission_denied" => (tr("shell-denied"), BadgeTone::Warning),
        "cancelled" => (tr("shell-cancelled"), BadgeTone::Default),
        "timeout" => (tr("shell-timed-out"), BadgeTone::Warning),
        "app_disconnected" => (tr("shell-app-closed"), BadgeTone::Warning),
        "invalid_arguments" => (tr("shell-invalid-request"), BadgeTone::Error),
        "result_too_large" => (tr("shell-result-too-large"), BadgeTone::Error),
        "invalid_app_result" | "app_error" => (tr("shell-app-error"), BadgeTone::Error),
        _ => (tr("shell-unknown"), BadgeTone::Default),
    }
}

impl SettingsApp {
    pub(super) fn agents_page<'a>(
        &'a self,
        body: iced::widget::Column<'a, Parent>,
    ) -> iced::widget::Column<'a, Parent> {
        let agents = match self.reading(Panel::Agents) {
            Some(Reading::Agents(a)) => Some(a),
            _ => None,
        };
        let live = self.controls_live();
        let named = |name: &Option<String>, id: &str| {
            name.clone()
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| id.to_owned())
        };
        let unavailable = |route: &'static str,
                           agents: &Option<Agents>|
         -> Option<Element<'a, Parent>> {
            match agents {
                None => Some(self.mark_setting(route, self.loading_row())),
                Some(Agents { available: false, .. }) => Some(self.mark_setting(
                    route,
                    self.info_row("bot", tr("shell-the-mcp-center-is-not-set-up"), tr("shell-agents-can-use-apps-once-the-enzui-mcp-center-runs-for-this-account")),
                )),
                Some(Agents { error: Some(error), .. }) => Some(self.mark_setting(route, self.info_row("triangle-alert", tr("shell-the-mcp-center-did-not-answer"), error.clone()))),
                Some(_) => None,
            }
        };

        // Remembered decisions: what an agent may or may not do without asking.
        let route = "agents/permissions";
        let mut rows = column![].spacing(2);
        if let Some(row) = unavailable(route, &agents) {
            rows = rows.push(row);
        } else if let Some(agents) = &agents {
            if agents.permissions.is_empty() {
                rows = rows.push(self.mark_setting(route, self.info_row("shield-check", tr("shell-no-remembered-decisions"), tr("shell-agents-ask-each-time-until-you-choose-always-allow-or-never-allow"))));
            }
            for (index, permission) in agents.permissions.iter().enumerate() {
                let badge = if permission.allowed {
                    self.status_badge(tr("shell-always-allowed"), BadgeTone::Success)
                } else {
                    self.status_badge(tr("shell-never-allowed"), BadgeTone::Warning)
                };
                let control = iced::widget::row![
                    badge,
                    enzui_widgets::toolbar_icon::enz_toolbar_icon(
                        "x",
                        Self::request(Request::AgentRevokePermission {
                            agent: permission.agent_id.clone(),
                            app: permission.app_id.clone(),
                            function: permission.function_id.clone(),
                        }),
                    )
                    .aria_label(enzui_shell::tr!("shell-a11y-forget-this-decision-for", "arg0" => enzui_shell::i18n::argument(&(permission.function_id))))
                    .tooltip(enzui_shell::i18n::tr("shell-forget"))
                    .id(format!("agent-permission-forget-{index}").into())
                    .enabled(live)
                    .into_element(),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center);
                let row = crate::settings_app::settings_row(format!("{} · {}", named(&permission.app_name, &permission.app_id), function_label(&permission.function_id)))
                    .subtitle(tr!("shell-since", "arg0" => named(&permission.agent_name, &permission.agent_id).clone(), "arg1" => when(permission.updated_unix_ms, self.settings.session.clock_24h).clone()))
                    .icon("shield")
                    .control(control)
                    .into_element();
                rows = rows.push(if index == 0 {
                    self.mark_setting(route, row)
                } else {
                    row
                });
            }
        }
        let mut body = body.push(
            self.section_card_for(route, rows)
                .title(tr("shell-permissions"))
                .description(tr(
                    "shell-choices-agents-remember-forgetting-one-makes-the-agent-ask-again",
                ))
                .into_element(),
        );
        body = body.push(self.assistant_section());
        body = body.push(self.writing_tools_section());
        body = self.local_ai_sections(body);

        // Enrolled identities: the agents and apps allowed to connect.
        let route = "agents/identities";
        let mut rows = column![].spacing(2);
        if let Some(row) = unavailable(route, &agents) {
            rows = rows.push(row);
        } else if let Some(agents) = &agents {
            let enrolled: Vec<_> = agents
                .identities
                .iter()
                .filter(|i| i.kind != "control")
                .collect();
            if enrolled.is_empty() {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "key-round",
                        tr("shell-nothing-enrolled"),
                        tr("shell-enroll-an-agent-or-app-with-enzui-center-enroll-in-a-terminal"),
                    ),
                ));
            }
            for (index, identity) in enrolled.into_iter().enumerate() {
                let control: Element<'a, Parent> = if identity.revoked {
                    self.status_badge(tr("shell-revoked"), BadgeTone::Default)
                } else {
                    enz_button(
                        tr("shell-revoke"),
                        Self::request(Request::AgentRevokeIdentity(identity.id.clone())),
                    )
                    .kind(ButtonKind::Secondary)
                    .size(ButtonSize::Sm)
                    .id(format!("agent-identity-revoke-{index}").into())
                    .enabled(live)
                    .into_element()
                };
                let kind = if identity.kind == "app" {
                    tr("shell-app")
                } else {
                    tr("shell-agent")
                };
                let row = crate::settings_app::settings_row(if identity.display_name.is_empty() { identity.id.clone() } else { identity.display_name.clone() })
                    .subtitle(tr!("shell-enrolled", "arg0" => kind.to_owned(), "arg1" => identity.id.clone(), "arg2" => when(identity.created_unix_ms, self.settings.session.clock_24h).clone()))
                    .icon(if identity.kind == "app" { "app-window" } else { "bot" })
                    .control(control)
                    .into_element();
                rows = rows.push(if index == 0 {
                    self.mark_setting(route, row)
                } else {
                    row
                });
            }
        }
        body = body.push(
            self.section_card_for(route, rows)
                .title(tr("shell-identities"))
                .description(tr("shell-agents-and-apps-enrolled-on-this-computer-a-revoked-identity-can-no-longer-c"))
                .into_element(),
        );

        // What agents did: names and outcomes only, never arguments or results.
        let route = "agents/activity";
        let mut rows = column![].spacing(2);
        if let Some(row) = unavailable(route, &agents) {
            rows = rows.push(row);
        } else if let Some(agents) = &agents {
            if agents.activity.is_empty() {
                rows = rows.push(self.mark_setting(
                    route,
                    self.info_row(
                        "history",
                        tr("shell-no-activity"),
                        tr("shell-requests-agents-make-appear-here"),
                    ),
                ));
            }
            for (index, event) in enzui_shell::agent_center::latest_per_request(&agents.activity)
                .into_iter()
                .take(50)
                .enumerate()
            {
                let (label, tone) = outcome(&event.outcome, event.request_id.is_some());
                let row = crate::settings_app::settings_row(format!(
                    "{} · {}",
                    named(&event.app_name, &event.app_id),
                    function_label(&event.function_id)
                ))
                .subtitle(format!(
                    "{} · {}",
                    named(&event.agent_name, &event.agent_id),
                    when(event.created_unix_ms, self.settings.session.clock_24h)
                ))
                .icon("activity")
                .control(self.status_badge(label, tone))
                .into_element();
                rows = rows.push(if index == 0 {
                    self.mark_setting(route, row)
                } else {
                    row
                });
            }
            if !agents.activity.is_empty() {
                rows = rows.push(
                    crate::settings_app::settings_row(tr("shell-clear-activity"))
                        .subtitle(tr("shell-clear-activity-detail"))
                        .icon("trash-2")
                        .control(
                            enz_button(
                                tr("shell-clear"),
                                Self::request(Request::AgentClearActivity),
                            )
                            .kind(ButtonKind::Secondary)
                            .size(ButtonSize::Sm)
                            .id("agent-activity-clear".into())
                            .enabled(live)
                            .into_element(),
                        )
                        .into_element(),
                );
            }
        }
        body.push(
            self.section_card_for(route, rows)
                .title(tr("shell-activity"))
                .description(tr("shell-which-agent-used-which-app-function-and-how-it-ended-arguments-and-results-a"))
                .into_element(),
        )
    }
}

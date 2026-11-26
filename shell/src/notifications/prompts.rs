use super::{
    model::{Closed, Key, Model},
    origin::Origin,
    payload::{Action, Draft, Urgency, text},
};
use crate::compositor::Compositor;
use enzui_shell::i18n::tr;
use serde::Deserialize;
use std::{collections::BTreeMap, time::Instant};

pub(super) const SHOW: &str = "show";

#[derive(Deserialize)]
pub(super) struct Dialog {
    id: String,
    class: String,
    title: String,
}

impl Dialog {
    fn into_draft(self) -> Draft {
        let body = [self.title, self.class]
            .into_iter()
            .filter_map(|value| text(&value, 128, false).ok())
            .find(|value| !value.trim().is_empty())
            .unwrap_or_else(|| tr("shell-fullscreen-prompt-fallback").into());
        Draft {
            app_name: "enzimOS".into(),
            desktop_entry: None,
            replaces: 0,
            summary: tr("shell-fullscreen-prompt-title").into(),
            body,
            actions: vec![Action {
                key: SHOW.into(),
                label: tr("shell-fullscreen-prompt-show").into(),
            }],
            reply: None,
            image: None,
            urgency: Urgency::Critical,
            resident: false,
            transient: true,
            timeout_ms: 0,
            value: None,
            saved_file: None,
        }
    }
}

#[derive(Default)]
pub(super) struct Prompts(BTreeMap<u64, Option<Key>>);

impl Prompts {
    pub(super) fn sync(
        &mut self,
        model: &mut Model,
        dialogs: Vec<Dialog>,
        now: Instant,
        wall_ms: u64,
    ) -> Result<Vec<Closed>, String> {
        let dialogs: BTreeMap<_, _> = dialogs
            .into_iter()
            .filter_map(|dialog| Some((u64::from_str_radix(&dialog.id, 16).ok()?, dialog)))
            .collect();
        let mut closed = Vec::new();
        self.0.retain(|id, key| {
            let keep = dialogs.contains_key(id);
            if !keep && let Some(key) = key {
                closed.extend(model.dismiss(*key, wall_ms));
            }
            keep
        });
        for (id, dialog) in dialogs {
            if let std::collections::btree_map::Entry::Vacant(slot) = self.0.entry(id) {
                let posted = model.post(
                    Origin::internal("dialog-password")?,
                    dialog.into_draft(),
                    now,
                    wall_ms,
                    true,
                )?;
                slot.insert(posted.notification.map(|notice| notice.key));
                closed.extend(posted.closed);
            }
        }
        Ok(closed)
    }

    pub(super) fn id_for(&self, key: Key) -> Option<u64> {
        self.0
            .iter()
            .find_map(|(id, stored)| (*stored == Some(key)).then_some(*id))
    }
}

pub(super) async fn reveal(compositor: &Compositor, id: u64) -> Result<(), String> {
    let action = format!("hl.dsp.window.reveal_deferred_dialog({{window='stableid:{id:x}'}})");
    match compositor.request(&["dispatch", &action]).await {
        Ok(reply) if reply.trim() == "ok" => Ok(()),
        _ => Err(tr("shell-notification-action-is-unavailable-or-invalid").into()),
    }
}

#[cfg(test)]
mod tests {
    use super::{super::model::Policy, *};

    fn dialogs() -> Vec<Dialog> {
        serde_json::from_str(
            r#"[{"address":"123","owner":"456","id":"ab","class":"test.Prompt","title":"Unlock \"Work, vault\""}]"#,
        )
        .unwrap()
    }

    #[test]
    fn a_deferred_prompt_bypasses_fullscreen_and_is_removed_on_release() {
        let mut model = Model::new(1, Policy::default()).unwrap();
        let mut prompts = Prompts::default();
        let now = Instant::now();
        prompts.sync(&mut model, dialogs(), now, 1).unwrap();
        let notice = model.active().next().unwrap().clone();
        assert_eq!(notice.draft.body, "Unlock \"Work, vault\"");
        assert!(notice.toast.visible(now));
        assert!(notice.expires.is_none());
        assert!(notice.draft.permits_action(SHOW));
        assert_eq!(prompts.id_for(notice.key), Some(0xab));
        prompts.sync(&mut model, dialogs(), now, 2).unwrap();
        assert_eq!(model.active().next().unwrap().key, notice.key);
        prompts.sync(&mut model, vec![], now, 3).unwrap();
        assert!(model.active().next().is_none());
        assert!(prompts.id_for(notice.key).is_none());
    }
}

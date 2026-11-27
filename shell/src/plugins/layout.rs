use super::manifest::{ElementSpec, Manifest};
use crate::bar_layout::{BarLayout, Section};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Catalog {
    entries: BTreeMap<String, Option<(Vec<ElementSpec>, bool)>>,
}
impl Catalog {
    pub(super) fn insert(&mut self, id: &str, manifest: Option<&Manifest>, granted: bool) {
        self.entries.insert(
            id.to_owned(),
            manifest.map(|m| (m.elements.clone(), granted)),
        );
    }
    pub(super) fn reconcile(&self, current: &BarLayout, reset: bool) -> Result<BarLayout, String> {
        let mut layout = if reset {
            BarLayout::default()
        } else {
            current.clone()
        };
        for section in Section::ALL {
            for item in current.items(section) {
                let Some((plugin, element)) = item
                    .id()
                    .strip_prefix("plugin:")
                    .and_then(|s| s.split_once('/'))
                else {
                    continue;
                };
                match self.entries.get(plugin) {
                    Some(None) if reset => {
                        layout.add_default(item.id(), section, None)?;
                        layout.set_enabled(item.id(), item.enabled())?;
                    }
                    Some(None) => {}
                    Some(Some((elements, _))) if elements.iter().any(|e| e.id == element) => {}
                    _ => {
                        layout.forget(item.id());
                    }
                }
            }
        }
        for (id, entry) in &self.entries {
            if let Some((elements, true)) = entry {
                for element in elements {
                    layout.add_default(
                        &format!("plugin:{id}/{}", element.id),
                        element.section,
                        element.before.as_deref(),
                    )?;
                }
            }
        }
        Ok(layout)
    }
}

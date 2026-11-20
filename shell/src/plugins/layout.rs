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

#[cfg(test)]
mod tests {
    use super::*;
    fn catalog(granted: bool) -> Catalog {
        Catalog {
            entries: BTreeMap::from([(
                "fixture".into(),
                Some((
                    vec![ElementSpec {
                        id: "entry".into(),
                        section: Section::End,
                        before: Some("builtin.controls".into()),
                    }],
                    granted,
                )),
            )]),
        }
    }
    #[test]
    fn layout_behaviour() {
        // Reconcile preserves moves and hidden state but reset restores defaults.
        {
            let defaults = catalog(true)
                .reconcile(&BarLayout::default(), false)
                .unwrap();
            assert_eq!(defaults.items(Section::End)[0].id(), "plugin:fixture/entry");
            let mut placed = defaults.clone();
            placed
                .move_before(&["plugin:fixture/entry"], Section::Start, None)
                .unwrap();
            placed.set_enabled("plugin:fixture/entry", false).unwrap();
            assert_eq!(catalog(true).reconcile(&placed, false).unwrap(), placed);
            assert_eq!(catalog(false).reconcile(&placed, false).unwrap(), placed);
            assert_eq!(catalog(true).reconcile(&placed, true).unwrap(), defaults);
            assert_eq!(
                catalog(false)
                    .reconcile(&BarLayout::default(), false)
                    .unwrap(),
                BarLayout::default()
            );
        }
        // Unavailable packages keep placements but removed packages and elements do not.
        {
            let mut layout = catalog(true)
                .reconcile(&BarLayout::default(), false)
                .unwrap();
            layout
                .add_default("plugin:removed/old", Section::Middle, None)
                .unwrap();
            layout
                .add_default("plugin:fixture/obsolete", Section::Start, None)
                .unwrap();
            layout.set_enabled("plugin:fixture/entry", false).unwrap();
            let mut unavailable = Catalog::default();
            unavailable.insert("fixture", None, true);
            for reset in [false, true] {
                let value = unavailable.reconcile(&layout, reset).unwrap();
                assert!(value.section_of("plugin:removed/old").is_none());
                assert_eq!(
                    value.section_of("plugin:fixture/obsolete"),
                    Some(Section::Start)
                );
                assert!(
                    !value
                        .items(Section::End)
                        .iter()
                        .find(|i| i.id() == "plugin:fixture/entry")
                        .unwrap()
                        .enabled()
                );
            }
            let valid = catalog(true).reconcile(&layout, false).unwrap();
            assert!(valid.section_of("plugin:fixture/obsolete").is_none());
            assert!(valid.section_of("plugin:removed/old").is_none());
            assert!(!valid.items(Section::End)[0].enabled());
            assert_eq!(
                Catalog::default().reconcile(&layout, false).unwrap(),
                BarLayout::default()
            );
        }
        // Pruning happens before capacity admission and overflow does not mutate input.
        {
            let mut layout = BarLayout::default();
            for i in 4..64 {
                layout
                    .add_default(&format!("plugin:old/e{i}"), Section::End, None)
                    .unwrap();
            }
            let mut full = catalog(true);
            full.insert("old", None, true);
            let original = layout.clone();
            assert!(full.reconcile(&layout, false).is_err());
            assert_eq!(layout, original);
            assert_eq!(catalog(true).reconcile(&layout, false).unwrap().len(), 5);
        }
    }
}

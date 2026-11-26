use super::*;
#[test]
fn launch_requests_resolve_routes_pages_and_gated_categories() {
    assert_eq!(
        Category::requested(Some("sizing/frame_width"), false),
        Category::Shell
    );
    assert_eq!(
        Category::requested(Some("sizing/text_size"), false),
        Category::Appearance
    );
    assert_eq!(Category::requested(Some("shell"), false), Category::Shell);
    assert_eq!(
        Category::requested(Some("bar-layout/items"), false),
        Category::Shell
    );
    assert_eq!(Category::requested(Some("plugins"), false), Category::Shell);
    assert_eq!(
        Category::requested(Some("notifications"), false),
        Category::Appearance
    );
    assert_eq!(Category::requested(None, true), Category::Appearance);
    assert_eq!(
        Category::requested(Some("datetime/lock_style"), true),
        Category::Appearance
    );
    assert_eq!(
        Category::requested(Some("datetime/timezone"), true),
        Category::DateTime
    );
    assert_eq!(
        Category::requested(Some("nonsense"), true),
        Category::Appearance
    );
}

#[test]
fn shortcut_search_targets_the_reference_and_individual_settings_rows() {
    let generic = search("shortcut");
    assert!(
        generic
            .iter()
            .any(|entry| entry.route == "keyboard/shortcut-reference")
    );
    assert!(
        generic
            .iter()
            .any(|entry| entry.route == "windows/layout_switch")
    );
    let action = &search("shortcut move window to desktop 6")[0];
    assert_eq!(action.route, "keyboard/shortcut-move-6");
    assert_eq!(find(action.route).unwrap().widget_id(), action.widget_id());
}

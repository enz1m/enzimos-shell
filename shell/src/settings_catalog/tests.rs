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

use iced::widget::image::Handle;
use std::{collections::HashMap, path::Path, sync::Mutex};

pub(crate) fn get(username: Option<&str>, display: &str) -> Handle {
    static CACHE: Mutex<Option<HashMap<String, Handle>>> = Mutex::new(None);
    let key = format!("{}/{display}", username.unwrap_or(""));
    let mut cache = CACHE.lock().expect("avatar cache");
    let cache = cache.get_or_insert_with(HashMap::new);
    if let Some(handle) = cache.get(&key) {
        return handle.clone();
    }
    let picture = username
        .filter(|name| {
            !name.is_empty()
                && !matches!(*name, "." | "..")
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
        .and_then(|name| {
            enzui_shell::profile_image::avatar(
                &Path::new("/var/lib/AccountsService/icons").join(name),
            )
        });
    let handle = if let Some(handle) = picture {
        handle
    } else {
        let svg = enzui_shell::lock_fonts::monogram(display);
        let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default())
            .expect("avatar vector");
        let mut pix = resvg::tiny_skia::Pixmap::new(192, 192).expect("avatar texture");
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(2.0, 2.0),
            &mut pix.as_mut(),
        );
        Handle::from_rgba(192, 192, pix.take())
    };
    if cache.len() >= 64 {
        cache.clear();
    }
    cache.insert(key, handle.clone());
    handle
}

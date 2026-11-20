pub(crate) fn theme() -> (iced::Theme, bool) {
    let store =
        enzui_shell::theme_store::Store::read(&enzui_core::theme_file::ThemePaths::from_env());
    let mut theme = enzui_theme::EnzTheme::dark();
    store
        .document()
        .apply(&mut theme, enzui_theme::EnzThemeMode::Dark);
    (theme.iced_theme(), store.reduced_motion())
}

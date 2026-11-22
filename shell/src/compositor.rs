mod core;
mod desktops;
mod policy;
pub use core::{OutputChange, announce_capture, restart_for_new_output, screen_reader_state};
pub use policy::fitted_toasts;
mod assistant;
mod popups;
pub use assistant::{FRAME_TAB, ISLAND_BUTTON, TriggerLayout, panel_layout, trigger_layout};
mod sidebar;
#[cfg(test)]
mod tests;
mod types;

pub(crate) use policy::popup_layout;
pub use types::{
    Compositor, ControlContent, ControlGeometry, Keyboard, MAX_TOAST_OVERSHOOT, PopupPlacement,
    Snapshot, ToastPlacement,
};

use std::sync::atomic::{AtomicBool, Ordering};
use zbus::{fdo, object_server::SignalEmitter};

pub(super) struct Lockdown(AtomicBool);

impl Lockdown {
    pub(super) fn new() -> Self {
        Self(AtomicBool::new(location_disabled()))
    }

    pub(super) async fn refresh(&self, emitter: &SignalEmitter<'_>) {
        let disabled = location_disabled();
        if self.0.swap(disabled, Ordering::Relaxed) != disabled {
            let _ = self.disable_location_changed(emitter).await;
        }
    }
}

fn location_disabled() -> bool {
    crate::location::load() == crate::location::Mode::Off
}

fn apply_policy(value: bool, current: bool) -> fdo::Result<()> {
    if value == current {
        Ok(())
    } else {
        Err(fdo::Error::PropertyReadOnly(
            "Lockdown is controlled by desktop policy".into(),
        ))
    }
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Lockdown")]
impl Lockdown {
    #[zbus(property, name = "disable-location")]
    fn disable_location(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    #[zbus(property, name = "disable-location")]
    fn set_disable_location(&self, value: bool) -> fdo::Result<()> {
        apply_policy(value, self.disable_location())
    }

    #[zbus(property, name = "disable-printing")]
    fn disable_printing(&self) -> bool {
        false
    }

    #[zbus(property, name = "disable-printing")]
    fn set_disable_printing(&self, value: bool) -> fdo::Result<()> {
        apply_policy(value, self.disable_printing())
    }

    #[zbus(property, name = "disable-save-to-disk")]
    fn disable_save_to_disk(&self) -> bool {
        false
    }

    #[zbus(property, name = "disable-save-to-disk")]
    fn set_disable_save_to_disk(&self, value: bool) -> fdo::Result<()> {
        apply_policy(value, self.disable_save_to_disk())
    }

    #[zbus(property, name = "disable-application-handlers")]
    fn disable_application_handlers(&self) -> bool {
        false
    }

    #[zbus(property, name = "disable-application-handlers")]
    fn set_disable_application_handlers(&self, value: bool) -> fdo::Result<()> {
        apply_policy(value, self.disable_application_handlers())
    }

    #[zbus(property, name = "disable-camera")]
    fn disable_camera(&self) -> bool {
        false
    }

    #[zbus(property, name = "disable-camera")]
    fn set_disable_camera(&self, value: bool) -> fdo::Result<()> {
        apply_policy(value, self.disable_camera())
    }

    #[zbus(property, name = "disable-microphone")]
    fn disable_microphone(&self) -> bool {
        false
    }

    #[zbus(property, name = "disable-microphone")]
    fn set_disable_microphone(&self, value: bool) -> fdo::Result<()> {
        apply_policy(value, self.disable_microphone())
    }

    #[zbus(property, name = "disable-sound-output")]
    fn disable_sound_output(&self) -> bool {
        false
    }

    #[zbus(property, name = "disable-sound-output")]
    fn set_disable_sound_output(&self, value: bool) -> fdo::Result<()> {
        apply_policy(value, self.disable_sound_output())
    }
}

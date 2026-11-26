use wayland_client::{
    Connection, Dispatch, QueueHandle, globals::GlobalList, protocol::wl_surface::WlSurface,
};
#[doc(hidden)]
pub use wayland_protocols::wp::{
    fractional_scale::v1::client::{
        wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        wp_fractional_scale_v1::{Event, WpFractionalScaleV1},
    },
    viewporter::client::{wp_viewport::WpViewport, wp_viewporter::WpViewporter},
};

const DENOMINATOR: u32 = 120;
const MAX_SCALE: u32 = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    Integer(i32),
    Fractional(u32),
}

impl Scale {
    pub fn supported(self) -> bool {
        match self {
            Self::Integer(scale) => (1..=MAX_SCALE as i32).contains(&scale),
            Self::Fractional(scale) => (1..=MAX_SCALE * DENOMINATOR).contains(&scale),
        }
    }

    pub fn physical(self, (width, height): (u32, u32)) -> (u32, u32) {
        let length = |logical: u32| {
            let physical = match self {
                Self::Integer(scale) => u64::from(logical) * u64::from(scale.max(1) as u32),
                // The protocol rounds half pixels away from zero.
                Self::Fractional(scale) => {
                    (u64::from(logical) * u64::from(scale) + u64::from(DENOMINATOR / 2))
                        / u64::from(DENOMINATOR)
                }
            };
            u32::try_from(physical.max(1)).unwrap_or(u32::MAX)
        };
        (length(width), length(height))
    }
}

pub struct ScaleProtocols {
    manager: WpFractionalScaleManagerV1,
    viewporter: WpViewporter,
}

impl ScaleProtocols {
    pub fn bind<D>(globals: &GlobalList, qh: &QueueHandle<D>) -> Option<Self>
    where
        D: Dispatch<WpFractionalScaleManagerV1, ()> + Dispatch<WpViewporter, ()> + 'static,
    {
        Some(Self {
            manager: globals.bind(qh, 1..=1, ()).ok()?,
            viewporter: globals.bind(qh, 1..=1, ()).ok()?,
        })
    }
}

struct Fractional {
    object: WpFractionalScaleV1,
    viewport: WpViewport,
    preferred: Option<u32>,
}

impl Drop for Fractional {
    fn drop(&mut self) {
        self.object.destroy();
        self.viewport.destroy();
    }
}

pub struct SurfaceScale {
    integer: i32,
    fractional: Option<Fractional>,
}

impl SurfaceScale {
    pub fn new<D>(
        integer: i32,
        protocols: Option<&ScaleProtocols>,
        surface: &WlSurface,
        qh: &QueueHandle<D>,
    ) -> Self
    where
        D: Dispatch<WpFractionalScaleV1, WlSurface> + Dispatch<WpViewport, ()> + 'static,
    {
        Self {
            integer,
            fractional: protocols.map(|protocols| Fractional {
                object: protocols
                    .manager
                    .get_fractional_scale(surface, qh, surface.clone()),
                viewport: protocols.viewporter.get_viewport(surface, qh, ()),
                preferred: None,
            }),
        }
    }

    pub fn scale(&self) -> Scale {
        self.fractional
            .as_ref()
            .and_then(|fractional| fractional.preferred)
            .map_or(Scale::Integer(self.integer), Scale::Fractional)
    }

    pub fn set_integer(&mut self, scale: i32) -> bool {
        let previous = self.scale();
        self.integer = scale;
        previous != self.scale()
    }

    pub fn set_preferred(&mut self, scale: u32) -> bool {
        let Some(fractional) = &mut self.fractional else {
            return false;
        };
        let scale = scale.clamp(1, MAX_SCALE * DENOMINATOR);
        fractional.preferred.replace(scale) != Some(scale)
    }

    pub fn apply(&self, surface: &WlSurface, (width, height): (u32, u32)) {
        match self.scale() {
            Scale::Integer(scale) => surface.set_buffer_scale(scale),
            Scale::Fractional(_) => {
                surface.set_buffer_scale(1);
                if let Some(fractional) = &self.fractional {
                    fractional
                        .viewport
                        .set_destination(width as i32, height as i32);
                }
            }
        }
    }
}

pub trait ScaleHandler {
    fn preferred_scale_changed(&mut self, surface: &WlSurface, scale: u32);
}

impl<D> Dispatch<WpFractionalScaleV1, WlSurface, D> for ScaleProtocols
where
    D: ScaleHandler + Dispatch<WpFractionalScaleV1, WlSurface>,
{
    fn event(
        state: &mut D,
        _: &WpFractionalScaleV1,
        event: Event,
        surface: &WlSurface,
        _: &Connection,
        _: &QueueHandle<D>,
    ) {
        if let Event::PreferredScale { scale } = event {
            state.preferred_scale_changed(surface, scale);
        }
    }
}

/// Connects a Wayland event queue to a layer surface's fractional-scale handler.
#[macro_export]
macro_rules! delegate_surface_scale {
    ($state:ty) => {
        wayland_client::delegate_noop!($state: $crate::surface_scale::WpFractionalScaleManagerV1);
        wayland_client::delegate_noop!($state: $crate::surface_scale::WpViewporter);
        wayland_client::delegate_noop!($state: $crate::surface_scale::WpViewport);
        wayland_client::delegate_dispatch!(
            $state: [
                $crate::surface_scale::WpFractionalScaleV1: wayland_client::protocol::wl_surface::WlSurface
            ] => $crate::surface_scale::ScaleProtocols
        );
    };
}

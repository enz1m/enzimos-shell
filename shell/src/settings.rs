mod actions;
mod edges;
mod metrics;
mod model;
mod store;
#[cfg(test)]
mod tests;

pub use actions::QuickAction;
pub use edges::{AssistantCorner, BarEdge, SidebarEdge};
pub use metrics::{Metric, Metrics, TextStep, apply_text_scale};
pub use model::{Change, MAX_ASSISTANT_WIDTH, MIN_ASSISTANT_WIDTH, NotificationState, Settings};
pub use store::Store;

mod geometry;
mod model;
mod providers;
mod query;
#[path = "quick_search_results.rs"]
mod results;
mod support;
#[cfg(test)]
mod tests;
mod update;
#[path = "quick_search_view.rs"]
mod view;

pub(crate) use model::{Message, State};
pub(crate) use support::{message, source_chord};

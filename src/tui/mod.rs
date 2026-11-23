//! Terminal presentation, input, layout, and application state.

mod app;
mod highlight;
mod input;
mod landing;
mod layout;
mod render;
mod state;

pub(crate) use app::run;

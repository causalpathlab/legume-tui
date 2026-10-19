//! What the tools' map viewers share: colour in linear light and sRGB
//! ([`colour`]), the page a map is drawn on ([`theme`]), the viewport
//! between data and pixels ([`viewport`]), and drawing a map: the frame
//! ([`raster`]), its points ([`points`]), text ([`text`]), labels a viewer
//! can move ([`labels`], [`offsets`]) and keys ([`legend`]). The mouse
//! and keys on a map are [`pointer`](mod@pointer); the frame on the terminal is
//! `display`, with feature `map-display`. The screen around a map, styled
//! by a theme, is [`crate::chrome`].

pub mod colour;
#[cfg(feature = "map-display")]
pub mod display;
pub mod labels;
pub mod legend;
pub mod offsets;
pub mod pointer;
pub mod points;
pub mod raster;
pub mod text;
pub mod theme;
pub mod viewport;

#[cfg(test)]
#[path = "tests/scene.rs"]
mod tests;

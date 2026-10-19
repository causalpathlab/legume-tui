//! What the tools' map viewers share: colour in linear light and sRGB
//! ([`colour`]), the page a map is drawn on ([`theme`]), the viewport
//! between data and pixels ([`viewport`]), and drawing a map: the frame
//! ([`raster`]), its points ([`points`]), text ([`text`]), labels a viewer
//! can move ([`labels`], [`offsets`]) and keys ([`legend`]). The mouse
//! and keys on a map are [`pointer`](mod@pointer); the frame on the terminal is
//! `display`, with feature `map-display`. The screen around a map, styled
//! by a theme, is [`crate::chrome`].
//!
//! Feature `style` gives the colours, the theme and the viewport (and
//! [`crate::chrome`]) without the fonts and images that `map` adds for
//! drawing.

pub mod colour;
#[cfg(feature = "map-display")]
pub mod display;
#[cfg(feature = "map-kitty")]
pub mod kitty;
#[cfg(feature = "map")]
pub mod labels;
#[cfg(feature = "map")]
pub mod legend;
#[cfg(feature = "map")]
pub mod offsets;
#[cfg(feature = "map")]
pub mod pointer;
#[cfg(feature = "map")]
pub mod points;
#[cfg(feature = "map")]
pub mod raster;
#[cfg(feature = "map")]
pub mod text;
pub mod theme;
pub mod viewport;

#[cfg(all(test, feature = "map"))]
#[path = "tests/scene.rs"]
mod tests;

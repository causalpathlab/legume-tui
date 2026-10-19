//! What the tools' map viewers share: colour in linear light and sRGB
//! ([`colour`]), the page a map is drawn on ([`theme`]), the viewport
//! between data and pixels ([`viewport`]), and drawing a map: the frame
//! ([`raster`]), its points ([`points`]), text ([`text`]), labels a viewer
//! can move ([`labels`], [`offsets`]) and keys ([`legend`]). The screen
//! around a map, styled by a theme, is [`crate::chrome`].

pub mod colour;
#[cfg(feature = "map-kitty")]
pub mod kitty;
pub mod labels;
pub mod legend;
pub mod offsets;
pub mod points;
pub mod raster;
pub mod text;
pub mod theme;
pub mod viewport;

#[cfg(test)]
#[path = "tests/scene.rs"]
mod tests;

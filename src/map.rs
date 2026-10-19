//! What the tools' map viewers share: colour in linear light and sRGB
//! ([`colour`]), the page a map is drawn on ([`theme`]), and the viewport
//! between data and pixels ([`viewport`]). The screen around a map, styled
//! by a theme, is [`crate::chrome`].

pub mod colour;
pub mod theme;
pub mod viewport;

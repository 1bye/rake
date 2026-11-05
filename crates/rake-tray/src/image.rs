//! Validated image ownership and explicit native rendering choices.

use crate::Error;
use std::sync::Arc;

/// Native rendering intent for an image.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rendering {
    /// Preserve full image color.
    #[default]
    Color,
    /// Ask a capable OS to render the image as a template.
    Template,
}

/// One straight-alpha sRGB RGBA8 raster representation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImageRepresentation {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) rgba: Arc<[u8]>,
}

impl ImageRepresentation {
    /// Validates and owns one raster; each dimension is bounded to 4096 pixels.
    /// # Errors
    /// Rejects zero/oversized dimensions and an incorrectly sized buffer.
    pub fn rgba(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, Error> {
        let expected = width.checked_mul(height).and_then(|n| n.checked_mul(4));

        if width == 0
            || height == 0
            || width > 4096
            || height > 4096
            || expected.map(|n| n as usize) != Some(rgba.len())
        {
            return Err(Error::InvalidInput(
                "image dimensions or RGBA8 byte length are invalid".into(),
            ));
        }

        Ok(Self {
            width,
            height,
            rgba: rgba.into(),
        })
    }

    /// Raster width in physical pixels.
    pub fn width(&self) -> u32 {
        self.width
    }
    /// Raster height in physical pixels.
    pub fn height(&self) -> u32 {
        self.height
    }
    /// Straight-alpha sRGB RGBA8 bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.rgba
    }
}

/// One icon source with optional native name and portable raster fallback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub(crate) representations: Vec<ImageRepresentation>,
    pub(crate) name: Option<String>,
    pub(crate) rendering: Rendering,
}

impl Image {
    /// Creates a portable image from one validated raster.
    pub fn new(image: ImageRepresentation) -> Self {
        Self {
            representations: vec![image],
            name: None,
            rendering: Rendering::Color,
        }
    }

    /// Creates a native-name source; unsupported hosts require a raster fallback.
    /// # Errors
    /// Rejects empty, NUL-containing or excessive names.
    pub fn named(name: impl Into<String>) -> Result<Self, Error> {
        let name = name.into();
        crate::text(&name, "image name", 1024)?;

        if name.is_empty() {
            return Err(Error::InvalidInput("image name is empty".into()));
        }

        Ok(Self {
            representations: vec![],
            name: Some(name),
            rendering: Rendering::Color,
        })
    }

    /// Adds or replaces a raster of the same pixel dimensions; at most 16 are retained.
    /// # Errors
    /// Rejects a seventeenth distinct representation.
    pub fn representation(mut self, image: ImageRepresentation) -> Result<Self, Error> {
        if let Some(old) = self
            .representations
            .iter_mut()
            .find(|r| r.width == image.width && r.height == image.height)
        {
            *old = image;
        } else {
            if self.representations.len() == 16 {
                return Err(Error::InvalidInput(
                    "image exceeds 16 representations".into(),
                ));
            }

            self.representations.push(image);
        }

        Ok(self)
    }

    /// Selects native color or template rendering.
    pub fn rendering(mut self, rendering: Rendering) -> Self {
        self.rendering = rendering;
        self
    }
    /// Returns raster representations without copying their buffers.
    pub fn representations(&self) -> &[ImageRepresentation] {
        &self.representations
    }
    /// Returns the optional platform-native image name.
    pub fn native_name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

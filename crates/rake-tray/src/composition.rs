//! Explicit raster composition; these effects are not native tray overlay promises.
use crate::{Error, Image, ImageRepresentation};

/// Corner used for a composed badge image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    /// Top left.
    TopLeft,
    /// Top right.
    TopRight,
    /// Bottom left.
    BottomLeft,
    /// Bottom right.
    BottomRight,
}
impl Image {
    /// Composes a badge into every raster representation using straight-alpha blending.
    ///
    /// `fraction` is the badge width relative to the base image width. The result
    /// uses full color rendering; a native-only named source needs a raster fallback.
    /// # Errors
    /// Rejects missing rasters, non-finite fractions or fractions outside (0, 1].
    pub fn badge(&self, badge: &Image, corner: Corner, fraction: f32) -> Result<Self, Error> {
        if !fraction.is_finite()
            || fraction <= 0.0
            || fraction > 1.0
            || self.representations.is_empty()
            || badge.representations.is_empty()
        {
            return Err(Error::InvalidInput(
                "badge composition requires rasters and a fraction in (0, 1]".into(),
            ));
        }
        let mut representations = Vec::new();
        for base in &self.representations {
            let width = ((base.width as f32 * fraction).round() as u32).max(1);
            let source = badge
                .representations
                .iter()
                .min_by_key(|rep| rep.width.abs_diff(width))
                .expect("validated nonempty badge");
            let height = (u64::from(width) * u64::from(source.height) / u64::from(source.width))
                .max(1)
                .min(u64::from(base.height)) as u32;
            let left = match corner {
                Corner::TopLeft | Corner::BottomLeft => 0,
                _ => base.width - width,
            };
            let top = match corner {
                Corner::TopLeft | Corner::TopRight => 0,
                _ => base.height - height,
            };
            let mut rgba = base.rgba.to_vec();
            for y in 0..height {
                for x in 0..width {
                    let sample = bilinear(
                        source,
                        (f64::from(x) + 0.5) * f64::from(source.width) / f64::from(width) - 0.5,
                        (f64::from(y) + 0.5) * f64::from(source.height) / f64::from(height) - 0.5,
                    );
                    let index = ((top + y) * base.width + left + x) as usize * 4;
                    blend(&mut rgba[index..index + 4], sample);
                }
            }
            representations.push(ImageRepresentation::rgba(base.width, base.height, rgba)?);
        }
        composed(representations)
    }
    /// Draws an antialiased clockwise progress ring over every raster representation.
    ///
    /// `fraction` is in [0, 1]; `thickness` is a fraction of the smaller dimension
    /// in (0, 0.5]. Ring color is straight-alpha RGBA8. Result rendering is full color.
    /// # Errors
    /// Rejects missing raster fallback, non-finite values and out-of-range parameters.
    pub fn progress(&self, fraction: f32, thickness: f32, color: [u8; 4]) -> Result<Self, Error> {
        if !fraction.is_finite()
            || !(0.0..=1.0).contains(&fraction)
            || !thickness.is_finite()
            || thickness <= 0.0
            || thickness > 0.5
            || self.representations.is_empty()
        {
            return Err(Error::InvalidInput(
                "progress requires rasters, fraction [0, 1] and thickness (0, 0.5]".into(),
            ));
        }
        let mut representations = Vec::new();
        for rep in &self.representations {
            let mut rgba = rep.rgba.to_vec();
            let radius = f64::from(rep.width.min(rep.height)) / 2.0;
            let inner = radius - radius * 2.0 * f64::from(thickness);
            for y in 0..rep.height {
                for x in 0..rep.width {
                    let mut covered = 0;
                    // Four subpixel samples soften both ring and angular boundaries.
                    for (sx, sy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                        let dx = f64::from(x) + sx - f64::from(rep.width) / 2.0;
                        let dy = f64::from(y) + sy - f64::from(rep.height) / 2.0;
                        let distance = dx.hypot(dy);
                        let angle = (dy.atan2(dx) + std::f64::consts::FRAC_PI_2)
                            .rem_euclid(std::f64::consts::TAU);
                        if distance >= inner
                            && distance <= radius
                            && angle < f64::from(fraction) * std::f64::consts::TAU
                        {
                            covered += 1;
                        }
                    }
                    let mut pixel = color;
                    pixel[3] = (u16::from(pixel[3]) * covered / 4) as u8;
                    let index = (y * rep.width + x) as usize * 4;
                    blend(&mut rgba[index..index + 4], pixel);
                }
            }
            representations.push(ImageRepresentation::rgba(rep.width, rep.height, rgba)?);
        }
        composed(representations)
    }
}
fn composed(representations: Vec<ImageRepresentation>) -> Result<Image, Error> {
    let mut images = representations.into_iter();
    let mut image = Image::new(
        images
            .next()
            .ok_or_else(|| Error::InvalidInput("missing raster".into()))?,
    );
    for rep in images {
        image = image.representation(rep)?;
    }
    Ok(image)
}
fn blend(destination: &mut [u8], source: [u8; 4]) {
    let sa = f64::from(source[3]) / 255.0;
    let da = f64::from(destination[3]) / 255.0;
    let alpha = sa + da * (1.0 - sa);
    if alpha == 0.0 {
        destination.fill(0);
        return;
    }
    for channel in 0..3 {
        destination[channel] = ((f64::from(source[channel]) * sa
            + f64::from(destination[channel]) * da * (1.0 - sa))
            / alpha)
            .round() as u8;
    }
    destination[3] = (alpha * 255.0).round() as u8;
}
fn bilinear(rep: &ImageRepresentation, x: f64, y: f64) -> [u8; 4] {
    let x = x.clamp(0.0, f64::from(rep.width - 1));
    let y = y.clamp(0.0, f64::from(rep.height - 1));
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(rep.width - 1);
    let y1 = (y0 + 1).min(rep.height - 1);
    let mut premultiplied = [0.0; 4];
    for (sx, sy, weight) in [
        (x0, y0, (1.0 - x.fract()) * (1.0 - y.fract())),
        (x1, y0, x.fract() * (1.0 - y.fract())),
        (x0, y1, (1.0 - x.fract()) * y.fract()),
        (x1, y1, x.fract() * y.fract()),
    ] {
        let index = (sy * rep.width + sx) as usize * 4;
        let alpha = f64::from(rep.rgba[index + 3]) / 255.0;
        for (channel, value) in premultiplied[..3].iter_mut().enumerate() {
            *value += f64::from(rep.rgba[index + channel]) * alpha * weight;
        }
        premultiplied[3] += alpha * weight;
    }
    let alpha = premultiplied[3];
    if alpha == 0.0 {
        return [0; 4];
    }
    [
        (premultiplied[0] / alpha).round() as u8,
        (premultiplied[1] / alpha).round() as u8,
        (premultiplied[2] / alpha).round() as u8,
        (alpha * 255.0).round() as u8,
    ]
}

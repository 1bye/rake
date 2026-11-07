//! Bounded animation descriptions; the application adapter owns playback.
use crate::{Error, Image};
use std::time::Duration;

/// Validated bounded icon frames and cadence.
#[derive(Clone, Debug)]
pub struct Animation {
    frames: Vec<Image>,
    interval: Duration,
}

impl Animation {
    /// Creates 2–120 frames at 1–20 frames/second, bounded to 64 MiB of raster data.
    /// # Errors
    /// Rejects invalid cadence, frame count, missing rasters or excessive data.
    pub fn new(frames: Vec<Image>, interval: Duration) -> Result<Self, Error> {
        let bytes: usize = frames
            .iter()
            .flat_map(|image| &image.representations)
            .map(|rep| rep.rgba.len())
            .sum();

        if !(2..=120).contains(&frames.len())
            || interval < Duration::from_millis(50)
            || interval > Duration::from_secs(1)
            || bytes > 64 * 1024 * 1024
            || frames.iter().any(|image| image.representations.is_empty())
        {
            return Err(Error::InvalidInput(
                "animation requires 2–120 raster frames, 50–1000 ms cadence and at most 64 MiB"
                    .into(),
            ));
        }

        Ok(Self { frames, interval })
    }

    /// Immutable frames, in playback order.
    pub fn frames(&self) -> &[Image] {
        &self.frames
    }

    /// Minimum time between frames; playback never catches up with a frame burst.
    pub fn interval(&self) -> Duration {
        self.interval
    }
}

//! Uniform and spatially varying Gaussian blur profiles.

use crate::{Point, Rectangle};

/// Approximate Gaussian sigma in logical pixels, evaluated before masking.
///
/// Gradients follow the complete backdrop bounds, not its visible clip.
/// This profile is for live backdrops; the image blur API keeps its scalar radius.
/// Renderers normalize radii to `0..=128` before applying scale factors.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Blur {
    /// The same sigma throughout the content.
    Uniform(f32),
    /// Sigma changes linearly along one axis.
    Linear(Linear),
}

/// The direction in which a blur radius changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Left to right.
    Horizontal,
    /// Top to bottom.
    Vertical,
}

/// A linear radius profile with normalized positions in its reference bounds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Linear {
    /// Direction of increasing position.
    pub direction: Direction,
    /// Sigma before the start of the transition.
    pub start: f32,
    /// Sigma after the end of the transition.
    pub end: f32,
    /// Start and end positions in `0..=1` of the reference bounds.
    pub range: [f32; 2],
}

impl Blur {
    /// Creates a top-to-bottom radius gradient. Radii may increase or decrease.
    pub fn vertical_gradient(start: f32, end: f32) -> Self {
        Self::Linear(Linear {
            direction: Direction::Vertical,
            start,
            end,
            range: [0.0, 1.0],
        })
    }

    /// Creates a left-to-right radius gradient. Radii may increase or decrease.
    pub fn horizontal_gradient(start: f32, end: f32) -> Self {
        Self::Linear(Linear {
            direction: Direction::Horizontal,
            start,
            end,
            range: [0.0, 1.0],
        })
    }

    /// Limits the transition to part of the reference bounds.
    ///
    /// For example, `.range(0.25, 0.75)` keeps the first and last quarters at
    /// the endpoint radii. Uniform profiles are unchanged. Positions clamp to
    /// `0..=1`; an end at or before the start produces a step at the start.
    pub fn range(mut self, start: f32, end: f32) -> Self {
        if let Self::Linear(linear) = &mut self {
            linear.range = [start, end];
        }
        self
    }

    /// Canonicalizes logical radii and positions for rendering and cache keys.
    ///
    /// NaN and nonpositive radii are sharp; positive infinity clamps to 128.
    /// Nonfinite transition positions use zero for the start and one for the end.
    pub fn normalized(self) -> Self {
        let radius = |value: f32| {
            if value.is_nan() || value <= 0.0 {
                0.0
            } else {
                value.min(128.0)
            }
        };
        match self {
            Self::Uniform(value) => Self::Uniform(radius(value)),
            Self::Linear(mut linear) => {
                linear.start = radius(linear.start);
                linear.end = radius(linear.end);
                if linear.start == linear.end {
                    return Self::Uniform(linear.start);
                }
                linear.range[0] = if linear.range[0].is_finite() {
                    linear.range[0].clamp(0.0, 1.0)
                } else {
                    0.0
                };
                linear.range[1] = if linear.range[1].is_finite() {
                    linear.range[1].clamp(linear.range[0], 1.0)
                } else {
                    1.0
                };
                Self::Linear(linear)
            }
        }
    }

    /// Converts an already normalized profile to another pixel scale.
    ///
    /// This does not clamp physical radii back to the logical 128 pixel limit.
    pub fn scaled(self, scale: f32) -> Self {
        if !scale.is_finite() || scale <= 0.0 {
            return Self::default();
        }
        match self {
            Self::Uniform(value) => Self::Uniform(value * scale),
            Self::Linear(mut linear) => {
                linear.start *= scale;
                linear.end *= scale;
                Self::Linear(linear)
            }
        }
    }

    /// Returns the largest radius in this profile, in its current pixel scale.
    pub fn maximum(self) -> f32 {
        match self {
            Self::Uniform(value) => value,
            Self::Linear(linear) => linear.start.max(linear.end),
        }
    }

    /// Evaluates a normalized profile at a point in its reference bounds.
    pub fn radius_at(self, point: Point, bounds: Rectangle) -> f32 {
        match self {
            Self::Uniform(value) => value,
            Self::Linear(linear) => {
                let position = match linear.direction {
                    Direction::Horizontal => (point.x - bounds.x) / bounds.width,
                    Direction::Vertical => (point.y - bounds.y) / bounds.height,
                };
                let [start, end] = linear.range;
                let fraction = if end <= start {
                    if position >= start { 1.0 } else { 0.0 }
                } else {
                    ((position - start) / (end - start)).clamp(0.0, 1.0)
                };
                linear.start + (linear.end - linear.start) * fraction
            }
        }
    }
}

impl Default for Blur {
    fn default() -> Self {
        Self::Uniform(0.0)
    }
}

impl From<f32> for Blur {
    fn from(radius: f32) -> Self {
        Self::Uniform(radius)
    }
}

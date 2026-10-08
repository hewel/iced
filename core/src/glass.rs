//! Experimental optical material and processing quality for live glass surfaces.

use crate::{Color, Vector};

/// An edge lens applied to the filtered scene, followed by tint and lighting.
///
/// Lengths are logical pixels. The surface's rounded contour defines its lens
/// and lighting normals; its foreground content is unaffected. This is a
/// stylized material, not a physical simulation of a volume of glass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Optics {
    /// Maximum inward sampling displacement, normalized to `0..=64`.
    pub refraction: f32,
    /// Width of the optical edge band, normalized to `0..=128`.
    pub depth: f32,
    /// Direction toward the light in surface coordinates. Normalized to unit length.
    pub light: Vector,
    /// White highlight strength, normalized to `0..=1`.
    pub highlight: f32,
    /// Opposite-edge shading strength, normalized to `0..=1`.
    pub shadow: f32,
    /// Color composited over the refracted scene before lighting.
    pub tint: Color,
}

impl Default for Optics {
    fn default() -> Self {
        Self {
            refraction: 0.0,
            depth: 16.0,
            light: Vector::new(
                -std::f32::consts::FRAC_1_SQRT_2,
                -std::f32::consts::FRAC_1_SQRT_2,
            ),
            highlight: 0.0,
            shadow: 0.0,
            tint: Color::TRANSPARENT,
        }
    }
}

impl Optics {
    /// Sanitizes logical parameters before rendering. NaN values become zero;
    /// zero or nonfinite light directions use the default upper-left light.
    pub fn normalized(self) -> Self {
        let bounded = |value: f32, max| {
            if value.is_nan() {
                0.0
            } else {
                value.clamp(0.0, max)
            }
        };
        let length = self.light.x.hypot(self.light.y);
        Self {
            refraction: bounded(self.refraction, 64.0),
            depth: bounded(self.depth, 128.0),
            light: if length.is_finite() && length > 0.0 {
                Vector::new(self.light.x / length, self.light.y / length)
            } else {
                Self::default().light
            },
            highlight: bounded(self.highlight, 1.0),
            shadow: bounded(self.shadow, 1.0),
            tint: Color {
                r: bounded(self.tint.r, 1.0),
                g: bounded(self.tint.g, 1.0),
                b: bounded(self.tint.b, 1.0),
                a: bounded(self.tint.a, 1.0),
            },
        }
    }

    /// Scales normalized lengths without clamping them back to logical limits.
    pub fn scaled(self, scale: f32) -> Self {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            0.0
        };
        Self {
            refraction: self.refraction * scale,
            depth: self.depth * scale,
            ..self
        }
    }

    /// Whether any material operation can change the sampled scene.
    pub fn is_visible(self) -> bool {
        self.tint.a > 0.0
            || (self.depth > 0.0
                && (self.refraction > 0.0 || self.highlight > 0.0 || self.shadow > 0.0))
    }
}

/// Processing resolution for the glass blur layer. Geometry, optical shading,
/// and foreground content remain at the window's resolution.
///
/// Progressive profiles and uniform sigma below two physical pixels retain
/// full resolution to preserve sharp detail. The modes are quality policies,
/// not guarantees of frame time on a particular device.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum Quality {
    /// Process at full resolution.
    #[default]
    Quality,
    /// Process at three quarters of the output resolution on each axis.
    Balanced,
    /// Process at half of the output resolution on each axis.
    Performance,
    /// Choose a resolution from area, radius, and observed input changes.
    /// Hysteresis prevents repeated switching near a threshold.
    Adaptive,
    /// A per-axis processing fraction, clamped to `0.25..=1`. NaN means full resolution.
    Fixed(f32),
}

impl Quality {
    /// Canonicalizes an explicit fraction for cache identity and rendering.
    pub fn normalized(self) -> Self {
        match self {
            Self::Fixed(value) => Self::Fixed(if value.is_nan() {
                1.0
            } else {
                value.clamp(0.25, 1.0)
            }),
            mode => mode,
        }
    }
}

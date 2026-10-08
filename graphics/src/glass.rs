//! Shared glass optics and deterministic quality policy for native renderers.
use crate::core::Point;
use crate::core::glass::{Optics, Quality};
use crate::core::time::{Duration, Instant};
use crate::shape::RoundedRectangle;

/// Returns an absolute sampling point, highlight, and shadow contribution.
/// All geometry and material lengths must already be in physical pixels.
pub fn offset(shape: &RoundedRectangle, point: Point, optics: Optics) -> (Point, f32, f32) {
    if optics.depth <= 0.0 {
        return (point, 0.0, 0.0);
    }
    let q = (1.0 + shape.distance(point) / optics.depth).clamp(0.0, 1.0);
    if q <= 0.0 {
        return (point, 0.0, 0.0);
    }
    let edge = q * q * (3.0 - 2.0 * q);
    // A signed-distance field has discontinuous unit normals where two edges
    // are equally close. A deep lens reaches these interior ridges. Average
    // its slope over the bevel width and retain its magnitude: normalizing a
    // near-zero slope would recreate a hard crease at the surface center.
    let span = (optics.depth * 0.5).max(0.5);
    let dx = shape.distance(Point::new(point.x + span, point.y))
        - shape.distance(Point::new(point.x - span, point.y));
    let dy = shape.distance(Point::new(point.x, point.y + span))
        - shape.distance(Point::new(point.x, point.y - span));
    let denominator = (2.0 * span).max(dx.hypot(dy));
    let normal = [dx / denominator, dy / denominator];
    let light = normal[0] * optics.light.x + normal[1] * optics.light.y;
    (
        Point::new(
            point.x - normal[0] * optics.refraction * edge,
            point.y - normal[1] * optics.refraction * edge,
        ),
        optics.highlight * light.max(0.0) * edge,
        optics.shadow * (-light).max(0.0) * edge,
    )
}

/// Composites a tint over premultiplied RGBA, then lights it without inventing
/// extra opacity. Components and tint must be in the renderer's working color space.
pub fn shade(pixel: [f32; 4], optics: Optics, highlight: f32, shadow: f32) -> [f32; 4] {
    let tint = optics.tint;
    let alpha = pixel[3] + tint.a * (1.0 - pixel[3]);
    let mut result = [0.0, 0.0, 0.0, alpha];
    for (index, color) in [tint.r, tint.g, tint.b].into_iter().enumerate() {
        let value = pixel[index] * (1.0 - tint.a) + color * tint.a;
        result[index] = (value + (alpha - value) * highlight) * (1.0 - shadow);
    }
    result
}

/// Quality history for one effect. Keep instances bounded and separate between
/// effects; source signatures describe their current lower scene, not identity.
#[derive(Debug)]
pub struct QualityState {
    source: Option<u64>,
    last_change: Option<Instant>,
    burst: u8,
    fraction: f32,
}

impl Default for QualityState {
    fn default() -> Self {
        Self {
            source: None,
            last_change: None,
            burst: 0,
            fraction: 1.0,
        }
    }
}

impl QualityState {
    /// Resolves a fraction of physical resolution. `None` denotes an input
    /// whose content cannot be fingerprinted, such as a live custom primitive.
    pub fn resolve(&mut self, mode: Quality, source: Option<u64>, area: f32, sigma: f32) -> f32 {
        self.resolve_at(mode, source, area, sigma, Instant::now())
    }

    fn resolve_at(
        &mut self,
        mode: Quality,
        source: Option<u64>,
        area: f32,
        sigma: f32,
        now: Instant,
    ) -> f32 {
        let changed = source.is_none() || self.last_change.is_none() || source != self.source;
        if changed {
            self.burst = if self.last_change.is_some_and(|last| {
                now.saturating_duration_since(last) <= Duration::from_millis(125)
            }) {
                self.burst.saturating_add(1)
            } else {
                0
            };
            self.last_change = Some(now);
            self.source = source;
        } else if self
            .last_change
            .is_some_and(|last| now.saturating_duration_since(last) > Duration::from_millis(250))
        {
            self.burst = 0;
        }
        if !sigma.is_finite() || sigma < 2.0 {
            return 1.0;
        }
        match mode.normalized() {
            Quality::Quality => 1.0,
            Quality::Balanced => 0.75,
            Quality::Performance => 0.5,
            Quality::Fixed(fraction) => fraction,
            Quality::Adaptive => {
                let area = if area.is_finite() { area.max(0.0) } else { 0.0 };
                let work = area
                    * (sigma / 16.0).clamp(0.25, 4.0)
                    * if self.burst >= 3 { 2.0 } else { 1.0 };
                self.fraction = if self.fraction >= 1.0 {
                    if work > 500_000.0 { 0.75 } else { 1.0 }
                } else if self.fraction >= 0.75 {
                    if work > 1_800_000.0 {
                        0.5
                    } else if work < 300_000.0 {
                        1.0
                    } else {
                        0.75
                    }
                } else if work < 1_100_000.0 {
                    0.75
                } else {
                    0.5
                };
                self.fraction
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_reacts_to_source_changes_but_not_unrelated_redraws() {
        let start = Instant::now();
        let mut stable = QualityState::default();
        let mut changing = QualityState::default();
        for frame in 0..12 {
            let now = start + Duration::from_millis(frame * 16);
            assert_eq!(
                stable.resolve_at(Quality::Adaptive, Some(1), 400_000.0, 16.0, now),
                1.0
            );
            let _ = changing.resolve_at(Quality::Adaptive, Some(frame), 400_000.0, 16.0, now);
        }
        assert!(changing.fraction < stable.fraction);
        // A stationary small surface can recover even if the rest of the UI redraws.
        assert_eq!(
            changing.resolve_at(
                Quality::Adaptive,
                Some(11),
                200_000.0,
                16.0,
                start + Duration::from_secs(1)
            ),
            1.0
        );
    }

    #[test]
    fn adaptive_hysteresis_avoids_alternating_resolutions() {
        let now = Instant::now();
        let mut state = QualityState::default();
        assert_eq!(
            state.resolve_at(Quality::Adaptive, Some(1), 550_000.0, 16.0, now),
            0.75
        );
        for frame in 1..20 {
            let area = if frame % 2 == 0 { 450_000.0 } else { 550_000.0 };
            assert_eq!(
                state.resolve_at(
                    Quality::Adaptive,
                    Some(1),
                    area,
                    16.0,
                    now + Duration::from_millis(frame * 16)
                ),
                0.75
            );
        }
    }
}

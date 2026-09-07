//! Pixel-space geometry shared by the native renderers.
//!
//! Smoothing keeps each corner's radius-sized footprint fixed. Zero smoothing
//! is circular; one smoothing is a fourth-order superellipse (not Figma geometry).

use crate::core::{Point, Rectangle, border::Radius};

/// A finite, positive rectangle with independently rounded, optionally smooth corners.
#[derive(Debug, Clone, Copy)]
pub struct RoundedRectangle {
    bounds: Rectangle,
    radii: [f32; 4],
    smoothing: f32,
    exponent: f32,
}

impl RoundedRectangle {
    /// Constructs a contour in physical pixels, after any desired snapping.
    ///
    /// Invalid or empty bounds are rejected. Radii are independently clamped to
    /// half the smaller dimension; smoothing is normalized to `[0, 1]`.
    pub fn new(bounds: Rectangle, radius: Radius, smoothing: f32) -> Option<Self> {
        if ![
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            bounds.x + bounds.width,
            bounds.y + bounds.height,
        ]
        .into_iter()
        .all(f32::is_finite)
            || bounds.width <= 0.0
            || bounds.height <= 0.0
        {
            return None;
        }
        let cap = bounds.width.min(bounds.height) * 0.5;
        let radii = [
            radius.top_left,
            radius.top_right,
            radius.bottom_right,
            radius.bottom_left,
        ]
        .map(|r| normalize_length(r).min(cap));
        let smoothing = normalize_smoothing(smoothing);
        Some(Self {
            bounds,
            radii,
            smoothing,
            exponent: 2.0 + 2.0 * smoothing,
        })
    }

    /// Returns signed Euclidean distance to the complete boundary in pixels.
    /// Negative distances are inside. The query point must be finite.
    pub fn distance(&self, point: Point) -> f32 {
        let p = [point.x - self.bounds.x, point.y - self.bounds.y];
        let w = self.bounds.width;
        let h = self.bounds.height;
        let r = self.radii;
        if r == [0.0; 4] || (self.smoothing == 0.0 && r == [r[0]; 4]) {
            let q = [
                (p[0] - w * 0.5).abs() - w * 0.5 + r[0],
                (p[1] - h * 0.5).abs() - h * 0.5 + r[0],
            ];
            return q[0].max(0.0).hypot(q[1].max(0.0)) + q[0].max(q[1]).min(0.0) - r[0];
        }

        let mut distance = segment_distance(p, [r[0], 0.0], [w - r[1], 0.0])
            .min(segment_distance(p, [w, r[1]], [w, h - r[2]]))
            .min(segment_distance(p, [r[3], h], [w - r[2], h]))
            .min(segment_distance(p, [0.0, r[0]], [0.0, h - r[3]]));
        let corners = [
            [r[0] - p[0], r[0] - p[1]],
            [p[0] - (w - r[1]), r[1] - p[1]],
            [p[0] - (w - r[2]), p[1] - (h - r[2])],
            [r[3] - p[0], p[1] - (h - r[3])],
        ];
        let mut inside = p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= w && p[1] <= h;
        for (local, radius) in corners.into_iter().zip(r) {
            if radius > 0.0 && local[0] > 0.0 && local[1] > 0.0 {
                inside &= (local[0] / radius).powf(self.exponent)
                    + (local[1] / radius).powf(self.exponent)
                    <= 1.0;
            }
            // The entire quarter arc lies in this box. Sign classification must
            // happen even when its boundary cannot improve the nearest distance.
            let lower = (local[0] - local[0].clamp(0.0, radius))
                .hypot(local[1] - local[1].clamp(0.0, radius));
            if lower < distance {
                distance = distance.min(arc_distance(local, radius, self.exponent));
            }
        }
        if inside { -distance } else { distance }
    }
}

/// Converts signed pixel distance to one-pixel antialiased coverage.
pub fn coverage(distance: f32) -> f32 {
    (0.5 - distance).clamp(0.0, 1.0)
}

/// Snaps both edges of a physical rectangle, not its size or expanded envelope.
/// Empty snapped rectangles must be discarded by the caller.
pub fn snap(bounds: Rectangle, enabled: bool) -> Rectangle {
    if !enabled {
        return bounds;
    }
    let x = (bounds.x + 0.001).round();
    let y = (bounds.y + 0.001).round();
    Rectangle {
        x,
        y,
        width: (bounds.x + bounds.width + 0.001).round() - x,
        height: (bounds.y + bounds.height + 0.001).round() - y,
    }
}

/// Normalizes a length without prematurely applying a dimension-dependent cap.
/// NaN and negative values become zero; positive infinity becomes `f32::MAX`.
pub fn normalize_length(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, f32::MAX)
    }
}

/// Normalizes smoothing to `[0, 1]`, treating NaN as zero.
pub fn normalize_smoothing(value: f32) -> f32 {
    if value.is_nan() {
        0.0
    } else {
        value.clamp(0.0, 1.0)
    }
}

fn segment_distance(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    // All boundary segments are axis-aligned, including degenerate endpoints.
    (p[0] - p[0].clamp(a[0].min(b[0]), a[0].max(b[0])))
        .hypot(p[1] - p[1].clamp(a[1].min(b[1]), a[1].max(b[1])))
}

fn arc_distance(p: [f32; 2], r: f32, n: f32) -> f32 {
    let endpoints = (p[0] - r).hypot(p[1]).min(p[0].hypot(p[1] - r));
    if r == 0.0 || p[0] < 0.0 || p[1] < 0.0 {
        return endpoints;
    }
    if n == 2.0 {
        return (p[0].hypot(p[1]) - r).abs();
    }
    let a = p[0].max(p[1]) / r;
    let b = p[0].min(p[1]) / r;
    let diagonal = r * 2.0_f32.powf(-1.0 / n);
    let mut best = endpoints.min((p[0] - diagonal).hypot(p[1] - diagonal));
    let mut lo = 0.0;
    let mut hi = 1.0;
    for _ in 0..24 {
        if r * (hi - lo) <= 1.0 / 512.0 {
            break;
        }
        let t = (lo + hi) * 0.5;
        if t == lo || t == hi {
            break;
        }
        let power = t.powf(n - 1.0);
        let x = (1.0 + t.powf(n)).powf(-1.0 / n);
        if x * (t - power) + a * power < b {
            lo = t;
        } else {
            hi = t;
        }
    }
    let t = (lo + hi) * 0.5;
    let x = r * (1.0 + t.powf(n)).powf(-1.0 / n);
    best = best.min((p[0].max(p[1]) - x).hypot(p[0].min(p[1]) - t * x));
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORACLE_ERROR: f64 = 1.0 / 4096.0;

    fn norm(a: [f64; 2], b: [f64; 2]) -> f64 {
        (a[0] - b[0]).hypot(a[1] - b[1])
    }

    fn curve(angle: f64, r: f64, n: f64) -> [f64; 2] {
        if angle == 0.0 {
            return [r, 0.0];
        }
        if angle == std::f64::consts::FRAC_PI_2 {
            return [0.0, r];
        }
        [r * angle.cos().powf(2.0 / n), r * angle.sin().powf(2.0 / n)]
    }

    // Independent angular branch-and-bound: a monotone arc interval lies in
    // the AABB of its endpoints. No production root equation is used here.
    fn oracle_arc(p: [f64; 2], r: f64, n: f64) -> f64 {
        let mut best = norm(p, [r, 0.0]).min(norm(p, [0.0, r]));
        if r == 0.0 {
            return best;
        }
        if n == 2.0 {
            if p[0] >= 0.0 && p[1] >= 0.0 {
                best = best.min((p[0].hypot(p[1]) - r).abs());
            }
            return best;
        }
        let middle = std::f64::consts::FRAC_PI_4;
        best = best.min(norm(p, curve(middle, r, n)));
        let mut pending = vec![(0.0, middle), (middle, std::f64::consts::FRAC_PI_2)];
        while let Some((lo, hi)) = pending.pop() {
            let a = curve(lo, r, n);
            let b = curve(hi, r, n);
            let lower = norm(p, [p[0].clamp(b[0], a[0]), p[1].clamp(a[1], b[1])]);
            if best - lower <= ORACLE_ERROR {
                continue;
            }
            let mid = (lo + hi) * 0.5;
            assert!(mid != lo && mid != hi, "oracle exhausted f64 precision");
            best = best.min(norm(p, curve(mid, r, n)));
            pending.push((lo, mid));
            pending.push((mid, hi));
        }
        best
    }

    fn oracle_segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
        let v = [b[0] - a[0], b[1] - a[1]];
        let length_squared = v[0] * v[0] + v[1] * v[1];
        let t = if length_squared == 0.0 {
            0.0
        } else {
            (((p[0] - a[0]) * v[0] + (p[1] - a[1]) * v[1]) / length_squared).clamp(0.0, 1.0)
        };
        norm(p, [a[0] + t * v[0], a[1] + t * v[1]])
    }

    fn oracle(shape: &RoundedRectangle, point: Point) -> f64 {
        let p = [
            f64::from(point.x) - f64::from(shape.bounds.x),
            f64::from(point.y) - f64::from(shape.bounds.y),
        ];
        let w = f64::from(shape.bounds.width);
        let h = f64::from(shape.bounds.height);
        let r = shape.radii.map(f64::from);
        let n = f64::from(shape.exponent);
        let segments = [
            ([r[0], 0.0], [w - r[1], 0.0]),
            ([w, r[1]], [w, h - r[2]]),
            ([w - r[2], h], [r[3], h]),
            ([0.0, h - r[3]], [0.0, r[0]]),
        ];
        let mut nearest = f64::INFINITY;
        for (a, b) in segments {
            nearest = nearest.min(oracle_segment(p, a, b));
        }
        let centers = [
            [r[0], r[0]],
            [w - r[1], r[1]],
            [w - r[2], h - r[2]],
            [r[3], h - r[3]],
        ];
        let directions = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]];
        let mut inside = p[0] >= 0.0 && p[1] >= 0.0 && p[0] <= w && p[1] <= h;
        for i in 0..4 {
            let q = [
                (p[0] - centers[i][0]) * directions[i][0],
                (p[1] - centers[i][1]) * directions[i][1],
            ];
            nearest = nearest.min(oracle_arc(q, r[i], n));
            if r[i] > 0.0 && q[0] > 0.0 && q[1] > 0.0 {
                inside &= q[0].powf(n) + q[1].powf(n) <= r[i].powf(n);
            }
        }
        if inside { -nearest } else { nearest }
    }

    fn compare(shape: &RoundedRectangle, p: Point) {
        let expected = oracle(shape, p);
        let actual = f64::from(shape.distance(p));
        let radius = shape.radii.into_iter().fold(0.0_f32, f32::max);
        let tolerance = (1.0_f64 / 128.0).max(8.0 * f64::from(f32::EPSILON) * f64::from(radius));
        assert!(
            (actual - expected).abs() <= tolerance,
            "{shape:?} at {p:?}: distance {actual}, oracle {expected}, tolerance {tolerance}"
        );
    }

    #[test]
    fn shape_independent_corners_match_branch_bound_oracle() {
        for smoothing in [0.0, 0.01, 0.25, 0.6, 1.0] {
            for radius in [0.0_f32, 0.1, 0.5, 1.0, 4.0, 24.0, 4096.0, f32::MAX] {
                let size = if radius >= 4096.0 { 16384.0 } else { 96.0 };
                for radii in [
                    Radius::from(radius),
                    Radius {
                        top_left: radius,
                        top_right: radius * 0.5,
                        bottom_right: 0.0,
                        bottom_left: radius * 0.75,
                    },
                    Radius::default().top(radius),
                ] {
                    let shape = RoundedRectangle::new(
                        Rectangle {
                            x: -3.25,
                            y: 7.5,
                            width: size * 1.5,
                            height: size,
                        },
                        radii,
                        smoothing,
                    )
                    .unwrap();
                    for x in [-0.1, 0.0, 0.01, 0.125, 0.3, 0.5, 0.75, 0.99, 1.0, 1.1] {
                        for y in [-0.1, 0.0, 0.01, 0.125, 0.3, 0.5, 0.75, 0.99, 1.0, 1.1] {
                            compare(
                                &shape,
                                Point::new(
                                    shape.bounds.x + x * shape.bounds.width,
                                    shape.bounds.y + y * shape.bounds.height,
                                ),
                            );
                        }
                    }
                    // Each independent corner center, arc endpoint, and diagonal.
                    for (i, r) in shape.radii.into_iter().enumerate() {
                        let right = i == 1 || i == 2;
                        let bottom = i >= 2;
                        let cx = if right { shape.bounds.width - r } else { r };
                        let cy = if bottom { shape.bounds.height - r } else { r };
                        let sx = if right { 1.0 } else { -1.0 };
                        let sy = if bottom { 1.0 } else { -1.0 };
                        for q in [
                            [0.0, 0.0],
                            [r, 0.0],
                            [0.0, r],
                            [r * 2.0_f32.powf(-1.0 / shape.exponent); 2],
                        ] {
                            compare(
                                &shape,
                                Point::new(
                                    shape.bounds.x + cx + sx * q[0],
                                    shape.bounds.y + cy + sy * q[1],
                                ),
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn shape_normal_offsets_are_euclidean() {
        for smoothing in [0.0, 0.01, 0.25, 0.6, 1.0] {
            // Keep the largest inward offset below the minimum curvature radius.
            for radius in [8.0, 24.0, 4096.0] {
                let shape = RoundedRectangle::new(
                    Rectangle {
                        x: 0.0,
                        y: 0.0,
                        width: radius * 4.0,
                        height: radius * 3.0,
                    },
                    Radius::from(radius),
                    smoothing,
                )
                .unwrap();
                for angle in [0.1, 0.3, std::f64::consts::FRAC_PI_4, 1.1, 1.4] {
                    let q = curve(angle, f64::from(radius), f64::from(shape.exponent));
                    let normal = [
                        q[0].powf(f64::from(shape.exponent) - 1.0),
                        q[1].powf(f64::from(shape.exponent) - 1.0),
                    ];
                    let length = normal[0].hypot(normal[1]);
                    for offset in [-2.0, -1.0, -0.5, -0.125, 0.125, 0.5, 1.0, 2.0] {
                        let p = Point::new(
                            (f64::from(radius) - q[0] - normal[0] / length * offset) as f32,
                            (f64::from(radius) - q[1] - normal[1] / length * offset) as f32,
                        );
                        compare(&shape, p);
                        let tolerance =
                            (1.0 / 128.0_f64).max(8.0 * f64::from(f32::EPSILON * radius));
                        assert!((f64::from(shape.distance(p)) - offset).abs() <= tolerance);
                    }
                }
            }
        }
    }

    #[test]
    fn shape_normalization_empty_bounds_and_subpixels() {
        for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(
                RoundedRectangle::new(
                    Rectangle {
                        x: 0.0,
                        y: 0.0,
                        width: invalid,
                        height: 1.0
                    },
                    Radius::default(),
                    0.0
                )
                .is_none()
            );
            assert!(
                RoundedRectangle::new(
                    Rectangle {
                        x: 0.0,
                        y: 0.0,
                        width: 1.0,
                        height: invalid
                    },
                    Radius::default(),
                    0.0
                )
                .is_none()
            );
        }
        for invalid in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(
                RoundedRectangle::new(
                    Rectangle {
                        x: invalid,
                        y: 0.0,
                        width: 1.0,
                        height: 1.0
                    },
                    Radius::default(),
                    0.0
                )
                .is_none()
            );
            assert!(
                RoundedRectangle::new(
                    Rectangle {
                        x: 0.0,
                        y: invalid,
                        width: 1.0,
                        height: 1.0
                    },
                    Radius::default(),
                    0.0
                )
                .is_none()
            );
        }
        for value in [f32::NAN, -1.0, f32::NEG_INFINITY] {
            assert_eq!(normalize_length(value), 0.0);
            assert_eq!(normalize_smoothing(value), 0.0);
        }
        assert_eq!(normalize_length(f32::INFINITY), f32::MAX);
        assert_eq!(normalize_smoothing(f32::INFINITY), 1.0);
        for size in [0.0001, 0.1, 0.5, 1.0] {
            let bounds = Rectangle {
                x: 0.0,
                y: 0.0,
                width: size,
                height: size,
            };
            for r in [f32::NAN, f32::NEG_INFINITY, -1.0, f32::INFINITY, f32::MAX] {
                for s in [f32::NAN, f32::NEG_INFINITY, -1.0, f32::INFINITY, 2.0] {
                    let shape = RoundedRectangle::new(bounds, Radius::from(r), s).unwrap();
                    for p in [
                        Point::ORIGIN,
                        Point::new(size * 0.5, size * 0.5),
                        Point::new(size, size),
                        Point::new(size * 1.1, size * 0.3),
                    ] {
                        compare(&shape, p);
                    }
                }
            }
        }
        let bounds = Rectangle {
            x: -0.501,
            y: 0.499,
            width: 0.1,
            height: 0.1,
        };
        assert_eq!(snap(bounds, false), bounds);
        let snapped = snap(bounds, true);
        assert_eq!(snapped.x, -1.0);
        assert_eq!(snapped.y, 1.0);
        assert!(RoundedRectangle::new(snapped, Radius::default(), 0.0).is_none());
        assert_eq!(
            [
                coverage(-1.0),
                coverage(-0.5),
                coverage(0.0),
                coverage(0.5),
                coverage(1.0)
            ],
            [1.0, 1.0, 0.5, 0.0, 0.0]
        );
    }

    #[test]
    fn shape_translation_and_uniform_scaling_preserve_distance() {
        for smoothing in [0.0, 0.01, 0.25, 0.6, 1.0] {
            let bounds = Rectangle {
                x: 0.0,
                y: 0.0,
                width: 96.0,
                height: 64.0,
            };
            let radius = Radius {
                top_left: 24.0,
                top_right: 12.0,
                bottom_right: 0.0,
                bottom_left: 32.0,
            };
            let original = RoundedRectangle::new(bounds, radius, smoothing).unwrap();
            for scale in [0.25, 1.0, 1.25, 1.5, 2.0, 4.0] {
                let transformed = RoundedRectangle::new(
                    Rectangle {
                        x: -17.25,
                        y: 9.5,
                        width: bounds.width * scale,
                        height: bounds.height * scale,
                    },
                    Radius {
                        top_left: 24.0 * scale,
                        top_right: 12.0 * scale,
                        bottom_right: 0.0,
                        bottom_left: 32.0 * scale,
                    },
                    smoothing,
                )
                .unwrap();
                for p in [
                    Point::new(1.0, 1.0),
                    Point::new(12.0, 12.0),
                    Point::new(48.0, 32.0),
                    Point::new(95.5, 63.5),
                    Point::new(-1.0, 24.0),
                ] {
                    let q = Point::new(-17.25 + p.x * scale, 9.5 + p.y * scale);
                    compare(&transformed, q);
                    assert!(
                        (transformed.distance(q) - original.distance(p) * scale).abs()
                            <= 1.0 / 128.0
                    );
                }
            }
        }
    }
}

//! Premultiplied, edge-extended three-box Gaussian approximation.
use crate::core::{Blur, Point, Rectangle};
use crate::graphics::shape::{self, RoundedRectangle};

pub fn filter(pixels: &mut [u32], scratch: &mut Vec<u32>, width: usize, height: usize, sigma: f32) {
    if scratch.capacity() < pixels.len() {
        scratch.reserve_exact(pixels.len() - scratch.len());
    }
    scratch.resize(pixels.len(), 0);
    // Match the variance of three odd-width box filters to the requested sigma.
    let ideal = (4.0 * sigma * sigma + 1.0).sqrt();
    let lower = ((ideal.floor() as usize).saturating_sub(1) / 2 * 2 + 1).max(1);
    let upper = lower + 2;
    let lower_count =
        ((12.0 * sigma * sigma - 3.0 * (lower * lower) as f32 - 12.0 * lower as f32 - 9.0)
            / (-4.0 * lower as f32 - 4.0))
            .round()
            .clamp(0.0, 3.0) as usize;
    for pass in 0..3 {
        let radius = (if pass < lower_count { lower } else { upper }) / 2;
        axis(pixels, scratch, width, height, radius, false);
        axis(scratch, pixels, width, height, radius, true);
    }
}

fn axis(
    source: &[u32],
    target: &mut [u32],
    width: usize,
    height: usize,
    radius: usize,
    vertical: bool,
) {
    let (lines, length, step) = if vertical {
        (width, height, width)
    } else {
        (height, width, 1)
    };
    let count = (radius * 2 + 1) as u64;
    for line in 0..lines {
        let base = if vertical { line } else { line * width };
        let first = source[base].to_ne_bytes();
        let mut sums = first.map(|channel| u64::from(channel) * (radius + 1) as u64);
        for offset in 1..=radius.min(length - 1) {
            let color = source[base + offset * step].to_ne_bytes();
            for channel in 0..4 {
                sums[channel] += u64::from(color[channel]);
            }
        }
        if radius >= length {
            let last = source[base + (length - 1) * step].to_ne_bytes();
            for channel in 0..4 {
                sums[channel] += u64::from(last[channel]) * (radius - length + 1) as u64;
            }
        }
        for position in 0..length {
            target[base + position * step] =
                u32::from_ne_bytes(sums.map(|sum| ((sum + count / 2) / count) as u8));
            let remove = source[base + position.saturating_sub(radius) * step].to_ne_bytes();
            let add = source[base + (position + radius + 1).min(length - 1) * step].to_ne_bytes();
            for channel in 0..4 {
                sums[channel] =
                    sums[channel] + u64::from(add[channel]) - u64::from(remove[channel]);
            }
        }
    }
}

/// Three separable, spatially varying boxes. Prefix sums keep each pass linear
/// in the number of pixels even when radii or DPI are large. The reference frame
/// is independent of this buffer's origin so tiled/partially clipped effects do
/// not restart the gradient.
pub fn progressive_filter(
    pixels: &mut [u32],
    scratch: &mut Vec<u32>,
    width: usize,
    height: usize,
    blur: Blur,
    origin: Point,
    reference: Rectangle,
) {
    if let Blur::Uniform(sigma) = blur {
        filter(pixels, scratch, width, height, sigma);
        return;
    }
    scratch.resize(pixels.len(), 0);
    let mut prefix = vec![[0u64; 4]; width.max(height) + 1];
    // Filter along the gradient first: the second axis then samples results
    // evaluated at the output's own radius coordinate in either orientation.
    let gradient_vertical = matches!(blur, Blur::Linear(linear)
        if linear.direction == crate::core::blur::Direction::Vertical);
    for pass in 0..3 {
        variable_axis(
            pixels,
            scratch,
            width,
            height,
            blur,
            origin,
            reference,
            pass,
            gradient_vertical,
            &mut prefix,
        );
        variable_axis(
            scratch,
            pixels,
            width,
            height,
            blur,
            origin,
            reference,
            pass,
            !gradient_vertical,
            &mut prefix,
        );
    }
}

/// Exact conservative source dependency rectangle for a requested output tile.
/// Walking passes backwards avoids allocating the maximum-radius halo around
/// every tile, especially at the sharp end of a large offscreen image.
#[cfg(feature = "image")]
pub fn source_region(
    blur: Blur,
    region: Rectangle<u32>,
    width: u32,
    height: u32,
) -> Rectangle<u32> {
    let reference = Rectangle::with_size(crate::core::Size::new(width as f32, height as f32));
    let mut left = region.x;
    let mut top = region.y;
    let mut right = region.x + region.width;
    let mut bottom = region.y + region.height;
    let gradient_vertical = matches!(blur, Blur::Linear(linear)
        if linear.direction == crate::core::blur::Direction::Vertical);
    for pass in (0..3).rev() {
        for vertical in [!gradient_vertical, gradient_vertical] {
            let maximum = blur
                .radius_at(Point::new(left as f32 + 0.5, top as f32 + 0.5), reference)
                .max(blur.radius_at(
                    Point::new(right as f32 - 0.5, bottom as f32 - 0.5),
                    reference,
                ));
            let radius = box_radius(maximum, pass) as u32;
            if vertical {
                top = top.saturating_sub(radius);
                bottom = bottom.saturating_add(radius).min(height);
            } else {
                left = left.saturating_sub(radius);
                right = right.saturating_add(radius).min(width);
            }
        }
    }
    Rectangle {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

fn box_radius(sigma: f32, pass: usize) -> usize {
    let ideal = (4.0 * sigma * sigma + 1.0).sqrt();
    let lower = ((ideal.floor() as usize).saturating_sub(1) / 2 * 2 + 1).max(1);
    let upper = lower + 2;
    let lower_count =
        ((12.0 * sigma * sigma - 3.0 * (lower * lower) as f32 - 12.0 * lower as f32 - 9.0)
            / (-4.0 * lower as f32 - 4.0))
            .round()
            .clamp(0.0, 3.0) as usize;
    (if pass < lower_count { lower } else { upper }) / 2
}

#[allow(clippy::too_many_arguments)]
fn variable_axis(
    source: &[u32],
    target: &mut [u32],
    width: usize,
    height: usize,
    blur: Blur,
    origin: Point,
    reference: Rectangle,
    pass: usize,
    vertical: bool,
    prefix: &mut [[u64; 4]],
) {
    let (lines, length, step) = if vertical {
        (width, height, width)
    } else {
        (height, width, 1)
    };
    let along_axis = matches!(blur, Blur::Linear(linear)
        if (linear.direction == crate::core::blur::Direction::Vertical) == vertical);
    let radii: Vec<_> = (0..if along_axis { length } else { lines }).map(|position| {
        let point = if matches!(blur, Blur::Linear(linear) if linear.direction == crate::core::blur::Direction::Vertical) {
            Point::new(origin.x, origin.y + position as f32 + 0.5)
        } else {
            Point::new(origin.x + position as f32 + 0.5, origin.y)
        };
        box_radius(blur.radius_at(point, reference), pass)
    }).collect();
    for line in 0..lines {
        let base = if vertical { line } else { line * width };
        prefix[0] = [0; 4];
        for position in 0..length {
            let color = source[base + position * step].to_ne_bytes();
            prefix[position + 1] = std::array::from_fn(|channel| {
                prefix[position][channel] + u64::from(color[channel])
            });
        }
        let first = source[base].to_ne_bytes();
        let last = source[base + (length - 1) * step].to_ne_bytes();
        for position in 0..length {
            let radius = radii[if along_axis { position } else { line }];
            let left = position.saturating_sub(radius);
            let right = (position + radius + 1).min(length);
            let count = (radius * 2 + 1) as u64;
            let color = std::array::from_fn(|channel| {
                let sum = prefix[right][channel] - prefix[left][channel]
                    + u64::from(first[channel]) * radius.saturating_sub(position) as u64
                    + u64::from(last[channel])
                        * (position + radius + 1).saturating_sub(length) as u64;
                ((sum + count / 2) / count) as u8
            });
            target[base + position * step] = u32::from_ne_bytes(color);
        }
    }
}

#[derive(Debug, Default)]
pub struct Scene {
    entries: Vec<SceneEntry>,
    scratch: Vec<u32>,
    quality: Vec<crate::graphics::glass::QualityState>,
    quality_index: usize,
    pub hits: u64,
    pub misses: u64,
}

#[derive(Debug)]
struct SceneEntry {
    width: u32,
    height: u32,
    output_width: u32,
    output_height: u32,
    resolution: f32,
    blur: Blur,
    reference: Rectangle,
    input: Vec<u32>,
    output: Vec<u32>,
}

impl Scene {
    pub fn begin_frame(&mut self) {
        self.quality_index = 0;
    }

    pub fn finish_frame(&mut self) {
        self.quality.truncate(self.quality_index);
    }

    pub fn retained_bytes(&self) -> usize {
        self.scratch.capacity() * 4
            + self
                .entries
                .iter()
                .map(|entry| (entry.input.capacity() + entry.output.capacity()) * 4)
                .sum::<usize>()
    }

    pub fn apply(
        &mut self,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &tiny_skia::Mask,
        clip_bounds: Rectangle,
        backdrop: crate::core::renderer::Backdrop,
    ) {
        let width = pixels.width();
        let height = pixels.height();
        let reference = backdrop
            .bounds
            .intersection(&Rectangle::with_size(crate::core::Size::new(
                width as f32,
                height as f32,
            )))
            .filter(|_| !backdrop.bounds.width.is_finite() || !backdrop.bounds.height.is_finite())
            .unwrap_or(backdrop.bounds);
        let Some(shape) =
            RoundedRectangle::new(reference, backdrop.border_radius, backdrop.border_smoothing)
        else {
            return;
        };
        let Some([left, top, right, bottom]) =
            crate::engine::pixel_roi(reference.expand(0.5), clip_bounds, width, height)
        else {
            return;
        };
        if self
            .entries
            .iter()
            .any(|entry| entry.width != width || entry.height != height)
        {
            self.entries.clear();
            self.scratch = Vec::new();
        }
        let input: &[u32] = bytemuck::cast_slice(pixels.data_mut());
        if self.quality_index == self.quality.len() {
            self.quality.push(Default::default());
        }
        let source_key =
            matches!(backdrop.quality, crate::core::glass::Quality::Adaptive).then(|| {
                use std::hash::{Hash, Hasher};
                let mut hasher = rustc_hash::FxHasher::default();
                (width, height).hash(&mut hasher);
                input.hash(&mut hasher);
                hasher.finish()
            });
        let resolution = self.quality[self.quality_index].resolve(
            backdrop.quality,
            source_key,
            width as f32 * height as f32,
            backdrop.blur.maximum(),
        );
        self.quality_index += 1;
        // A progressive rendition contains sharp regions; its physical sampling
        // grid remains full resolution, as does a clear refractive material.
        let resolution = if matches!(backdrop.blur, Blur::Uniform(sigma) if sigma >= 2.0) {
            resolution
        } else {
            1.0
        };
        let output_width = (width as f32 * resolution).ceil().max(1.0) as u32;
        let output_height = (height as f32 * resolution).ceil().max(1.0) as u32;
        let found = self.entries.iter().position(|entry| {
            entry.width == width
                && entry.height == height
                && entry.output_width == output_width
                && entry.output_height == output_height
                && entry.resolution == resolution
                && entry.blur == backdrop.blur
                && entry.reference == reference
                && entry.input == input
        });
        let index = if let Some(index) = found {
            self.hits += 1;
            index
        } else {
            self.misses += 1;
            // At most four snapshots and 64 MiB, except one viewport snapshot
            // together with its scratch buffer. Compare the complete lower scene:
            // pixels outside the output mask still contribute through the halo.
            let output_len = output_width as usize * output_height as usize;
            let bytes = (input.len() + output_len * 2) * 4;
            let mut entry =
                if self.entries.len() >= 4 || self.retained_bytes() + bytes > 64 * 1024 * 1024 {
                    if self.entries.is_empty() {
                        None
                    } else {
                        Some(self.entries.remove(0))
                    }
                } else {
                    None
                }
                .unwrap_or_else(|| SceneEntry {
                    width,
                    height,
                    output_width,
                    output_height,
                    resolution,
                    blur: backdrop.blur,
                    reference,
                    input: Vec::new(),
                    output: Vec::new(),
                });
            entry.width = width;
            entry.height = height;
            entry.output_width = output_width;
            entry.output_height = output_height;
            entry.resolution = resolution;
            entry.blur = backdrop.blur;
            entry.reference = reference;
            entry.input.clear();
            entry.input.extend_from_slice(input);
            entry.output.clear();
            if output_width == width && output_height == height {
                entry.output.extend_from_slice(input);
            } else {
                entry.output.resize(output_len, 0);
                downsample(
                    input,
                    width,
                    height,
                    &mut entry.output,
                    output_width,
                    output_height,
                );
            }
            if backdrop.blur.maximum() > 0.0 {
                progressive_filter(
                    &mut entry.output,
                    &mut self.scratch,
                    output_width as usize,
                    output_height as usize,
                    backdrop.blur.scaled(resolution),
                    Point::ORIGIN,
                    reference * resolution,
                );
            }
            let entry_bytes = (entry.input.capacity() + entry.output.capacity()) * 4;
            while !self.entries.is_empty() && self.retained_bytes() + entry_bytes > 64 * 1024 * 1024
            {
                drop(self.entries.remove(0));
            }
            self.entries.push(entry);
            self.entries.len() - 1
        };
        let output = &self.entries[index].output;
        let target: &mut [u32] = bytemuck::cast_slice_mut(pixels.data_mut());
        for y in top..bottom {
            for x in left..right {
                let point = Point::new(x as f32 + 0.5, y as f32 + 0.5);
                let index = y as usize * width as usize + x as usize;
                let coverage = shape::coverage(shape.distance(point))
                    * f32::from(clip_mask.data()[index])
                    / 255.0;
                if coverage == 0.0 {
                    continue;
                }
                let original = target[index].to_ne_bytes();
                let (sample_point, highlight, shadow) = backdrop
                    .optics
                    .map(|optics| crate::graphics::glass::offset(&shape, point, optics))
                    .unwrap_or((point, 0.0, 0.0));
                let sample_point = Point::new(
                    sample_point.x * output_width as f32 / width as f32,
                    sample_point.y * output_height as f32 / height as f32,
                );
                let mut blurred = if backdrop.optics.is_none()
                    && output_width == width
                    && output_height == height
                {
                    output[index].to_ne_bytes().map(f32::from)
                } else {
                    sample(output, output_width, output_height, sample_point)
                        .map(|channel| channel * 255.0)
                };
                if let Some(optics) = backdrop.optics {
                    // Software surface pixels are BGRA; material shading uses RGBA.
                    let color = crate::graphics::glass::shade(
                        [blurred[2], blurred[1], blurred[0], blurred[3]]
                            .map(|channel| channel / 255.0),
                        optics,
                        highlight,
                        shadow,
                    );
                    blurred =
                        [color[2], color[1], color[0], color[3]].map(|channel| channel * 255.0);
                }
                // This is a replacement of the lower scene, including alpha.
                // Source-over would composite a semitransparent background twice.
                target[index] = u32::from_ne_bytes(std::array::from_fn(|channel| {
                    (f32::from(original[channel]) * (1.0 - coverage) + blurred[channel] * coverage)
                        .round() as u8
                }));
            }
        }
    }
}

/// Integrates each output pixel's source footprint before reducing resolution.
/// Point sampling can change the average of repetitive detail before the blur
/// has a chance to remove it. All premultiplied channels use the same coverage;
/// the existing reduced output allocation is the only destination buffer.
fn downsample(
    source: &[u32],
    width: u32,
    height: u32,
    target: &mut [u32],
    target_width: u32,
    target_height: u32,
) {
    let scale_x = f64::from(width) / f64::from(target_width);
    let scale_y = f64::from(height) / f64::from(target_height);
    for y in 0..target_height {
        let top = f64::from(y) * scale_y;
        let bottom = (f64::from(y) + 1.0) * scale_y;
        for x in 0..target_width {
            let left = f64::from(x) * scale_x;
            let right = (f64::from(x) + 1.0) * scale_x;
            let mut sum = [0.0; 4];
            for source_y in top.floor() as u32..(bottom.ceil() as u32).min(height) {
                let coverage_y =
                    bottom.min(f64::from(source_y) + 1.0) - top.max(f64::from(source_y));
                for source_x in left.floor() as u32..(right.ceil() as u32).min(width) {
                    let coverage_x =
                        right.min(f64::from(source_x) + 1.0) - left.max(f64::from(source_x));
                    let coverage = coverage_x * coverage_y;
                    let pixel = source[source_y as usize * width as usize + source_x as usize]
                        .to_ne_bytes();
                    for channel in 0..4 {
                        sum[channel] += f64::from(pixel[channel]) * coverage;
                    }
                }
            }
            target[y as usize * target_width as usize + x as usize] = u32::from_ne_bytes(
                sum.map(|channel| (channel / (scale_x * scale_y)).round() as u8),
            );
        }
    }
}

/// Edge-clamped, premultiplied bilinear sampling. Reads only the immutable
/// lower-scene rendition, even when output and refracted source regions overlap.
fn sample(pixels: &[u32], width: u32, height: u32, point: Point) -> [f32; 4] {
    let x = (point.x - 0.5).clamp(0.0, width.saturating_sub(1) as f32);
    let y = (point.y - 0.5).clamp(0.0, height.saturating_sub(1) as f32);
    let left = x.floor() as u32;
    let top = y.floor() as u32;
    let right = left.saturating_add(1).min(width - 1);
    let bottom = top.saturating_add(1).min(height - 1);
    let tx = x - left as f32;
    let ty = y - top as f32;
    let pixel = |x: u32, y: u32| pixels[y as usize * width as usize + x as usize].to_ne_bytes();
    let a = pixel(left, top);
    let b = pixel(right, top);
    let c = pixel(left, bottom);
    let d = pixel(right, bottom);
    std::array::from_fn(|channel| {
        ((f32::from(a[channel]) * (1.0 - tx) + f32::from(b[channel]) * tx) * (1.0 - ty)
            + (f32::from(c[channel]) * (1.0 - tx) + f32::from(d[channel]) * tx) * ty)
            / 255.0
    })
}

#[cfg(all(test, feature = "image"))]
mod tests {
    use super::*;
    use crate::core::Size;

    #[test]
    fn progressive_filter_is_symmetric_when_content_and_profile_are_transposed() {
        let width = 127;
        let height = 93;
        let source: Vec<_> = (0..width * height)
            .map(|index| {
                let x = index % width;
                let y = index / width;
                let alpha = ((x * 17 + y * 31) % 256) as u8;
                u32::from_ne_bytes([alpha / 2, alpha / 3, alpha, alpha])
            })
            .collect();
        let reference = Rectangle {
            x: -7.0,
            y: 13.0,
            width: 191.0,
            height: 231.0,
        };
        let origin = Point::new(23.0, 41.0);
        for (start, end, range) in [
            (0.0, 17.0, [0.1, 0.85]),
            (17.0, 0.0, [0.1, 0.85]),
            (0.0, 17.0, [0.4, 0.4]),
        ] {
            let mut horizontal = source.clone();
            let mut vertical = vec![0; source.len()];
            for y in 0..height {
                for x in 0..width {
                    vertical[x * height + y] = source[y * width + x];
                }
            }
            progressive_filter(
                &mut horizontal,
                &mut Vec::new(),
                width,
                height,
                Blur::horizontal_gradient(start, end).range(range[0], range[1]),
                origin,
                reference,
            );
            progressive_filter(
                &mut vertical,
                &mut Vec::new(),
                height,
                width,
                Blur::vertical_gradient(start, end).range(range[0], range[1]),
                Point::new(origin.y, origin.x),
                Rectangle {
                    x: reference.y,
                    y: reference.x,
                    width: reference.height,
                    height: reference.width,
                },
            );
            for y in 0..height {
                for x in 0..width {
                    assert_eq!(
                        horizontal[y * width + x],
                        vertical[x * height + y],
                        "directional asymmetry at {x},{y}, radii {start} -> {end}, range {range:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn progressive_tiles_with_halos_match_full_content_at_boundaries() {
        let width = 720usize;
        let height = 360usize;
        let source: Vec<_> = (0..width * height)
            .map(|index| {
                let x = index % width;
                let y = index / width;
                let alpha = ((x * 17 + y * 31) % 256) as u8;
                u32::from_ne_bytes([alpha / 2, alpha / 3, alpha, alpha])
            })
            .collect();
        for blur in [
            Blur::horizontal_gradient(0.0, 16.0).range(0.2, 0.8),
            Blur::vertical_gradient(16.0, 0.0).range(0.1, 0.9),
        ] {
            let reference = Rectangle::with_size(Size::new(width as f32, height as f32));
            let mut full = source.clone();
            progressive_filter(
                &mut full,
                &mut Vec::new(),
                width,
                height,
                blur,
                Point::ORIGIN,
                reference,
            );
            for tile_left in [0usize, 256, 512] {
                let tile_right = (tile_left + 256).min(width);
                let region = source_region(
                    blur,
                    Rectangle {
                        x: tile_left as u32,
                        y: 128,
                        width: (tile_right - tile_left) as u32,
                        height: 128,
                    },
                    width as u32,
                    height as u32,
                );
                let left = region.x as usize;
                let right = (region.x + region.width) as usize;
                let top = region.y as usize;
                let bottom = (region.y + region.height) as usize;
                let mut tile: Vec<_> = (top..bottom)
                    .flat_map(|y| source[y * width + left..y * width + right].iter().copied())
                    .collect();
                progressive_filter(
                    &mut tile,
                    &mut Vec::new(),
                    right - left,
                    bottom - top,
                    blur,
                    Point::new(left as f32, top as f32),
                    reference,
                );
                for y in 128..256 {
                    assert_eq!(
                        &tile[(y - top) * (right - left) + tile_left - left
                            ..(y - top) * (right - left) + tile_right - left],
                        &full[y * width + tile_left..y * width + tile_right],
                        "tile seam in {blur:?}, row {y}"
                    );
                }
            }
        }
    }
}

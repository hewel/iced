//! Premultiplied, edge-extended three-box Gaussian approximation.
use crate::core::Rectangle;

pub fn radius(value: f32) -> f32 {
    if value.is_nan() || value <= 0.0 {
        0.0
    } else {
        value.min(128.0)
    }
}

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

#[derive(Debug, Default)]
pub struct Scene {
    entries: Vec<SceneEntry>,
    scratch: Vec<u32>,
    pub hits: u64,
    pub misses: u64,
}

#[derive(Debug)]
struct SceneEntry {
    width: u32,
    height: u32,
    sigma: f32,
    scale: f32,
    input: Vec<u32>,
    output: Vec<u32>,
}

impl Scene {
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
        bounds: Rectangle,
        sigma: f32,
        scale: f32,
    ) {
        let width = pixels.width();
        let height = pixels.height();
        if self
            .entries
            .iter()
            .any(|entry| entry.width != width || entry.height != height)
        {
            self.entries.clear();
            self.scratch = Vec::new();
        }
        let input: &[u32] = bytemuck::cast_slice(pixels.data_mut());
        let found = self.entries.iter().position(|entry| {
            entry.width == width
                && entry.height == height
                && entry.sigma == sigma
                && entry.scale == scale
                && entry.input == input
        });
        let index = if let Some(index) = found {
            self.hits += 1;
            index
        } else {
            self.misses += 1;
            let bytes = input.len() * 8;
            // At most four snapshots and 64 MiB, except one viewport-sized snapshot.
            // Recycle an entry instead of allocating on live-background updates.
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
                    sigma,
                    scale,
                    input: Vec::new(),
                    output: Vec::new(),
                });
            entry.width = width;
            entry.height = height;
            entry.sigma = sigma;
            entry.scale = scale;
            entry.input.clear();
            if entry.input.capacity() < input.len() {
                entry.input.reserve_exact(input.len());
            }
            entry.input.extend_from_slice(input);
            entry.output.clear();
            if entry.output.capacity() < input.len() {
                entry.output.reserve_exact(input.len());
            }
            entry.output.extend_from_slice(input);
            filter(
                &mut entry.output,
                &mut self.scratch,
                width as usize,
                height as usize,
                sigma,
            );
            let entry_bytes = (entry.input.capacity() + entry.output.capacity()) * 4;
            while !self.entries.is_empty() && self.retained_bytes() + entry_bytes > 64 * 1024 * 1024
            {
                drop(self.entries.remove(0));
            }
            self.entries.push(entry);
            self.entries.len() - 1
        };
        let output = &self.entries[index].output;
        if let Some([left, top, right, bottom]) =
            crate::engine::pixel_roi(bounds, bounds, width, height)
        {
            let target: &mut [u32] = bytemuck::cast_slice_mut(pixels.data_mut());
            for y in top..bottom {
                let start = y as usize * width as usize + left as usize;
                let end = y as usize * width as usize + right as usize;
                target[start..end].copy_from_slice(&output[start..end]);
            }
        }
    }
}

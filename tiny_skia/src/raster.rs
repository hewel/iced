use crate::core::image as raster;
use crate::core::{Blur, Point, Rectangle, Size};
use crate::engine::{image_envelope, pixel_channels, pixel_roi, source_over};
use crate::graphics;
use crate::graphics::shape::{self, RoundedRectangle};

use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::collections::hash_map;

#[derive(Debug)]
pub struct Pipeline {
    cache: RefCell<Cache>,
    blur: BlurCache,
}

impl Pipeline {
    pub fn new() -> Self {
        Self {
            cache: RefCell::new(Cache::default()),
            blur: BlurCache::default(),
        }
    }

    pub fn load(&self, handle: &raster::Handle) -> Result<raster::Allocation, raster::Error> {
        let mut cache = self.cache.borrow_mut();
        let image = cache.allocate(handle)?;

        #[allow(unsafe_code)]
        Ok(unsafe { raster::allocate(handle, Size::new(image.width(), image.height())) })
    }

    pub fn dimensions(&self, handle: &raster::Handle) -> Option<Size<u32>> {
        let mut cache = self.cache.borrow_mut();
        let image = cache.allocate(handle).ok()?;

        Some(Size::new(image.width(), image.height()))
    }

    pub fn draw(
        &mut self,
        image: &raster::Image,
        bounds: Rectangle,
        frame: &RoundedRectangle,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &tiny_skia::Mask,
        clip_bounds: Rectangle,
        blur: Blur,
    ) {
        let mut cache = self.cache.borrow_mut();

        let Ok(source) = cache.allocate(&image.handle) else {
            return;
        };
        let Some(crop) =
            raster::crop_bounds(Size::new(source.width(), source.height()), image.crop)
        else {
            return;
        };
        let rotation = f32::from(image.rotation);
        let Some(envelope) = image_envelope(bounds, clip_bounds, rotation) else {
            return;
        };
        let Some([left, top, right, bottom]) =
            pixel_roi(envelope, clip_bounds, pixels.width(), pixels.height())
        else {
            return;
        };
        let Some(content) = RoundedRectangle::new(bounds, 0.0.into(), 0.0) else {
            return;
        };
        let opacity = if image.opacity.is_nan() {
            0.0
        } else {
            image.opacity.clamp(0.0, 1.0)
        };
        if opacity == 0.0 {
            return;
        }
        if matches!(blur, Blur::Linear(_)) {
            draw_progressive(
                &mut self.blur,
                image,
                source,
                crop,
                bounds,
                frame,
                pixels,
                clip_mask,
                clip_bounds,
                [left, top, right, bottom],
                opacity,
                blur,
            );
            return;
        }
        let sigma = blur.maximum();
        let (source, crop) = if sigma > 0.0 {
            let source = self.blur.prepare(image, source, crop, bounds.size(), sigma);
            let crop = Rectangle {
                x: 0,
                y: 0,
                width: source.width(),
                height: source.height(),
            };
            (source, crop)
        } else {
            (source, crop)
        };
        let (sin, cos) = rotation.sin_cos();
        let center = bounds.center();
        let stride = pixels.width() as usize;
        let sample = |x: i64, y: i64| {
            let x = x.clamp(
                i64::from(crop.x),
                i64::from(crop.x) + i64::from(crop.width) - 1,
            ) as usize;
            let y = y.clamp(
                i64::from(crop.y),
                i64::from(crop.y) + i64::from(crop.height) - 1,
            ) as usize;
            pixel_channels(source.pixels()[y * source.width() as usize + x])
        };
        for y in top..bottom {
            for x in left..right {
                let index = y as usize * stride + x as usize;
                let mask = f32::from(clip_mask.data()[index]) / 255.0;
                if mask == 0.0 {
                    continue;
                }
                let point = Point::new(x as f32 + 0.5, y as f32 + 0.5);
                let delta = point - center;
                let local = Point::new(
                    center.x + cos * delta.x + sin * delta.y,
                    center.y - sin * delta.x + cos * delta.y,
                );
                let coverage = shape::coverage(frame.distance(point).max(content.distance(local)))
                    * opacity
                    * mask;
                if coverage == 0.0 {
                    continue;
                }
                let u = crop.x as f32 + (local.x - bounds.x) / bounds.width * crop.width as f32;
                let v = crop.y as f32 + (local.y - bounds.y) / bounds.height * crop.height as f32;
                let color = match image.filter_method {
                    raster::FilterMethod::Nearest => sample(u.floor() as i64, v.floor() as i64),
                    raster::FilterMethod::Linear => {
                        let u = u - 0.5;
                        let v = v - 0.5;
                        let x = u.floor() as i64;
                        let y = v.floor() as i64;
                        let tx = u - u.floor();
                        let ty = v - v.floor();
                        let a = sample(x, y);
                        let b = sample(x + 1, y);
                        let c = sample(x, y + 1);
                        let d = sample(x + 1, y + 1);
                        std::array::from_fn(|channel| {
                            (a[channel] * (1.0 - tx) + b[channel] * tx) * (1.0 - ty)
                                + (c[channel] * (1.0 - tx) + d[channel] * tx) * ty
                        })
                    }
                };
                source_over(
                    &mut pixels.pixels_mut()[index],
                    color.map(|channel| channel * coverage),
                );
            }
        }
    }

    pub fn trim_cache(&mut self) {
        self.cache.borrow_mut().trim();
        let cache = self.cache.borrow();
        self.blur
            .entries
            .retain(|entry| cache.entries.contains_key(&entry.key.id));
    }

    pub fn blur_statistics(&self) -> crate::core::renderer::BlurStatistics {
        crate::core::renderer::BlurStatistics {
            image_hits: self.blur.hits,
            image_misses: self.blur.misses,
            retained_bytes: self.blur.retained_bytes(),
            ..Default::default()
        }
    }
}

/// A physical pixel grid with bounded output tiles. Only tiles intersecting the
/// inverse-rotated viewport are materialized, and source halos precede clipping.
#[allow(clippy::too_many_arguments)]
fn draw_progressive(
    cache: &mut BlurCache,
    image: &raster::Image,
    source: tiny_skia::PixmapRef<'_>,
    crop: Rectangle<u32>,
    bounds: Rectangle,
    frame: &RoundedRectangle,
    pixels: &mut tiny_skia::PixmapMut<'_>,
    clip_mask: &tiny_skia::Mask,
    clip_bounds: Rectangle,
    roi: [u32; 4],
    opacity: f32,
    blur: Blur,
) {
    const TILE: u32 = 256;
    let [left, top, right, bottom] = roi;
    let width = bounds.width.ceil().max(1.0) as u32;
    let height = bounds.height.ceil().max(1.0) as u32;
    let (sin, cos) = f32::from(image.rotation).sin_cos();
    let center = bounds.center();
    let inverse = |point: Point| {
        let delta = point - center;
        Point::new(
            center.x + cos * delta.x + sin * delta.y,
            center.y - sin * delta.x + cos * delta.y,
        )
    };
    let forward = |point: Point| {
        let delta = point - center;
        Point::new(
            center.x + cos * delta.x - sin * delta.y,
            center.y + sin * delta.x + cos * delta.y,
        )
    };
    let corners = [
        Point::new(left as f32, top as f32),
        Point::new(right as f32, top as f32),
        Point::new(left as f32, bottom as f32),
        Point::new(right as f32, bottom as f32),
    ]
    .map(inverse);
    let x0 = (((corners.iter().map(|p| p.x).fold(f32::INFINITY, f32::min) - bounds.x)
        / bounds.width
        * width as f32)
        .floor()
        .max(0.0) as u32)
        .min(width - 1)
        / TILE;
    let y0 = (((corners.iter().map(|p| p.y).fold(f32::INFINITY, f32::min) - bounds.y)
        / bounds.height
        * height as f32)
        .floor()
        .max(0.0) as u32)
        .min(height - 1)
        / TILE;
    let x1 = (((corners
        .iter()
        .map(|p| p.x)
        .fold(f32::NEG_INFINITY, f32::max)
        - bounds.x)
        / bounds.width
        * width as f32)
        .ceil()
        .max(0.0) as u32)
        .min(width - 1)
        / TILE;
    let y1 = (((corners
        .iter()
        .map(|p| p.y)
        .fold(f32::NEG_INFINITY, f32::max)
        - bounds.y)
        / bounds.height
        * height as f32)
        .ceil()
        .max(0.0) as u32)
        .min(height - 1)
        / TILE;
    let Some(content) = RoundedRectangle::new(bounds, 0.0.into(), 0.0) else {
        return;
    };
    let stride = pixels.width() as usize;
    for tile_y in y0..=y1 {
        for tile_x in x0..=x1 {
            let x = tile_x * TILE;
            let y = tile_y * TILE;
            let end_x = x.saturating_add(TILE).min(width);
            let end_y = y.saturating_add(TILE).min(height);
            let tile_corners = [
                Point::new(x as f32, y as f32),
                Point::new(end_x as f32, y as f32),
                Point::new(x as f32, end_y as f32),
                Point::new(end_x as f32, end_y as f32),
            ]
            .map(|point| {
                forward(Point::new(
                    bounds.x + point.x / width as f32 * bounds.width,
                    bounds.y + point.y / height as f32 * bounds.height,
                ))
            });
            let min_x = tile_corners
                .iter()
                .map(|p| p.x)
                .fold(f32::INFINITY, f32::min);
            let min_y = tile_corners
                .iter()
                .map(|p| p.y)
                .fold(f32::INFINITY, f32::min);
            let max_x = tile_corners
                .iter()
                .map(|p| p.x)
                .fold(f32::NEG_INFINITY, f32::max);
            let max_y = tile_corners
                .iter()
                .map(|p| p.y)
                .fold(f32::NEG_INFINITY, f32::max);
            let Some([tile_left, tile_top, tile_right, tile_bottom]) = pixel_roi(
                Rectangle {
                    x: min_x,
                    y: min_y,
                    width: max_x - min_x,
                    height: max_y - min_y,
                }
                .expand(0.5),
                clip_bounds,
                pixels.width(),
                pixels.height(),
            ) else {
                continue;
            };
            // One extra pixel supports interpolation across tile edges.
            let region = Rectangle {
                x: x.saturating_sub(1),
                y: y.saturating_sub(1),
                width: end_x.saturating_add(1).min(width) - x.saturating_sub(1),
                height: end_y.saturating_add(1).min(height) - y.saturating_sub(1),
            };
            let blurred =
                cache.prepare_progressive(image, source, crop, bounds.size(), blur, region);
            let blurred_crop = Rectangle {
                x: 0,
                y: 0,
                width: blurred.width(),
                height: blurred.height(),
            };
            for py in tile_top..tile_bottom {
                for px in tile_left..tile_right {
                    let index = py as usize * stride + px as usize;
                    let mask = f32::from(clip_mask.data()[index]) / 255.0;
                    if mask == 0.0 {
                        continue;
                    }
                    let point = Point::new(px as f32 + 0.5, py as f32 + 0.5);
                    let local = inverse(point);
                    let u = (local.x - bounds.x) / bounds.width * width as f32;
                    let v = (local.y - bounds.y) / bounds.height * height as f32;
                    let owner_x = (u.floor().max(0.0) as u32).min(width - 1) / TILE;
                    let owner_y = (v.floor().max(0.0) as u32).min(height - 1) / TILE;
                    if owner_x != tile_x || owner_y != tile_y {
                        continue;
                    }
                    let coverage =
                        shape::coverage(frame.distance(point).max(content.distance(local)))
                            * opacity
                            * mask;
                    if coverage == 0.0 {
                        continue;
                    }
                    let color = if blur.radius_at(local, bounds) == 0.0 {
                        // Preserve the source sampling contract at the sharp end,
                        // including fractional placement and nearest-neighbor detail.
                        sample_image(
                            source,
                            crop,
                            crop.x as f32 + u / width as f32 * crop.width as f32,
                            crop.y as f32 + v / height as f32 * crop.height as f32,
                            image.filter_method,
                        )
                    } else {
                        sample_image(
                            blurred,
                            blurred_crop,
                            u - region.x as f32,
                            v - region.y as f32,
                            image.filter_method,
                        )
                    };
                    source_over(
                        &mut pixels.pixels_mut()[index],
                        color.map(|channel| channel * coverage),
                    );
                }
            }
        }
    }
}

fn sample_image(
    source: tiny_skia::PixmapRef<'_>,
    crop: Rectangle<u32>,
    u: f32,
    v: f32,
    filter: raster::FilterMethod,
) -> [f32; 4] {
    let sample = |x: i64, y: i64| {
        let x = x.clamp(
            i64::from(crop.x),
            i64::from(crop.x) + i64::from(crop.width) - 1,
        ) as usize;
        let y = y.clamp(
            i64::from(crop.y),
            i64::from(crop.y) + i64::from(crop.height) - 1,
        ) as usize;
        pixel_channels(source.pixels()[y * source.width() as usize + x])
    };
    match filter {
        raster::FilterMethod::Nearest => sample(u.floor() as i64, v.floor() as i64),
        raster::FilterMethod::Linear => {
            let u = u - 0.5;
            let v = v - 0.5;
            let x = u.floor() as i64;
            let y = v.floor() as i64;
            let tx = u - u.floor();
            let ty = v - v.floor();
            let a = sample(x, y);
            let b = sample(x + 1, y);
            let c = sample(x, y + 1);
            let d = sample(x + 1, y + 1);
            std::array::from_fn(|channel| {
                (a[channel] * (1.0 - tx) + b[channel] * tx) * (1.0 - ty)
                    + (c[channel] * (1.0 - tx) + d[channel] * tx) * ty
            })
        }
    }
}

#[derive(Debug, Default)]
struct Cache {
    entries: FxHashMap<raster::Id, Option<Entry>>,
    hits: FxHashSet<raster::Id>,
}

impl Cache {
    pub fn allocate(
        &mut self,
        handle: &raster::Handle,
    ) -> Result<tiny_skia::PixmapRef<'_>, raster::Error> {
        let id = handle.id();

        if let hash_map::Entry::Vacant(entry) = self.entries.entry(id) {
            let image = match graphics::image::load(handle) {
                Ok(image) => image,
                Err(error) => {
                    let _ = entry.insert(None);

                    return Err(error);
                }
            };

            if image.width() == 0 || image.height() == 0 {
                return Err(raster::Error::Empty);
            }

            let mut buffer = vec![0u32; image.width() as usize * image.height() as usize];

            for (i, pixel) in image.pixels().enumerate() {
                let [r, g, b, a] = pixel.0;

                buffer[i] = bytemuck::cast(tiny_skia::ColorU8::from_rgba(b, g, r, a).premultiply());
            }

            let _ = entry.insert(Some(Entry {
                width: image.width(),
                height: image.height(),
                pixels: buffer,
            }));
        }

        let _ = self.hits.insert(id);

        Ok(self
            .entries
            .get(&id)
            .unwrap()
            .as_ref()
            .map(|entry| {
                tiny_skia::PixmapRef::from_bytes(
                    bytemuck::cast_slice(&entry.pixels),
                    entry.width,
                    entry.height,
                )
                .expect("Build pixmap from image bytes")
            })
            .expect("Image should be allocated"))
    }

    fn trim(&mut self) {
        self.entries.retain(|key, _| self.hits.contains(key));
        self.hits.clear();
    }
}

#[derive(Debug)]
struct Entry {
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}

#[derive(Debug, PartialEq)]
struct BlurKey {
    id: raster::Id,
    crop: Rectangle<u32>,
    size: Size,
    blur: Blur,
    region: Option<Rectangle<u32>>,
    filter: raster::FilterMethod,
}

#[derive(Debug)]
struct Blurred {
    key: BlurKey,
    width: u32,
    height: u32,
    pixels: Vec<u32>,
}

#[derive(Debug, Default)]
struct BlurCache {
    entries: Vec<Blurred>,
    scratch: Vec<u32>,
    hits: u64,
    misses: u64,
}

impl BlurCache {
    fn retained_bytes(&self) -> usize {
        self.scratch.capacity() * 4
            + self
                .entries
                .iter()
                .map(|entry| entry.pixels.capacity() * 4)
                .sum::<usize>()
    }

    fn prepare_progressive(
        &mut self,
        image: &raster::Image,
        source: tiny_skia::PixmapRef<'_>,
        crop: Rectangle<u32>,
        size: Size,
        blur: Blur,
        region: Rectangle<u32>,
    ) -> tiny_skia::PixmapRef<'_> {
        let key = BlurKey {
            id: image.handle.id(),
            crop,
            size,
            blur,
            region: Some(region),
            filter: image.filter_method,
        };
        let index = if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            self.hits += 1;
            index
        } else {
            self.misses += 1;
            let width = size.width.ceil().max(1.0) as u32;
            let height = size.height.ceil().max(1.0) as u32;
            let dependencies = crate::blur::source_region(blur, region, width, height);
            let left = dependencies.x;
            let top = dependencies.y;
            let tile_width = dependencies.width;
            let tile_height = dependencies.height;
            let count = tile_width as usize * tile_height as usize;
            let needed = (count + region.width as usize * region.height as usize) * 4;
            while !self.entries.is_empty()
                && (self.entries.len() >= 64 || self.retained_bytes() + needed > 64 * 1024 * 1024)
            {
                drop(self.entries.remove(0));
            }
            let mut pixels = vec![0u32; count];
            for y in 0..tile_height {
                for x in 0..tile_width {
                    let u = crop.x as f32
                        + (left as f32 + x as f32 + 0.5) / width as f32 * crop.width as f32;
                    let v = crop.y as f32
                        + (top as f32 + y as f32 + 0.5) / height as f32 * crop.height as f32;
                    let color = sample_image(source, crop, u, v, image.filter_method)
                        .map(|channel| (channel * 255.0).round() as u8);
                    pixels[y as usize * tile_width as usize + x as usize] =
                        u32::from_ne_bytes(color);
                }
            }
            crate::blur::progressive_filter(
                &mut pixels,
                &mut self.scratch,
                tile_width as usize,
                tile_height as usize,
                blur,
                Point::new(left as f32, top as f32),
                Rectangle::with_size(Size::new(width as f32, height as f32)),
            );
            let mut output = Vec::with_capacity(region.width as usize * region.height as usize);
            for y in region.y..region.y + region.height {
                let start = (y - top) as usize * tile_width as usize + (region.x - left) as usize;
                output.extend_from_slice(&pixels[start..start + region.width as usize]);
            }
            // An unusually large DPI/kernel can need a large temporary halo; do
            // not retain that allocation beyond the operation's working set.
            if self.scratch.capacity() * 4 + output.capacity() * 4 > 64 * 1024 * 1024 {
                self.scratch = Vec::new();
            }
            while !self.entries.is_empty()
                && self.retained_bytes() + output.capacity() * 4 > 64 * 1024 * 1024
            {
                drop(self.entries.remove(0));
            }
            self.entries.push(Blurred {
                key,
                width: region.width,
                height: region.height,
                pixels: output,
            });
            self.entries.len() - 1
        };
        let entry = &self.entries[index];
        tiny_skia::PixmapRef::from_bytes(
            bytemuck::cast_slice(&entry.pixels),
            entry.width,
            entry.height,
        )
        .expect("Blurred image tile")
    }

    fn prepare(
        &mut self,
        image: &raster::Image,
        source: tiny_skia::PixmapRef<'_>,
        crop: Rectangle<u32>,
        size: Size,
        sigma: f32,
    ) -> tiny_skia::PixmapRef<'_> {
        let key = BlurKey {
            id: image.handle.id(),
            crop,
            size,
            blur: Blur::Uniform(sigma),
            region: None,
            filter: image.filter_method,
        };
        let index = if let Some(index) = self.entries.iter().position(|entry| entry.key == key) {
            self.hits += 1;
            index
        } else {
            self.misses += 1;
            // Isolated full-content renditions, capped at 2048 on either axis.
            // Large blur kernels remain linear-time; no atlas neighbors leak in.
            let reduction = (2048.0 / size.width.max(size.height)).min(1.0);
            let width = (size.width * reduction).ceil().max(1.0) as u32;
            let height = (size.height * reduction).ceil().max(1.0) as u32;
            let count = width as usize * height as usize;
            let mut pixels = if !self.entries.is_empty()
                && (self.entries.len() >= 16
                    || self.retained_bytes() + count * 8 > 64 * 1024 * 1024)
            {
                self.entries.remove(0).pixels
            } else {
                Vec::new()
            };
            if pixels.capacity() < count {
                pixels.reserve_exact(count - pixels.len());
            }
            if self.scratch.capacity() < count {
                self.scratch.reserve_exact(count - self.scratch.len());
            }
            while !self.entries.is_empty()
                && self.retained_bytes() + pixels.capacity() * 4 > 64 * 1024 * 1024
            {
                drop(self.entries.remove(0));
            }
            pixels.resize(count, 0);
            let sample = |x: i64, y: i64| {
                let x = x.clamp(i64::from(crop.x), i64::from(crop.x + crop.width - 1)) as usize;
                let y = y.clamp(i64::from(crop.y), i64::from(crop.y + crop.height - 1)) as usize;
                let pixel = source.pixels()[y * source.width() as usize + x];
                [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
            };
            for y in 0..height {
                for x in 0..width {
                    let u = crop.x as f32 + (x as f32 + 0.5) / width as f32 * crop.width as f32;
                    let v = crop.y as f32 + (y as f32 + 0.5) / height as f32 * crop.height as f32;
                    let color = match image.filter_method {
                        raster::FilterMethod::Nearest => sample(u.floor() as i64, v.floor() as i64),
                        raster::FilterMethod::Linear => {
                            let u = u - 0.5;
                            let v = v - 0.5;
                            let x = u.floor() as i64;
                            let y = v.floor() as i64;
                            let tx = u - u.floor();
                            let ty = v - v.floor();
                            let a = sample(x, y);
                            let b = sample(x + 1, y);
                            let c = sample(x, y + 1);
                            let d = sample(x + 1, y + 1);
                            std::array::from_fn(|channel| {
                                ((f32::from(a[channel]) * (1.0 - tx) + f32::from(b[channel]) * tx)
                                    * (1.0 - ty)
                                    + (f32::from(c[channel]) * (1.0 - tx)
                                        + f32::from(d[channel]) * tx)
                                        * ty)
                                    .round() as u8
                            })
                        }
                    };
                    pixels[y as usize * width as usize + x as usize] = u32::from_ne_bytes(color);
                }
            }
            crate::blur::filter(
                &mut pixels,
                &mut self.scratch,
                width as usize,
                height as usize,
                sigma * reduction,
            );
            self.entries.push(Blurred {
                key,
                width,
                height,
                pixels,
            });
            self.entries.len() - 1
        };
        let entry = &self.entries[index];
        tiny_skia::PixmapRef::from_bytes(
            bytemuck::cast_slice(&entry.pixels),
            entry.width,
            entry.height,
        )
        .expect("Blurred image pixels")
    }
}

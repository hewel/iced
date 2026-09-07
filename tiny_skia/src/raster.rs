use crate::core::image as raster;
use crate::core::{Point, Rectangle, Size};
use crate::engine::{image_envelope, pixel_channels, pixel_roi, source_over};
use crate::graphics;
use crate::graphics::shape::{self, RoundedRectangle};

use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::collections::hash_map;

#[derive(Debug)]
pub struct Pipeline {
    cache: RefCell<Cache>,
}

impl Pipeline {
    pub fn new() -> Self {
        Self {
            cache: RefCell::new(Cache::default()),
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

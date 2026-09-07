use crate::Primitive;
use crate::core::renderer::Quad;
use crate::core::{Background, Color, Gradient, Point, Rectangle, Size, Transformation, Vector};
use crate::graphics::shape::{self, RoundedRectangle};
use crate::graphics::{Image, Text};
use crate::text;

#[derive(Debug)]
pub struct Engine {
    text_pipeline: text::Pipeline,
    scratch: Option<tiny_skia::Pixmap>,

    #[cfg(feature = "image")]
    pub(crate) raster_pipeline: crate::raster::Pipeline,
    #[cfg(feature = "svg")]
    pub(crate) vector_pipeline: crate::vector::Pipeline,
}

impl Engine {
    pub fn new() -> Self {
        Self {
            text_pipeline: text::Pipeline::new(),
            scratch: None,
            #[cfg(feature = "image")]
            raster_pipeline: crate::raster::Pipeline::new(),
            #[cfg(feature = "svg")]
            vector_pipeline: crate::vector::Pipeline::new(),
        }
    }

    pub fn draw_quad(
        &mut self,
        quad: &Quad,
        background: &Background,
        transformation: Transformation,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &mut tiny_skia::Mask,
        clip_bounds: Rectangle,
    ) {
        let scale = transformation.scale_factor();
        let bounds = shape::snap(quad.bounds * transformation, quad.snap);
        let Some(frame) = RoundedRectangle::new(
            bounds,
            scaled_radius(quad.border.radius, scale),
            quad.border.smoothing,
        ) else {
            return;
        };
        let Some(envelope) = quad_envelope(quad, transformation) else {
            return;
        };
        let Some([left, top, right, bottom]) =
            pixel_roi(envelope, clip_bounds, pixels.width(), pixels.height())
        else {
            return;
        };
        let width =
            scaled_length(quad.border.width, scale).min(bounds.width.min(bounds.height) * 0.5);
        let saturated = width > 0.0 && width >= bounds.width.min(bounds.height) * 0.5;
        let border = premultiplied(quad.border.color);
        let shadow = premultiplied(quad.shadow.color);
        let offset = shadow_offset(quad.shadow.offset, scale);
        let blur = shadow_blur(quad.shadow.blur_radius, scale);
        let solid = match background {
            Background::Color(color) => Some(premultiplied(*color)),
            Background::Gradient(Gradient::Linear(linear)) => {
                if self.scratch.as_ref().is_none_or(|scratch| {
                    scratch.width() < pixels.width() || scratch.height() < pixels.height()
                }) {
                    let (width, height) = self.scratch.as_ref().map_or(
                        (pixels.width(), pixels.height()),
                        |scratch| {
                            (
                                scratch.width().max(pixels.width()),
                                scratch.height().max(pixels.height()),
                            )
                        },
                    );
                    self.scratch = tiny_skia::Pixmap::new(width, height);
                }
                let Some(scratch) = self.scratch.as_mut() else {
                    return;
                };
                let reference_bounds = linear
                    .reference_bounds
                    .map(|bounds| bounds * transformation)
                    .unwrap_or(bounds);
                let (start, end) = reference_bounds.chord(linear.angle);
                let mut stops: Vec<_> = linear
                    .stops
                    .into_iter()
                    .flatten()
                    .map(|stop| tiny_skia::GradientStop::new(stop.offset, into_color(stop.color)))
                    .collect();
                if stops.is_empty() {
                    stops.push(tiny_skia::GradientStop::new(0.0, tiny_skia::Color::BLACK));
                }
                let shader = tiny_skia::LinearGradient::new(
                    tiny_skia::Point::from_xy(start.x, start.y),
                    tiny_skia::Point::from_xy(end.x, end.y),
                    stops,
                    tiny_skia::SpreadMode::Pad,
                    tiny_skia::Transform::identity(),
                )
                .unwrap_or(tiny_skia::Shader::SolidColor(tiny_skia::Color::TRANSPARENT));
                scratch.fill_rect(
                    tiny_skia::Rect::from_xywh(
                        left as f32,
                        top as f32,
                        (right - left) as f32,
                        (bottom - top) as f32,
                    )
                    .expect("Nonempty ROI"),
                    &tiny_skia::Paint {
                        shader,
                        blend_mode: tiny_skia::BlendMode::Source,
                        anti_alias: false,
                        ..Default::default()
                    },
                    tiny_skia::Transform::identity(),
                    None,
                );
                None
            }
        };
        let stride = pixels.width() as usize;
        // A rectangular solid's fully covered interior is a span, not a set
        // of distance queries. The remaining boundary still uses the same AA.
        let rectangular = <[f32; 4]>::from(quad.border.radius)
            .into_iter()
            .all(|radius| shape::normalize_length(radius) == 0.0);
        let interior_left = ((bounds.x + width + 0.5).ceil() as u32).clamp(left, right);
        let interior_right = ((bounds.x + bounds.width - width - 0.5).floor().max(0.0) as u32)
            .clamp(interior_left, right);
        for y in top..bottom {
            let interior = if rectangular
                && !saturated
                && solid.is_some()
                && y as f32 + 0.5 >= bounds.y + width + 0.5
                && y as f32 + 0.5 <= bounds.y + bounds.height - width - 0.5
            {
                interior_left..interior_right
            } else {
                left..left
            };
            if let Some(fill) = solid {
                let start = y as usize * stride + interior.start as usize;
                let end = y as usize * stride + interior.end as usize;
                let mask = &clip_mask.data()[start..end];
                let target = &mut pixels.pixels_mut()[start..end];
                if fill[3] == 1.0 && mask.iter().all(|alpha| *alpha == 255) {
                    let mut color = tiny_skia::PremultipliedColorU8::TRANSPARENT;
                    source_over(&mut color, fill);
                    target.fill(color);
                } else {
                    for (target, alpha) in target.iter_mut().zip(mask) {
                        source_over(
                            target,
                            fill.map(|channel| channel * f32::from(*alpha) / 255.0),
                        );
                    }
                }
            }
            for x in (left..interior.start).chain(interior.end..right) {
                let index = y as usize * stride + x as usize;
                let mask = f32::from(clip_mask.data()[index]) / 255.0;
                if mask == 0.0 {
                    continue;
                }
                let point = Point::new(x as f32 + 0.5, y as f32 + 0.5);
                let distance = frame.distance(point);
                let outer = shape::coverage(distance);
                let inner = if saturated {
                    0.0
                } else {
                    shape::coverage(distance + width)
                };
                let border_coverage = (outer - inner).max(0.0);
                let fill = solid.unwrap_or_else(|| {
                    let scratch = self.scratch.as_ref().expect("Gradient color field");
                    pixel_channels(
                        scratch.pixels()[y as usize * scratch.width() as usize + x as usize],
                    )
                });
                let shadow_coverage = if shadow[3] > 0.0 && outer < 1.0 {
                    let distance = frame.distance(point - offset);
                    (if blur > 0.0 {
                        1.0 - smoothstep(-blur, blur, distance)
                    } else {
                        shape::coverage(distance)
                    }) * (1.0 - outer)
                } else {
                    0.0
                };
                let source = std::array::from_fn(|channel| {
                    (fill[channel] * inner
                        + border[channel] * border_coverage
                        + shadow[channel] * shadow_coverage)
                        * mask
                });
                source_over(&mut pixels.pixels_mut()[index], source);
            }
        }
    }

    pub fn draw_text(
        &mut self,
        text: &Text,
        transformation: Transformation,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &mut tiny_skia::Mask,
        clip_bounds: Rectangle,
    ) {
        match text {
            Text::Paragraph {
                paragraph,
                position,
                color,
                clip_bounds: local_clip_bounds,
                transformation: local_transformation,
            } => {
                let transformation = transformation * *local_transformation;
                let Some(clip_bounds) =
                    clip_bounds.intersection(&(*local_clip_bounds * transformation))
                else {
                    return;
                };

                let physical_bounds =
                    Rectangle::new(*position, paragraph.min_bounds) * transformation;

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = match physical_bounds.is_within(&clip_bounds) {
                    true => None,
                    false => {
                        adjust_clip_mask(clip_mask, clip_bounds);
                        Some(clip_mask as &_)
                    }
                };

                self.text_pipeline.draw_paragraph(
                    paragraph,
                    *position,
                    *color,
                    pixels,
                    clip_mask,
                    transformation,
                );
            }
            Text::Editor {
                editor,
                position,
                color,
                clip_bounds: local_clip_bounds,
                transformation: local_transformation,
            } => {
                let transformation = transformation * *local_transformation;

                let Some(clip_bounds) =
                    clip_bounds.intersection(&(*local_clip_bounds * transformation))
                else {
                    return;
                };

                adjust_clip_mask(clip_mask, clip_bounds);

                self.text_pipeline.draw_editor(
                    editor,
                    *position,
                    *color,
                    pixels,
                    Some(clip_mask),
                    transformation,
                );
            }
            Text::Cached {
                content,
                bounds,
                color,
                size,
                line_height,
                font,
                align_x,
                align_y,
                shaping,
                wrapping,
                ellipsis,
                clip_bounds: local_clip_bounds,
            } => {
                let physical_bounds = *local_clip_bounds * transformation;

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask = match physical_bounds.is_within(&clip_bounds) {
                    true => None,
                    false => {
                        adjust_clip_mask(clip_mask, clip_bounds);
                        Some(clip_mask as &_)
                    }
                };

                self.text_pipeline.draw_cached(
                    content,
                    *bounds,
                    *color,
                    *size,
                    *line_height,
                    *font,
                    *align_x,
                    *align_y,
                    *shaping,
                    *wrapping,
                    *ellipsis,
                    pixels,
                    clip_mask,
                    transformation,
                );
            }
            Text::Raw {
                raw,
                transformation: local_transformation,
            } => {
                let Some(buffer) = raw.buffer.upgrade() else {
                    return;
                };

                let transformation = transformation * *local_transformation;
                let (width, height) = buffer.size();

                let physical_bounds = Rectangle::new(
                    raw.position,
                    Size::new(
                        width.unwrap_or(clip_bounds.width),
                        height.unwrap_or(clip_bounds.height),
                    ),
                ) * transformation;

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask =
                    (!physical_bounds.is_within(&clip_bounds)).then_some(clip_mask as &_);

                self.text_pipeline.draw_raw(
                    &buffer,
                    raw.position,
                    raw.color,
                    pixels,
                    clip_mask,
                    transformation,
                );
            }
        }
    }

    pub fn draw_primitive(
        &mut self,
        primitive: &Primitive,
        transformation: Transformation,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &mut tiny_skia::Mask,
        clip_bounds: Rectangle,
    ) {
        match primitive {
            Primitive::Fill { path, paint, rule } => {
                let physical_bounds = {
                    let bounds = path.bounds();

                    Rectangle {
                        x: bounds.x(),
                        y: bounds.y(),
                        width: bounds.width(),
                        height: bounds.height(),
                    } * transformation
                };

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask =
                    (!physical_bounds.is_within(&clip_bounds)).then_some(clip_mask as &_);

                pixels.fill_path(
                    path,
                    paint,
                    *rule,
                    into_transform(transformation),
                    clip_mask,
                );
            }
            Primitive::Stroke {
                path,
                paint,
                stroke,
            } => {
                let physical_bounds = {
                    let bounds = path.bounds();

                    Rectangle {
                        x: bounds.x() - stroke.width / 2.0,
                        y: bounds.y() - stroke.width / 2.0,
                        width: bounds.width() + stroke.width,
                        height: bounds.height() + stroke.width,
                    } * transformation
                };

                if !clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask =
                    (!physical_bounds.is_within(&clip_bounds)).then_some(clip_mask as &_);

                pixels.stroke_path(
                    path,
                    paint,
                    stroke,
                    into_transform(transformation),
                    clip_mask,
                );
            }
        }
    }

    pub fn draw_image(
        &mut self,
        image: &Image,
        _transformation: Transformation,
        _pixels: &mut tiny_skia::PixmapMut<'_>,
        _clip_mask: &mut tiny_skia::Mask,
        _clip_bounds: Rectangle,
    ) {
        match image {
            #[cfg(feature = "image")]
            Image::Raster {
                image,
                bounds,
                clip_bounds: local_clip_bounds,
            } => {
                let scale = _transformation.scale_factor();
                let bounds = shape::snap(*bounds * _transformation, image.snap);
                let frame_bounds = shape::snap(*local_clip_bounds * _transformation, image.snap);
                let Some(frame) = RoundedRectangle::new(
                    frame_bounds,
                    scaled_radius(image.border_radius, scale),
                    image.border_smoothing,
                ) else {
                    return;
                };
                let Some(clip_bounds) = frame_bounds.expand(0.5).intersection(&_clip_bounds) else {
                    return;
                };
                self.raster_pipeline.draw(
                    image,
                    bounds,
                    &frame,
                    _pixels,
                    _clip_mask,
                    clip_bounds,
                    image.blur * scale.abs(),
                );
            }
            #[cfg(feature = "svg")]
            Image::Vector { svg, bounds, .. } => {
                let physical_bounds = *bounds * _transformation;

                if !_clip_bounds.intersects(&physical_bounds) {
                    return;
                }

                let clip_mask =
                    (!physical_bounds.is_within(&_clip_bounds)).then_some(_clip_mask as &_);

                let center = physical_bounds.center();
                let radians = f32::from(svg.rotation);

                let transform = into_transform(_transformation).post_rotate_at(
                    radians.to_degrees(),
                    center.x,
                    center.y,
                );

                self.vector_pipeline.draw(
                    &svg.handle,
                    svg.color,
                    *bounds,
                    svg.opacity,
                    _pixels,
                    transform,
                    clip_mask,
                );
            }
            #[cfg(not(feature = "image"))]
            Image::Raster { .. } => {
                log::warn!("Unsupported primitive in `iced_tiny_skia`: {image:?}",);
            }
            #[cfg(not(feature = "svg"))]
            Image::Vector { .. } => {
                log::warn!("Unsupported primitive in `iced_tiny_skia`: {image:?}",);
            }
        }
    }

    pub fn trim(&mut self) {
        self.text_pipeline.trim_cache();

        #[cfg(feature = "image")]
        self.raster_pipeline.trim_cache();

        #[cfg(feature = "svg")]
        self.vector_pipeline.trim_cache();
    }
}

pub fn into_color(color: Color) -> tiny_skia::Color {
    tiny_skia::Color::from_rgba(color.b, color.g, color.r, color.a)
        .expect("Convert color from iced to tiny_skia")
}

fn into_transform(transformation: Transformation) -> tiny_skia::Transform {
    let translation = transformation.translation();

    tiny_skia::Transform {
        sx: transformation.scale_factor(),
        kx: 0.0,
        ky: 0.0,
        sy: transformation.scale_factor(),
        tx: translation.x,
        ty: translation.y,
    }
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let x = ((x - a) / (b - a)).clamp(0.0, 1.0);

    x * x * (3.0 - 2.0 * x)
}

pub fn adjust_clip_mask(clip_mask: &mut tiny_skia::Mask, bounds: Rectangle) {
    clip_mask.clear();

    let path = tiny_skia::PathBuilder::from_rect(
        tiny_skia::Rect::from_xywh(bounds.x, bounds.y, bounds.width, bounds.height)
            .expect("Create clip rectangle"),
    );

    clip_mask.fill_path(
        &path,
        tiny_skia::FillRule::EvenOdd,
        false,
        tiny_skia::Transform::default(),
    );
}

pub(crate) fn scaled_length(value: f32, scale: f32) -> f32 {
    (f64::from(shape::normalize_length(value)) * f64::from(scale)).clamp(0.0, f64::from(f32::MAX))
        as f32
}

pub(crate) fn scaled_radius(
    radius: crate::core::border::Radius,
    scale: f32,
) -> crate::core::border::Radius {
    let [top_left, top_right, bottom_right, bottom_left] =
        <[f32; 4]>::from(radius).map(|value| scaled_length(value, scale));
    crate::core::border::Radius {
        top_left,
        top_right,
        bottom_right,
        bottom_left,
    }
}

pub(crate) fn shadow_offset(offset: Vector, scale: f32) -> Vector {
    Vector::new(
        if offset.x.is_finite() {
            offset.x * scale
        } else {
            0.0
        },
        if offset.y.is_finite() {
            offset.y * scale
        } else {
            0.0
        },
    )
}

pub(crate) fn shadow_blur(blur: f32, scale: f32) -> f32 {
    if blur.is_finite() {
        scaled_length(blur, scale)
    } else {
        0.0
    }
}

pub(crate) fn quad_envelope(quad: &Quad, transformation: Transformation) -> Option<Rectangle> {
    let bounds = shape::snap(quad.bounds * transformation, quad.snap);
    let _ = RoundedRectangle::new(bounds, 0.0.into(), 0.0)?;
    let envelope = if quad.shadow.color.a > 0.0 {
        let scale = transformation.scale_factor();
        bounds.union(
            &(bounds + shadow_offset(quad.shadow.offset, scale))
                .expand(shadow_blur(quad.shadow.blur_radius, scale)),
        )
    } else {
        bounds
    };
    Some(envelope.expand(0.5))
}

pub(crate) fn image_envelope(
    bounds: Rectangle,
    frame: Rectangle,
    rotation: f32,
) -> Option<Rectangle> {
    let _ = RoundedRectangle::new(bounds, 0.0.into(), 0.0)?;
    let _ = RoundedRectangle::new(frame, 0.0.into(), 0.0)?;
    if !rotation.is_finite() {
        return None;
    }
    let (sin, cos) = rotation.sin_cos();
    let width = bounds.width * cos.abs() + bounds.height * sin.abs();
    let height = bounds.width * sin.abs() + bounds.height * cos.abs();
    Rectangle::new(
        Point::new(
            bounds.center_x() - width * 0.5,
            bounds.center_y() - height * 0.5,
        ),
        Size::new(width, height),
    )
    .expand(0.5)
    .intersection(&frame.expand(0.5))
}

pub(crate) fn pixel_roi(
    bounds: Rectangle,
    clip: Rectangle,
    width: u32,
    height: u32,
) -> Option<[u32; 4]> {
    let bounds = bounds
        .intersection(&clip)?
        .intersection(&Rectangle::with_size(Size::new(
            width as f32,
            height as f32,
        )))?;
    let left = bounds.x.floor().max(0.0) as u32;
    let top = bounds.y.floor().max(0.0) as u32;
    let right = (bounds.x + bounds.width).ceil().min(width as f32) as u32;
    let bottom = (bounds.y + bounds.height).ceil().min(height as f32) as u32;
    (right > left && bottom > top).then_some([left, top, right, bottom])
}

fn premultiplied(color: Color) -> [f32; 4] {
    [
        color.b * color.a,
        color.g * color.a,
        color.r * color.a,
        color.a,
    ]
}

pub(crate) fn pixel_channels(pixel: tiny_skia::PremultipliedColorU8) -> [f32; 4] {
    [pixel.red(), pixel.green(), pixel.blue(), pixel.alpha()]
        .map(|channel| f32::from(channel) / 255.0)
}

pub(crate) fn source_over(target: &mut tiny_skia::PremultipliedColorU8, source: [f32; 4]) {
    let destination = pixel_channels(*target);
    let result: [u8; 4] = std::array::from_fn(|channel| {
        ((source[channel] + destination[channel] * (1.0 - source[3])).clamp(0.0, 1.0) * 255.0)
            .round() as u8
    });
    *target = tiny_skia::PremultipliedColorU8::from_rgba(
        result[0].min(result[3]),
        result[1].min(result[3]),
        result[2].min(result[3]),
        result[3],
    )
    .expect("Premultiplied composition");
}

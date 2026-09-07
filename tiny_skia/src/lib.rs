#![allow(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]
pub mod window;

mod blur;
mod engine;
mod layer;
mod primitive;
mod text;

#[cfg(feature = "image")]
mod raster;

#[cfg(feature = "svg")]
mod vector;

#[cfg(feature = "geometry")]
pub mod geometry;

use iced_debug as debug;
pub use iced_graphics as graphics;
pub use iced_graphics::core;

pub use layer::Layer;
pub use primitive::Primitive;

#[cfg(feature = "geometry")]
pub use geometry::Geometry;

use crate::core::renderer;
use crate::core::{Background, Color, Font, Point, Rectangle, Size, Transformation};
use crate::engine::Engine;
use crate::graphics::Viewport;
use crate::graphics::compositor;
use crate::graphics::text::{Editor, Paragraph};

/// A [`tiny-skia`] graphics renderer for [`iced`].
///
/// [`tiny-skia`]: https://github.com/RazrFalcon/tiny-skia
/// [`iced`]: https://github.com/iced-rs/iced
#[derive(Debug)]
pub struct Renderer {
    settings: renderer::Settings,
    layers: layer::Stack,
    engine: Engine, // TODO: Shared engine
    scene_blur: blur::Scene,
    had_backdrop: bool,
}

impl Renderer {
    pub fn new(settings: renderer::Settings) -> Self {
        Self {
            settings,
            layers: layer::Stack::new(),
            engine: Engine::new(),
            scene_blur: blur::Scene::default(),
            had_backdrop: false,
        }
    }

    pub fn layers(&mut self) -> &[Layer] {
        self.layers.flush();
        self.layers.as_slice()
    }

    pub fn draw(
        &mut self,
        pixels: &mut tiny_skia::PixmapMut<'_>,
        clip_mask: &mut tiny_skia::Mask,
        viewport: &Viewport,
        damage: &[Rectangle],
        background_color: Color,
    ) {
        let scale_factor = viewport.scale_factor();
        self.layers.flush();
        let has_backdrop = self
            .layers
            .iter()
            .any(|layer| layer.backdrop_blur.is_some());
        let full_damage = [Rectangle::with_size(viewport.logical_size())];
        let damage = if has_backdrop || self.had_backdrop {
            &full_damage[..]
        } else {
            damage
        };
        self.had_backdrop = has_backdrop;

        for &damage_bounds in damage {
            let damage_bounds = damage_bounds * scale_factor;

            let path = tiny_skia::PathBuilder::from_rect(
                tiny_skia::Rect::from_xywh(
                    damage_bounds.x,
                    damage_bounds.y,
                    damage_bounds.width,
                    damage_bounds.height,
                )
                .expect("Create damage rectangle"),
            );

            pixels.fill_path(
                &path,
                &tiny_skia::Paint {
                    shader: tiny_skia::Shader::SolidColor(engine::into_color(background_color)),
                    anti_alias: false,
                    blend_mode: tiny_skia::BlendMode::Source,
                    ..Default::default()
                },
                tiny_skia::FillRule::default(),
                tiny_skia::Transform::identity(),
                None,
            );

            for layer in self.layers.iter() {
                let Some(layer_bounds) = damage_bounds.intersection(&(layer.bounds * scale_factor))
                else {
                    continue;
                };

                if let Some(radius) = layer.backdrop_blur {
                    self.scene_blur.apply(
                        pixels,
                        layer_bounds,
                        radius * scale_factor,
                        scale_factor,
                    );
                }
                engine::adjust_clip_mask(clip_mask, layer_bounds);

                if !layer.quads.is_empty() {
                    let render_span = debug::render(debug::Primitive::Quad);
                    for (quad, background) in &layer.quads {
                        self.engine.draw_quad(
                            quad,
                            background,
                            Transformation::scale(scale_factor),
                            pixels,
                            clip_mask,
                            layer_bounds,
                        );
                    }
                    render_span.finish();
                }

                if !layer.primitives.is_empty() {
                    let render_span = debug::render(debug::Primitive::Triangle);

                    for group in &layer.primitives {
                        let Some(group_bounds) =
                            (group.clip_bounds() * scale_factor).intersection(&layer_bounds)
                        else {
                            continue;
                        };

                        engine::adjust_clip_mask(clip_mask, group_bounds);

                        for primitive in group.as_slice() {
                            self.engine.draw_primitive(
                                primitive,
                                Transformation::scale(scale_factor) * group.transformation(),
                                pixels,
                                clip_mask,
                                group_bounds,
                            );
                        }

                        engine::adjust_clip_mask(clip_mask, layer_bounds);
                    }

                    render_span.finish();
                }

                if !layer.images.is_empty() {
                    let render_span = debug::render(debug::Primitive::Image);

                    for image in &layer.images {
                        self.engine.draw_image(
                            image,
                            Transformation::scale(scale_factor),
                            pixels,
                            clip_mask,
                            layer_bounds,
                        );
                    }

                    render_span.finish();
                }

                if !layer.text.is_empty() {
                    let render_span = debug::render(debug::Primitive::Image);

                    for group in &layer.text {
                        for text in group.as_slice() {
                            self.engine.draw_text(
                                text,
                                Transformation::scale(scale_factor) * group.transformation(),
                                pixels,
                                clip_mask,
                                layer_bounds,
                            );
                        }
                    }

                    render_span.finish();
                }
            }
        }

        self.engine.trim();
    }
}

impl core::Renderer for Renderer {
    fn blur_backdrop(&mut self, radius: f32) {
        let radius = blur::radius(radius);
        if radius > 0.0 {
            let (layer, transformation) = self.layers.barrier();
            layer.backdrop_blur = Some(radius * transformation.scale_factor().abs());
        }
    }

    fn blur_statistics(&self) -> renderer::BlurStatistics {
        #[cfg(feature = "image")]
        let image = self.engine.raster_pipeline.blur_statistics();
        #[cfg(not(feature = "image"))]
        let image = renderer::BlurStatistics::default();
        renderer::BlurStatistics {
            scene_hits: self.scene_blur.hits,
            scene_misses: self.scene_blur.misses,
            retained_bytes: self.scene_blur.retained_bytes() + image.retained_bytes,
            ..image
        }
    }

    fn start_layer(&mut self, bounds: Rectangle) {
        self.layers.push_clip(bounds);
    }

    fn end_layer(&mut self) {
        self.layers.pop_clip();
    }

    fn start_transformation(&mut self, transformation: Transformation) {
        self.layers.push_transformation(transformation);
    }

    fn end_transformation(&mut self) {
        self.layers.pop_transformation();
    }

    fn fill_quad(&mut self, quad: renderer::Quad, background: impl Into<Background>) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_quad(quad, background.into(), transformation);
    }

    fn allocate_image(
        &self,
        _handle: &core::image::Handle,
        callback: impl FnOnce(Result<core::image::Allocation, core::image::Error>) + Send + 'static,
    ) {
        #[cfg(feature = "image")]
        #[allow(unsafe_code)]
        // TODO: Concurrency
        callback(self.engine.raster_pipeline.load(_handle));

        #[cfg(not(feature = "image"))]
        callback(Err(core::image::Error::Unsupported));
    }

    fn hint(&mut self, _scale: renderer::Scale) {
        // TODO: No hinting supported
        // We'll replace `tiny-skia` with `vello_cpu` soon
    }

    fn scale(&self) -> Option<renderer::Scale> {
        None
    }

    fn reset(&mut self, new_bounds: Rectangle) {
        self.layers.reset(new_bounds);
    }

    fn settings(&self) -> renderer::Settings {
        self.settings
    }
}

impl core::text::Renderer for Renderer {
    type Paragraph = Paragraph;
    type Editor = Editor;

    const ICON_FONT: Font = Font::new("Iced-Icons");
    const CHECKMARK_ICON: char = '\u{f00c}';
    const ARROW_DOWN_ICON: char = '\u{e800}';
    const ICED_LOGO: char = '\u{e801}';
    const SCROLL_UP_ICON: char = '\u{e802}';
    const SCROLL_DOWN_ICON: char = '\u{e803}';
    const SCROLL_LEFT_ICON: char = '\u{e804}';
    const SCROLL_RIGHT_ICON: char = '\u{e805}';

    fn fill_paragraph(
        &mut self,
        text: &Self::Paragraph,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
    ) {
        let (layer, transformation) = self.layers.current_mut();

        layer.draw_paragraph(text, position, color, clip_bounds, transformation);
    }

    fn fill_editor(
        &mut self,
        editor: &Self::Editor,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
    ) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_editor(editor, position, color, clip_bounds, transformation);
    }

    fn fill_text(
        &mut self,
        text: core::Text,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
    ) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_text(text, position, color, clip_bounds, transformation);
    }
}

impl graphics::text::Renderer for Renderer {
    fn fill_raw(&mut self, raw: graphics::text::Raw) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_text_raw(raw, transformation);
    }
}

#[cfg(feature = "geometry")]
impl graphics::geometry::Renderer for Renderer {
    type Geometry = Geometry;
    type Frame = geometry::Frame;

    fn new_frame(&self, bounds: Rectangle) -> Self::Frame {
        geometry::Frame::new(bounds)
    }

    fn draw_geometry(&mut self, geometry: Self::Geometry) {
        let (layer, transformation) = self.layers.current_mut();

        match geometry {
            Geometry::Live {
                primitives,
                images,
                text,
                clip_bounds,
            } => {
                layer.draw_primitive_group(primitives, clip_bounds, transformation);

                for image in images {
                    layer.draw_image(image, transformation);
                }

                layer.draw_text_group(text, clip_bounds, transformation);
            }
            Geometry::Cache(cache) => {
                layer.draw_primitive_cache(cache.primitives, cache.clip_bounds, transformation);

                for image in cache.images.iter() {
                    layer.draw_image(image.clone(), transformation);
                }

                layer.draw_text_cache(cache.text, cache.clip_bounds, transformation);
            }
        }
    }
}

impl graphics::mesh::Renderer for Renderer {
    fn draw_mesh(&mut self, _mesh: graphics::Mesh) {
        log::warn!("iced_tiny_skia does not support drawing meshes");
    }

    fn draw_mesh_cache(&mut self, _cache: iced_graphics::mesh::Cache) {
        log::warn!("iced_tiny_skia does not support drawing meshes");
    }
}

#[cfg(feature = "image")]
impl core::image::Renderer for Renderer {
    type Handle = core::image::Handle;

    fn load_image(
        &self,
        handle: &Self::Handle,
    ) -> Result<core::image::Allocation, core::image::Error> {
        self.engine.raster_pipeline.load(handle)
    }

    fn measure_image(&self, handle: &Self::Handle) -> Option<crate::core::Size<u32>> {
        self.engine.raster_pipeline.dimensions(handle)
    }

    fn draw_image(&mut self, image: core::Image, bounds: Rectangle, clip_bounds: Rectangle) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_raster(image, bounds, clip_bounds, transformation);
    }
}

#[cfg(feature = "svg")]
impl core::svg::Renderer for Renderer {
    fn measure_svg(&self, handle: &core::svg::Handle) -> crate::core::Size<u32> {
        self.engine.vector_pipeline.viewport_dimensions(handle)
    }

    fn draw_svg(&mut self, svg: core::Svg, bounds: Rectangle, clip_bounds: Rectangle) {
        let (layer, transformation) = self.layers.current_mut();
        layer.draw_svg(svg, bounds, clip_bounds, transformation);
    }
}

impl compositor::Default for Renderer {
    type Compositor = window::Compositor;
}

impl renderer::Headless for Renderer {
    async fn new(settings: renderer::Settings, backend: Option<&str>) -> Option<Self> {
        if backend.is_some_and(|backend| !["tiny-skia", "tiny_skia", "software"].contains(&backend))
        {
            return None;
        }

        Some(Self::new(settings))
    }

    fn name(&self) -> String {
        "tiny-skia".to_owned()
    }

    fn screenshot(
        &mut self,
        size: Size<u32>,
        scale_factor: f32,
        background_color: Color,
    ) -> Vec<u8> {
        let viewport = Viewport::with_physical_size(
            size,
            renderer::Scale {
                window: 1.0,
                application: scale_factor,
            },
        );

        window::compositor::screenshot(self, &viewport, background_color)
    }
}

#[cfg(all(test, feature = "image"))]
mod corner_smoothing_tests {
    use super::*;
    use crate::core::Renderer as _;
    use crate::core::image::Renderer as _;

    fn viewport(scale: f32) -> Viewport {
        Viewport::with_physical_size(
            Size::new(720, 720),
            renderer::Scale {
                window: 1.0,
                application: scale,
            },
        )
    }

    fn scene(renderer: &mut Renderer, handle: &core::image::Handle, scale: f32, state: usize) {
        renderer.reset(Rectangle::with_size(viewport(scale).logical_size()));
        // A fixed layer keeps removals on the primitive-diff path, not the
        // whole-layer fallback. The fractional snapped/rotated bottom edge
        // extends past the old unsnapped expand(1) damage at scale one.
        renderer.with_layer(
            Rectangle::new(Point::new(1.0, 1.0), Size::new(170.0, 170.0)),
            |renderer| {
                renderer.with_translation(core::Vector::new(32.0, 32.0), |renderer| {
                    if state != 4 {
                        let image = core::Image::new(handle.clone())
                            .rotation(core::Radians(std::f32::consts::FRAC_PI_4))
                            .border_radius(core::border::top(24.0))
                            .border_smoothing(if state == 0 { 0.0 } else { 0.6 })
                            .snap(state != 3);
                        let image = if state >= 2 {
                            image.crop(Rectangle {
                                x: 3,
                                y: 2,
                                width: 7,
                                height: 9,
                            })
                        } else {
                            image
                        };
                        renderer.draw_image(
                            image,
                            Rectangle {
                                x: 0.499,
                                y: -0.499,
                                width: 98.002,
                                height: 101.0,
                            },
                            Rectangle {
                                x: -30.0,
                                y: -30.0,
                                width: 160.0,
                                height: 160.0,
                            },
                        );
                    }
                    // This independent image must not inherit the first image's
                    // display-frame mask, including on a partial damage redraw.
                    renderer.draw_image(
                        core::Image::new(handle.clone()).snap(false),
                        Rectangle {
                            x: 107.0,
                            y: 4.0,
                            width: 12.0,
                            height: 18.0,
                        },
                        Rectangle {
                            x: 107.0,
                            y: 4.0,
                            width: 12.0,
                            height: 18.0,
                        },
                    );
                });
            },
        );
        renderer.with_transformation(
            Transformation::translate(9.25, 140.5) * Transformation::scale(1.25),
            |renderer| {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            x: 0.25,
                            y: 0.5,
                            width: 28.0,
                            height: 20.0,
                        },
                        border: core::Border::default()
                            .rounded(8.0)
                            .smoothing(if state == 0 { 0.0 } else { 0.6 })
                            .width(1.0)
                            .color(Color::WHITE),
                        shadow: core::Shadow {
                            color: Color::BLACK,
                            offset: core::Vector::new(-3.0, 2.0),
                            blur_radius: 4.0,
                        },
                        snap: state != 3,
                    },
                    Color::from_rgb(0.2, 0.5, 0.8),
                );
                renderer.draw_image(
                    core::Image::new(handle.clone())
                        .border_radius(8.0)
                        .border_smoothing(0.6)
                        .snap(false),
                    Rectangle {
                        x: 33.25,
                        y: 0.5,
                        width: 28.0,
                        height: 20.0,
                    },
                    Rectangle {
                        x: 33.25,
                        y: 0.5,
                        width: 28.0,
                        height: 20.0,
                    },
                );
            },
        );
    }

    fn compare_transition(
        renderer: &mut Renderer,
        pixels: &mut tiny_skia::Pixmap,
        mask: &mut tiny_skia::Mask,
        previous: &[Layer],
        previous_scale: f32,
        scale: f32,
        label: &str,
    ) {
        let viewport = viewport(scale);
        let bounds = Rectangle::with_size(viewport.logical_size());
        let damage =
            window::compositor::frame_damage(previous, renderer.layers(), previous_scale, scale)
                .unwrap_or_else(|| vec![bounds]);
        let damage = graphics::damage::group(damage, bounds);
        renderer.draw(
            &mut pixels.as_mut(),
            mask,
            &viewport,
            &damage,
            Color::TRANSPARENT,
        );
        let mut reference = tiny_skia::Pixmap::new(720, 720).unwrap();
        renderer.draw(
            &mut reference.as_mut(),
            mask,
            &viewport,
            &[bounds],
            Color::TRANSPARENT,
        );
        assert_eq!(
            pixels.data(),
            reference.data(),
            "{label}, scale {previous_scale} -> {scale}"
        );
    }

    #[test]
    fn corner_smoothing_damage_matches_full_redraw() {
        let rgba = (0..16 * 16)
            .flat_map(|index| [(index % 16 * 16) as u8, (index / 16 * 16) as u8, 96, 255])
            .collect::<Vec<_>>();
        let handle = core::image::Handle::from_rgba(16, 16, rgba);
        for scale in [1.0, 2.5, 4.0] {
            let mut renderer = Renderer::new(renderer::Settings::default());
            let mut pixels = tiny_skia::Pixmap::new(720, 720).unwrap();
            let mut mask = tiny_skia::Mask::new(720, 720).unwrap();
            scene(&mut renderer, &handle, scale, 0);
            let bounds = Rectangle::with_size(viewport(scale).logical_size());
            renderer.draw(
                &mut pixels.as_mut(),
                &mut mask,
                &viewport(scale),
                &[bounds],
                Color::TRANSPARENT,
            );
            for (state, label) in [
                (1, "smoothing"),
                (2, "crop"),
                (3, "snap"),
                (0, "restore snapped image"),
                (4, "remove rotated image"),
            ] {
                let previous = renderer.layers().to_vec();
                scene(&mut renderer, &handle, scale, state);
                compare_transition(
                    &mut renderer,
                    &mut pixels,
                    &mut mask,
                    &previous,
                    scale,
                    scale,
                    label,
                );
            }
            let previous = renderer.layers().to_vec();
            let next_scale = if scale == 4.0 { 2.5 } else { 4.0 };
            scene(&mut renderer, &handle, next_scale, 1);
            compare_transition(
                &mut renderer,
                &mut pixels,
                &mut mask,
                &previous,
                scale,
                next_scale,
                "viewport scale",
            );
        }
    }

    #[test]
    fn corner_smoothing_sublogical_damage_survives_grouping() {
        let handle = core::image::Handle::from_rgba(1, 1, vec![255; 4]);
        for scale in [2.5, 4.0] {
            let mut renderer = Renderer::new(renderer::Settings::default());
            let viewport = viewport(scale);
            let bounds = Rectangle::with_size(viewport.logical_size());
            renderer.reset(bounds);
            let tiny = Rectangle {
                x: 30.125,
                y: 40.125,
                width: 0.1,
                height: 0.1,
            };
            renderer.fill_quad(
                renderer::Quad {
                    bounds: tiny,
                    snap: false,
                    ..Default::default()
                },
                Color::WHITE,
            );
            renderer.draw_image(
                core::Image::new(handle.clone()).snap(false),
                tiny + core::Vector::new(10.0, 0.0),
                tiny + core::Vector::new(10.0, 0.0),
            );
            let mut pixels = tiny_skia::Pixmap::new(720, 720).unwrap();
            let mut mask = tiny_skia::Mask::new(720, 720).unwrap();
            renderer.draw(
                &mut pixels.as_mut(),
                &mut mask,
                &viewport,
                &[bounds],
                Color::TRANSPARENT,
            );
            let previous = renderer.layers().to_vec();
            renderer.reset(bounds);
            compare_transition(
                &mut renderer,
                &mut pixels,
                &mut mask,
                &previous,
                scale,
                scale,
                "remove sublogical primitives",
            );
        }
    }

    #[test]
    fn corner_smoothing_layer_transforms_preserve_image_quad_coverage() {
        let handle = core::image::Handle::from_rgba(1, 1, vec![255; 4]);
        let radius = core::border::Radius {
            top_left: 12.0,
            top_right: 5.0,
            bottom_right: 0.0,
            bottom_left: 18.0,
        };
        let bounds = Rectangle {
            x: 0.25,
            y: 0.5,
            width: 40.0,
            height: 30.0,
        };
        for scale in [1.0, 2.5, 4.0] {
            for snap in [false, true] {
                let mut renderer = Renderer::new(renderer::Settings::default());
                let viewport = viewport(scale);
                let surface = Rectangle::with_size(viewport.logical_size());
                let mut mask = tiny_skia::Mask::new(720, 720).unwrap();
                let mut quad_pixels = tiny_skia::Pixmap::new(720, 720).unwrap();
                let mut image_pixels = tiny_skia::Pixmap::new(720, 720).unwrap();
                let transform = Transformation::translate(20.25, 18.5) * Transformation::scale(1.5);
                renderer.reset(surface);
                renderer.with_transformation(transform, |renderer| {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds,
                            border: core::Border::default().rounded(radius).smoothing(0.6),
                            snap,
                            ..Default::default()
                        },
                        Color::WHITE,
                    );
                });
                renderer.draw(
                    &mut quad_pixels.as_mut(),
                    &mut mask,
                    &viewport,
                    &[surface],
                    Color::TRANSPARENT,
                );
                renderer.reset(surface);
                renderer.with_transformation(transform, |renderer| {
                    renderer.draw_image(
                        core::Image::new(handle.clone())
                            .border_radius(radius)
                            .border_smoothing(0.6)
                            .snap(snap),
                        bounds,
                        bounds,
                    );
                });
                renderer.draw(
                    &mut image_pixels.as_mut(),
                    &mut mask,
                    &viewport,
                    &[surface],
                    Color::TRANSPARENT,
                );
                for (quad, image) in quad_pixels.pixels().iter().zip(image_pixels.pixels()) {
                    assert!(
                        quad.alpha().abs_diff(image.alpha()) <= 1,
                        "scale {scale}, snap {snap}: quad {} image {}",
                        quad.alpha(),
                        image.alpha()
                    );
                }
            }
        }
    }
}

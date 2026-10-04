use crate::core::{self, Background, Color, Point, Rectangle, Svg, Transformation, renderer};
use crate::graphics;
use crate::graphics::Mesh;
use crate::graphics::color;
use crate::graphics::layer;
use crate::graphics::mesh;
use crate::graphics::text::{Editor, Paragraph};
use crate::image::{self, Image};
use crate::primitive::{self, Primitive};
use crate::quad::{self, Quad};
use crate::text::{self, Text};
use crate::triangle;

pub type Stack = layer::Stack<Layer>;

#[derive(Debug)]
pub struct Layer {
    pub bounds: Rectangle,
    pub backdrop_blur: Option<renderer::Backdrop>,
    pub quads: quad::Batch,
    pub triangles: triangle::Batch,
    pub primitives: primitive::Batch,
    pub images: image::Batch,
    pub text: text::Batch,
    pending_meshes: Vec<Mesh>,
    pending_text: Vec<Text>,
}

impl Layer {
    pub fn is_empty(&self) -> bool {
        self.backdrop_blur.is_none()
            && self.quads.is_empty()
            && self.triangles.is_empty()
            && self.primitives.is_empty()
            && self.images.is_empty()
            && self.text.is_empty()
            && self.pending_meshes.is_empty()
            && self.pending_text.is_empty()
    }

    pub fn draw_quad(
        &mut self,
        quad: renderer::Quad,
        background: Background,
        transformation: Transformation,
    ) {
        let bounds = quad.bounds * transformation;
        let scale = transformation.scale_factor();
        if !scale.is_finite() || scale <= 0.0 || !valid_shape_bounds(bounds) {
            return;
        }
        let background = match background {
            Background::Color(color) => Background::Color(finite_color(color)),
            Background::Gradient(core::Gradient::Linear(mut linear)) => {
                linear.reference_bounds = linear
                    .reference_bounds
                    .map(|bounds| bounds * transformation);
                if !linear.angle.0.is_finite() {
                    linear.angle.0 = 0.0;
                }
                for stop in linear.stops.iter_mut().flatten() {
                    stop.color = finite_color(stop.color);
                    stop.offset = graphics::shape::normalize_smoothing(stop.offset);
                }
                Background::Gradient(core::Gradient::Linear(linear))
            }
        };

        let quad = Quad {
            position: [bounds.x, bounds.y],
            size: [bounds.width, bounds.height],
            border_color: color::pack(finite_color(quad.border.color)),
            border_radius: <[f32; 4]>::from(quad.border.radius)
                .map(|radius| scale_length(radius, scale)),
            border_width: scale_length(quad.border.width, scale),
            shadow_color: color::pack(finite_color(quad.shadow.color)),
            shadow_offset: [quad.shadow.offset.x, quad.shadow.offset.y]
                .map(|offset| scale_finite(offset, scale)),
            shadow_blur_radius: scale_finite(quad.shadow.blur_radius, scale).max(0.0),
            snap: quad.snap as u32,
            smoothing: graphics::shape::normalize_smoothing(quad.border.smoothing),
        };

        self.quads.add(quad, &background);
    }

    pub fn draw_paragraph(
        &mut self,
        paragraph: &Paragraph,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
        transformation: Transformation,
    ) {
        let paragraph = Text::Paragraph {
            paragraph: paragraph.downgrade(),
            position,
            color,
            clip_bounds,
            transformation,
        };

        self.pending_text.push(paragraph);
    }

    pub fn draw_editor(
        &mut self,
        editor: &Editor,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
        transformation: Transformation,
    ) {
        let editor = Text::Editor {
            editor: editor.downgrade(),
            position,
            color,
            clip_bounds,
            transformation,
        };

        self.pending_text.push(editor);
    }

    pub fn draw_text(
        &mut self,
        text: crate::core::Text,
        position: Point,
        color: Color,
        clip_bounds: Rectangle,
        transformation: Transformation,
    ) {
        let text = Text::Cached {
            content: text.content,
            bounds: Rectangle::new(position, text.bounds) * transformation,
            color,
            size: text.size * transformation.scale_factor(),
            line_height: text.line_height.to_absolute(text.size) * transformation.scale_factor(),
            font: text.font,
            align_x: text.align_x,
            align_y: text.align_y,
            shaping: text.shaping,
            wrapping: text.wrapping,
            ellipsis: text.ellipsis,
            clip_bounds: clip_bounds * transformation,
        };

        self.pending_text.push(text);
    }

    pub fn draw_text_raw(&mut self, raw: graphics::text::Raw, transformation: Transformation) {
        let raw = Text::Raw {
            raw,
            transformation,
        };

        self.pending_text.push(raw);
    }

    pub fn draw_image(&mut self, image: Image, transformation: Transformation) {
        match image {
            Image::Raster {
                image,
                bounds,
                clip_bounds,
            } => {
                self.draw_raster_transformed(image, bounds, clip_bounds, transformation);
            }
            Image::Vector {
                svg,
                bounds,
                clip_bounds,
            } => {
                self.draw_svg(svg, bounds, clip_bounds, transformation);
            }
        }
    }

    pub fn draw_raster(
        &mut self,
        mut image: core::Image,
        bounds: Rectangle,
        clip_bounds: Rectangle,
        transformation: Transformation,
    ) {
        image.blur = image.blur.normalized();
        self.draw_raster_transformed(image, bounds, clip_bounds, transformation);
    }

    fn draw_raster_transformed(
        &mut self,
        image: core::Image,
        bounds: Rectangle,
        clip_bounds: Rectangle,
        transformation: Transformation,
    ) {
        let scale = transformation.scale_factor();
        let bounds = bounds * transformation;
        let clip_bounds = clip_bounds * transformation;
        if !scale.is_finite()
            || scale <= 0.0
            || !valid_shape_bounds(bounds)
            || !valid_shape_bounds(clip_bounds)
        {
            return;
        }
        let [top_left, top_right, bottom_right, bottom_left] =
            <[f32; 4]>::from(image.border_radius).map(|radius| scale_length(radius, scale));
        let image = Image::Raster {
            image: core::Image {
                blur: image.blur.scaled(scale),
                border_radius: core::border::Radius {
                    top_left,
                    top_right,
                    bottom_right,
                    bottom_left,
                },
                border_smoothing: graphics::shape::normalize_smoothing(image.border_smoothing),
                ..image
            },
            bounds,
            clip_bounds,
        };

        self.images.push(image);
    }

    pub fn draw_svg(
        &mut self,
        svg: Svg,
        bounds: Rectangle,
        clip_bounds: Rectangle,
        transformation: Transformation,
    ) {
        let svg = Image::Vector {
            svg,
            bounds: bounds * transformation,
            clip_bounds: clip_bounds * transformation,
        };

        self.images.push(svg);
    }

    pub fn draw_mesh(&mut self, mut mesh: Mesh, transformation: Transformation) {
        match &mut mesh {
            Mesh::Solid {
                transformation: local_transformation,
                ..
            }
            | Mesh::Gradient {
                transformation: local_transformation,
                ..
            } => {
                *local_transformation = *local_transformation * transformation;
            }
        }

        self.pending_meshes.push(mesh);
    }

    pub fn draw_mesh_group(&mut self, meshes: Vec<Mesh>, transformation: Transformation) {
        self.flush_meshes();

        self.triangles.push(triangle::Item::Group {
            meshes,
            transformation,
        });
    }

    pub fn draw_mesh_cache(&mut self, cache: mesh::Cache, transformation: Transformation) {
        self.flush_meshes();

        self.triangles.push(triangle::Item::Cached {
            cache,
            transformation,
        });
    }

    pub fn draw_text_group(&mut self, text: Vec<Text>, transformation: Transformation) {
        self.flush_text();

        self.text.push(text::Item::Group {
            text,
            transformation,
        });
    }

    pub fn draw_text_cache(&mut self, cache: text::Cache, transformation: Transformation) {
        self.flush_text();

        self.text.push(text::Item::Cached {
            cache,
            transformation,
        });
    }

    pub fn draw_primitive(
        &mut self,
        bounds: Rectangle,
        primitive: impl Primitive,
        transformation: Transformation,
    ) {
        let bounds = bounds * transformation;

        self.primitives
            .push(primitive::Instance::new(bounds, primitive));
    }

    #[allow(clippy::drain_collect)] // We want to reuse `Layer` capacity
    fn flush_meshes(&mut self) {
        if !self.pending_meshes.is_empty() {
            self.triangles.push(triangle::Item::Group {
                transformation: Transformation::IDENTITY,
                meshes: self.pending_meshes.drain(..).collect(),
            });
        }
    }

    #[allow(clippy::drain_collect)] // We want to reuse `Layer` capacity
    fn flush_text(&mut self) {
        if !self.pending_text.is_empty() {
            self.text.push(text::Item::Group {
                transformation: Transformation::IDENTITY,
                text: self.pending_text.drain(..).collect(),
            });
        }
    }
}

impl graphics::Layer for Layer {
    fn is_barrier(&self) -> bool {
        self.backdrop_blur.is_some()
    }
    fn with_bounds(bounds: Rectangle) -> Self {
        Self {
            bounds,
            ..Self::default()
        }
    }

    fn bounds(&self) -> Rectangle {
        self.bounds
    }

    fn flush(&mut self) {
        self.flush_meshes();
        self.flush_text();
    }

    fn resize(&mut self, bounds: Rectangle) {
        self.bounds = bounds;
    }

    fn reset(&mut self) {
        self.bounds = Rectangle::INFINITE;
        self.backdrop_blur = None;

        self.quads.clear();
        self.triangles.clear();
        self.primitives.clear();
        self.text.clear();
        self.images.clear();
        self.pending_meshes.clear();
        self.pending_text.clear();
    }

    fn start(&self) -> usize {
        if !self.quads.is_empty() {
            return 1;
        }

        if !self.triangles.is_empty() {
            return 2;
        }

        if !self.primitives.is_empty() {
            return 3;
        }

        if !self.images.is_empty() {
            return 4;
        }

        if !self.text.is_empty() {
            return 5;
        }

        usize::MAX
    }

    fn end(&self) -> usize {
        if !self.text.is_empty() {
            return 5;
        }

        if !self.images.is_empty() {
            return 4;
        }

        if !self.primitives.is_empty() {
            return 3;
        }

        if !self.triangles.is_empty() {
            return 2;
        }

        if !self.quads.is_empty() {
            return 1;
        }

        0
    }

    fn merge(&mut self, layer: &mut Self) {
        self.quads.append(&mut layer.quads);
        self.triangles.append(&mut layer.triangles);
        self.primitives.append(&mut layer.primitives);
        self.images.append(&mut layer.images);
        self.text.append(&mut layer.text);
    }
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            bounds: Rectangle::INFINITE,
            backdrop_blur: None,
            quads: quad::Batch::default(),
            triangles: triangle::Batch::default(),
            primitives: primitive::Batch::default(),
            text: text::Batch::default(),
            images: image::Batch::default(),
            pending_meshes: Vec::new(),
            pending_text: Vec::new(),
        }
    }
}

fn valid_shape_bounds(bounds: Rectangle) -> bool {
    bounds.x.is_finite()
        && bounds.y.is_finite()
        && bounds.width.is_finite()
        && bounds.height.is_finite()
        && bounds.width > 0.0
        && bounds.height > 0.0
        && (bounds.x + bounds.width).is_finite()
        && (bounds.y + bounds.height).is_finite()
}

// Keep saturated lengths saturated through a layer transform; only the final
// snapped physical bounds are allowed to cap a radius or border width.
fn scale_length(value: f32, scale: f32) -> f32 {
    (f64::from(graphics::shape::normalize_length(value)) * f64::from(scale))
        .min(f64::from(f32::MAX)) as f32
}

fn scale_finite(value: f32, scale: f32) -> f32 {
    if value.is_finite() {
        (f64::from(value) * f64::from(scale)).clamp(-f64::from(f32::MAX), f64::from(f32::MAX))
            as f32
    } else {
        0.0
    }
}

fn finite_color(color: Color) -> Color {
    Color {
        r: graphics::shape::normalize_smoothing(color.r),
        g: graphics::shape::normalize_smoothing(color.g),
        b: graphics::shape::normalize_smoothing(color.b),
        a: graphics::shape::normalize_smoothing(color.a),
    }
}

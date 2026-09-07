//! Images display raster graphics in different formats (PNG, JPG, etc.).
//!
//! # Example
//! ```no_run
//! # mod iced { pub mod widget { pub use iced_widget::*; } }
//! # pub type State = ();
//! # pub type Element<'a, Message> = iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>;
//! use iced::widget::image;
//!
//! enum Message {
//!     // ...
//! }
//!
//! fn view(state: &State) -> Element<'_, Message> {
//!     image("ferris.png").into()
//! }
//! ```
//! <img src="https://github.com/iced-rs/iced/blob/9712b319bb7a32848001b96bd84977430f14b623/examples/resources/ferris.png?raw=true" width="300">
pub mod viewer;
pub use viewer::Viewer;

use crate::core::border;
use crate::core::image;
use crate::core::layout;
use crate::core::mouse;
use crate::core::renderer;
use crate::core::widget::Tree;
use crate::core::{
    ContentFit, Element, Layout, Length, Point, Rectangle, Rotation, Size, Vector, Widget,
};

pub use image::{FilterMethod, Handle};

/// Creates a new [`Viewer`] with the given image `Handle`.
pub fn viewer<Handle>(handle: Handle) -> Viewer<Handle> {
    Viewer::new(handle)
}

/// A frame that displays an image while keeping aspect ratio.
///
/// # Example
/// ```no_run
/// # mod iced { pub mod widget { pub use iced_widget::*; } }
/// # pub type State = ();
/// # pub type Element<'a, Message> = iced_widget::core::Element<'a, Message, iced_widget::Theme, iced_widget::Renderer>;
/// use iced::widget::image;
///
/// enum Message {
///     // ...
/// }
///
/// fn view(state: &State) -> Element<'_, Message> {
///     image("ferris.png").into()
/// }
/// ```
/// <img src="https://github.com/iced-rs/iced/blob/9712b319bb7a32848001b96bd84977430f14b623/examples/resources/ferris.png?raw=true" width="300">
pub struct Image<Handle = image::Handle> {
    handle: Handle,
    width: Length,
    height: Length,
    crop: Option<Rectangle<u32>>,
    border_radius: border::Radius,
    border_smoothing: f32,
    content_fit: ContentFit,
    filter_method: FilterMethod,
    rotation: Rotation,
    opacity: f32,
    scale: f32,
    expand: bool,
    snap: bool,
}

impl<Handle> Image<Handle> {
    /// Creates a new [`Image`] with the given path.
    pub fn new(handle: impl Into<Handle>) -> Self {
        Image {
            handle: handle.into(),
            width: Length::Shrink,
            height: Length::Shrink,
            crop: None,
            border_radius: border::Radius::default(),
            border_smoothing: 0.0,
            content_fit: ContentFit::default(),
            filter_method: FilterMethod::default(),
            rotation: Rotation::default(),
            opacity: 1.0,
            scale: 1.0,
            expand: false,
            snap: renderer::CRISP,
        }
    }

    /// Sets the width of the [`Image`] boundaries.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }

    /// Sets the height of the [`Image`] boundaries.
    pub fn height(mut self, height: impl Into<Length>) -> Self {
        self.height = height.into();
        self
    }

    /// Sets whether the [`Image`] should try to fill as much space
    /// available as possible while keeping aspect ratio and without
    /// allocating extra space in any axis with a [`Length::Shrink`]
    /// sizing strategy.
    ///
    /// This is similar to using [`Length::Fill`] for both the
    /// [`width`](Self::width) and the [`height`](Self::height),
    /// but without the downside of blank space.
    pub fn expand(mut self, expand: bool) -> Self {
        self.expand = expand;
        self
    }

    /// Sets the [`ContentFit`] of the [`Image`].
    ///
    /// Defaults to [`ContentFit::Contain`]
    pub fn content_fit(mut self, content_fit: ContentFit) -> Self {
        self.content_fit = content_fit;
        self
    }

    /// Sets the [`FilterMethod`] of the [`Image`].
    pub fn filter_method(mut self, filter_method: FilterMethod) -> Self {
        self.filter_method = filter_method;
        self
    }

    /// Applies the given [`Rotation`] to the [`Image`].
    pub fn rotation(mut self, rotation: impl Into<Rotation>) -> Self {
        self.rotation = rotation.into();
        self
    }

    /// Sets the opacity of the [`Image`].
    ///
    /// It should be in the [0.0, 1.0] range—`0.0` meaning completely transparent,
    /// and `1.0` meaning completely opaque.
    pub fn opacity(mut self, opacity: impl Into<f32>) -> Self {
        self.opacity = opacity.into();
        self
    }

    /// Sets the scale of the [`Image`].
    ///
    /// The region of the [`Image`] drawn will be scaled from the center by the given scale factor.
    /// This can be useful to create certain effects and animations, like smooth zoom in / out.
    pub fn scale(mut self, scale: impl Into<f32>) -> Self {
        self.scale = scale.into();
        self
    }

    /// Crops the [`Image`] to the given region described by the [`Rectangle`] in absolute
    /// coordinates.
    ///
    /// Cropping is done before applying any transformation or [`ContentFit`]. In practice,
    /// this means that cropping an [`Image`] with this method should produce the same result
    /// as cropping it externally (e.g. with an image editor) and creating a new [`Handle`]
    /// for the cropped version.
    ///
    /// Rendering reuses the existing image allocation and samples only the selected
    /// source region, without allocating a cropped copy.
    ///
    /// Rendering intersects the requested `region` with the image dimensions.
    /// An empty intersection draws nothing. Layout still uses the requested crop
    /// size, capped to the image size, so partially out-of-bounds requests may
    /// reserve different space than an externally cropped image.
    pub fn crop(mut self, region: Rectangle<u32>) -> Self {
        self.crop = Some(region);
        self
    }

    /// Sets the [`border::Radius`] of the [`Image`].
    ///
    /// Currently, it will only be applied around the rectangular bounding box
    /// of the [`Image`].
    pub fn border_radius(mut self, border_radius: impl Into<border::Radius>) -> Self {
        self.border_radius = border_radius.into();
        self
    }

    /// Sets the corner smoothing of the [`Image`] bounding box.
    ///
    /// Values are clamped to the range `0.0..=1.0`. The default is `0.0`,
    /// which produces circular corners.
    pub fn border_smoothing(mut self, border_smoothing: f32) -> Self {
        self.border_smoothing = border_smoothing;
        self
    }

    /// Sets whether the [`Image`] content and display bounds snap to physical pixels.
    ///
    /// Defaults to [`renderer::CRISP`].
    pub fn snap(mut self, snap: bool) -> Self {
        self.snap = snap;
        self
    }
}

/// Computes the layout of an [`Image`].
pub fn layout<Renderer, Handle>(
    renderer: &Renderer,
    limits: &layout::Limits,
    handle: &Handle,
    width: Length,
    height: Length,
    region: Option<Rectangle<u32>>,
    content_fit: ContentFit,
    rotation: Rotation,
    expand: bool,
) -> layout::Node
where
    Renderer: image::Renderer<Handle = Handle>,
{
    // The raw w/h of the underlying image
    let image_size = crop(renderer.measure_image(handle).unwrap_or_default(), region);

    // The rotated size of the image
    let rotated_size = rotation.apply(image_size);

    // The size to be available to the widget prior to `Shrink`ing
    let bounds = if expand {
        limits.width(width).height(height).max()
    } else {
        limits.resolve(width, height, rotated_size)
    };

    // The uncropped size of the image when fit to the bounds above
    let full_size = content_fit.fit(rotated_size, bounds);

    // Shrink the widget to fit the resized image, if requested
    let final_size = Size {
        width: match width {
            Length::Shrink => f32::min(bounds.width, full_size.width),
            _ => bounds.width,
        },
        height: match height {
            Length::Shrink => f32::min(bounds.height, full_size.height),
            _ => bounds.height,
        },
    };

    layout::Node::new(final_size)
}

fn drawing_bounds<Renderer, Handle>(
    renderer: &Renderer,
    bounds: Rectangle,
    handle: &Handle,
    region: Option<Rectangle<u32>>,
    content_fit: ContentFit,
    rotation: Rotation,
    scale: f32,
) -> Option<Rectangle>
where
    Renderer: image::Renderer<Handle = Handle>,
{
    let original_size = renderer.measure_image(handle)?;
    let region = image::crop_bounds(original_size, region)?;

    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }

    let image_size = Size::new(region.width as f32, region.height as f32);
    let rotated_size = rotation.apply(image_size);
    let adjusted_fit = content_fit.fit(rotated_size, bounds.size());

    let fit_scale = Vector::new(
        adjusted_fit.width / rotated_size.width,
        adjusted_fit.height / rotated_size.height,
    );

    let final_size = image_size * fit_scale * scale;

    let position = match content_fit {
        ContentFit::None => Point::new(
            bounds.x + (rotated_size.width - adjusted_fit.width) / 2.0,
            bounds.y + (rotated_size.height - adjusted_fit.height) / 2.0,
        ),
        _ => Point::new(
            bounds.center_x() - final_size.width / 2.0,
            bounds.center_y() - final_size.height / 2.0,
        ),
    };

    let bounds = Rectangle::new(position, final_size);

    (bounds.x.is_finite()
        && bounds.y.is_finite()
        && bounds.width.is_finite()
        && bounds.height.is_finite()
        && bounds.width > 0.0
        && bounds.height > 0.0)
        .then_some(bounds)
}

fn crop(size: Size<u32>, region: Option<Rectangle<u32>>) -> Size<f32> {
    if let Some(region) = region {
        Size::new(
            region.width.min(size.width) as f32,
            region.height.min(size.height) as f32,
        )
    } else {
        Size::new(size.width as f32, size.height as f32)
    }
}

/// Draws an [`Image`]
pub fn draw<Renderer, Handle>(
    renderer: &mut Renderer,
    layout: Layout<'_>,
    handle: &Handle,
    crop: Option<Rectangle<u32>>,
    border_radius: border::Radius,
    border_smoothing: f32,
    content_fit: ContentFit,
    filter_method: FilterMethod,
    rotation: Rotation,
    opacity: f32,
    scale: f32,
    snap: bool,
) where
    Renderer: image::Renderer<Handle = Handle>,
    Handle: Clone,
{
    let bounds = layout.bounds();
    let Some(drawing_bounds) =
        drawing_bounds(renderer, bounds, handle, crop, content_fit, rotation, scale)
    else {
        return;
    };

    renderer.draw_image(
        image::Image {
            handle: handle.clone(),
            border_radius,
            border_smoothing,
            snap,
            crop,
            filter_method,
            rotation: rotation.radians(),
            opacity,
        },
        drawing_bounds,
        bounds,
    );
}

impl<Message, Theme, Renderer, Handle> Widget<Message, Theme, Renderer> for Image<Handle>
where
    Renderer: image::Renderer<Handle = Handle>,
    Handle: Clone,
{
    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: self.height,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout(
            renderer,
            limits,
            &self.handle,
            self.width,
            self.height,
            self.crop,
            self.content_fit,
            self.rotation,
            self.expand,
        )
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        draw(
            renderer,
            layout,
            &self.handle,
            self.crop,
            self.border_radius,
            self.border_smoothing,
            self.content_fit,
            self.filter_method,
            self.rotation,
            self.opacity,
            self.scale,
            self.snap,
        );
    }
}

impl<'a, Message, Theme, Renderer, Handle> From<Image<Handle>>
    for Element<'a, Message, Theme, Renderer>
where
    Renderer: image::Renderer<Handle = Handle>,
    Handle: Clone + 'a,
{
    fn from(image: Image<Handle>) -> Element<'a, Message, Theme, Renderer> {
        Element::new(image)
    }
}

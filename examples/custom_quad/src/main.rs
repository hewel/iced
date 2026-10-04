//! Native quad and raster-image corner smoothing acceptance surface.
use iced::border;
use iced::widget::{
    button, column, container, image, pick_list, row, scrollable, slider, space, stack, text,
    toggler,
};
use iced::{
    Background, Border, Color, ContentFit, Fill, Padding, Radians, Rectangle, Shadow, Subscription,
    Task, Theme, Vector, Widget,
};

pub fn main() -> iced::Result {
    iced::application(Example::default, Example::update, Example::view)
        .title("Native corner smoothing")
        .theme(|state: &Example| {
            if state.dark {
                Theme::Dark
            } else {
                Theme::Light
            }
        })
        .style(|state, _| iced::theme::Style {
            background_color: state.colors().0,
            text_color: if state.dark {
                Color::WHITE
            } else {
                Color::BLACK
            },
        })
        .scale_factor(|state| state.dpi as f32 / 100.0)
        .subscription(Example::subscription)
        .window_size((1160, 860))
        .run()
}

struct Example {
    radius: border::Radius,
    smoothing: f32,
    border_width: f32,
    shadow: Shadow,
    snap: bool,
    dark: bool,
    gradient: bool,
    dpi: u16,
    phase: u16,
    crop: bool,
    fit: ContentFit,
    rotation: u16,
    image_scale: u16,
    opacity: u16,
    poster: image::Handle,
    fragmented: image::Handle,
    capture_status: String,
}

#[derive(Debug, Clone)]
enum Message {
    Radius(usize, f32),
    Smoothing(f32),
    BorderWidth(f32),
    ShadowX(f32),
    ShadowY(f32),
    Blur(f32),
    Snap(bool),
    Dark(bool),
    Gradient(bool),
    Dpi(u16),
    Phase(u16),
    Crop(bool),
    Fit(ContentFit),
    Rotation(u16),
    ImageScale(u16),
    Opacity(u16),
    Capture,
    Captured(iced::window::screenshot::Screenshot),
    Saved(Result<String, String>),
}

impl Default for Example {
    fn default() -> Self {
        let generate = |width: u32, height: u32| {
            let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
            for y in 0..height {
                for x in 0..width {
                    rgba.extend_from_slice(&[
                        (x % 251) as u8,
                        (y % 251) as u8,
                        ((x / 7 + y / 3) % 251) as u8,
                        if (x / 16 + y / 16) % 9 == 0 { 100 } else { 255 },
                    ]);
                }
            }
            image::Handle::from_rgba(width, height, rgba)
        };
        Self {
            radius: border::radius(24),
            smoothing: 0.6,
            border_width: 1.0,
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.2),
                offset: Vector::new(0.0, 8.0),
                blur_radius: 16.0,
            },
            snap: false,
            dark: false,
            gradient: false,
            dpi: 100,
            phase: 0,
            crop: true,
            fit: ContentFit::Cover,
            rotation: 0,
            image_scale: 100,
            opacity: 100,
            poster: generate(256, 320),
            fragmented: generate(4097, 97),
            capture_status: "F5 captures only this window to PNG".into(),
        }
    }
}

impl Example {
    fn colors(&self) -> (Color, Color, Color) {
        if self.dark {
            (
                Color::from_rgb8(23, 24, 28),
                Color::from_rgb8(37, 39, 45),
                Color::from_rgb8(68, 71, 80),
            )
        } else {
            (
                Color::from_rgb8(248, 248, 250),
                Color::WHITE,
                Color::from_rgb8(216, 218, 223),
            )
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Radius(corner, value) => match corner {
                0 => self.radius.top_left = value,
                1 => self.radius.top_right = value,
                2 => self.radius.bottom_right = value,
                _ => self.radius.bottom_left = value,
            },
            Message::Smoothing(value) => self.smoothing = value,
            Message::BorderWidth(value) => self.border_width = value,
            Message::ShadowX(value) => self.shadow.offset.x = value,
            Message::ShadowY(value) => self.shadow.offset.y = value,
            Message::Blur(value) => self.shadow.blur_radius = value,
            Message::Snap(value) => self.snap = value,
            Message::Dark(value) => self.dark = value,
            Message::Gradient(value) => self.gradient = value,
            Message::Dpi(value) => self.dpi = value,
            Message::Phase(value) => self.phase = value,
            Message::Crop(value) => self.crop = value,
            Message::Fit(value) => self.fit = value,
            Message::Rotation(value) => self.rotation = value,
            Message::ImageScale(value) => self.image_scale = value,
            Message::Opacity(value) => self.opacity = value,
            Message::Capture => {
                self.capture_status = "Capturing window…".into();
                return iced::window::latest()
                    .and_then(iced::window::screenshot)
                    .map(Message::Captured);
            }
            Message::Captured(screenshot) => {
                let backend = std::env::var("ICED_BACKEND").unwrap_or_else(|_| "auto".into());
                let path = format!(
                    "corner-{backend}-{}-{}-snap-{}-s-{:.2}.png",
                    if self.dark { "dark" } else { "light" },
                    self.dpi,
                    self.snap,
                    self.smoothing
                );
                return Task::perform(
                    async move {
                        ::image::save_buffer(
                            &path,
                            &screenshot.rgba,
                            screenshot.size.width,
                            screenshot.size.height,
                            ::image::ColorType::Rgba8,
                        )
                        .map(|()| path)
                        .map_err(|error| error.to_string())
                    },
                    Message::Saved,
                );
            }
            Message::Saved(result) => {
                self.capture_status = match result {
                    Ok(path) => format!("Saved {path}"),
                    Err(error) => format!("Capture failed: {error}"),
                }
            }
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Message> {
        iced::keyboard::listen().filter_map(|event| match event {
            iced::keyboard::Event::KeyPressed {
                modified_key: iced::keyboard::Key::Named(iced::keyboard::key::Named::F5),
                ..
            } => Some(Message::Capture),
            _ => None,
        })
    }

    fn poster(&self, fragmented: bool, top_only: bool) -> impl Widget<Message> {
        let radius = if top_only {
            border::top(24)
        } else {
            self.radius
        };
        let handle = if fragmented {
            &self.fragmented
        } else {
            &self.poster
        };
        let mut picture = image(handle)
            .width(220)
            .height(170)
            .content_fit(self.fit)
            .border_radius(radius)
            .border_smoothing(self.smoothing)
            .snap(self.snap)
            .rotation(Radians((self.rotation as f32).to_radians()))
            .scale(self.image_scale as f32 / 100.0)
            .opacity(self.opacity as f32 / 100.0);
        if self.crop {
            picture = picture.crop(if fragmented {
                Rectangle {
                    x: 2010,
                    y: 5,
                    width: 80,
                    height: 67,
                }
            } else {
                Rectangle {
                    x: 40,
                    y: 20,
                    width: 160,
                    height: 220,
                }
            });
        }
        let border = Border::default()
            .rounded(radius)
            .smoothing(self.smoothing)
            .width(1)
            .color(self.colors().2);
        let snap = self.snap;
        // Stack establishes the existing image-before-overlay layer ordering.
        let selected = stack![
            picture,
            container(space().width(Fill).height(Fill))
                .width(Fill)
                .height(Fill)
                .style(move |_| container::Style {
                    border,
                    snap,
                    ..Default::default()
                })
        ];
        column![
            selected,
            text(if fragmented {
                "4097×97 atlas / selected crop"
            } else if top_only {
                "Top 24 / square bottom card"
            } else {
                "Independent-corner poster"
            })
            .size(13)
        ]
        .spacing(8)
    }

    fn view(&self) -> impl Widget<Message> {
        let (_, panel, border_color) = self.colors();
        let fill = if self.gradient {
            Background::Gradient(
                iced::gradient::Linear::new(Radians(0.8))
                    .add_stop(0.0, panel)
                    .add_stop(1.0, self.colors().0)
                    .into(),
            )
        } else {
            Background::Color(panel)
        };
        let controls = column![
            text("Native corner smoothing").size(24),
            text(format!(
                "s = {:.2} (0 = circular, 1 = quartic)",
                self.smoothing
            )),
            slider(0.0..=1.0, self.smoothing, Message::Smoothing).step(0.01),
            text(format!(
                "Radii TL/TR/BR/BL: {:.1}/{:.1}/{:.1}/{:.1}",
                self.radius.top_left,
                self.radius.top_right,
                self.radius.bottom_right,
                self.radius.bottom_left
            )),
            slider(0.0..=120.0, self.radius.top_left, |v| Message::Radius(0, v)).step(0.25),
            slider(0.0..=120.0, self.radius.top_right, |v| Message::Radius(
                1, v
            ))
            .step(0.25),
            slider(0.0..=120.0, self.radius.bottom_right, |v| Message::Radius(
                2, v
            ))
            .step(0.25),
            slider(0.0..=120.0, self.radius.bottom_left, |v| Message::Radius(
                3, v
            ))
            .step(0.25),
            text(format!("Inside border: {:.2}px", self.border_width)),
            slider(0.0..=12.0, self.border_width, Message::BorderWidth).step(0.25),
            text(format!(
                "Shadow x/y/blur: {:.1}/{:.1}/{:.1}",
                self.shadow.offset.x, self.shadow.offset.y, self.shadow.blur_radius
            )),
            slider(-30.0..=30.0, self.shadow.offset.x, Message::ShadowX),
            slider(-30.0..=30.0, self.shadow.offset.y, Message::ShadowY),
            slider(0.0..=30.0, self.shadow.blur_radius, Message::Blur),
            toggler(self.snap)
                .label("Pixel snap")
                .on_toggle(Message::Snap),
            toggler(self.gradient)
                .label("Gradient quad")
                .on_toggle(Message::Gradient),
            toggler(self.dark)
                .label("Dark theme")
                .on_toggle(Message::Dark),
            row![
                text("DPI %"),
                pick_list(Some(self.dpi), [100, 125, 150, 200], u16::to_string)
                    .on_select(Message::Dpi),
                text("Phase /100 px"),
                pick_list(Some(self.phase), [0, 25, 50, 75], u16::to_string)
                    .on_select(Message::Phase)
            ]
            .spacing(8),
            toggler(self.crop)
                .label("Source crop")
                .on_toggle(Message::Crop),
            row![
                text("Fit"),
                pick_list(
                    Some(self.fit),
                    [ContentFit::Cover, ContentFit::Contain, ContentFit::None],
                    |fit| format!("{fit:?}")
                )
                .on_select(Message::Fit),
                text("Angle"),
                pick_list(Some(self.rotation), [0, 15, 45, 90], u16::to_string)
                    .on_select(Message::Rotation)
            ]
            .spacing(8),
            row![
                text("Image %"),
                pick_list(Some(self.image_scale), [75, 100, 150, 200], u16::to_string)
                    .on_select(Message::ImageScale),
                text("Alpha %"),
                pick_list(Some(self.opacity), [0, 50, 100], u16::to_string)
                    .on_select(Message::Opacity)
            ]
            .spacing(8),
            button("Capture window (F5)").on_press(Message::Capture),
            text(&self.capture_status).size(12),
        ]
        .spacing(10)
        .width(410);
        let previews = column![
            text(format!(
                "backend={}  scale={}  s={:.2}  snap={}  phase={:.2}",
                std::env::var("ICED_BACKEND").unwrap_or_else(|_| "auto".into()),
                self.dpi,
                self.smoothing,
                self.snap,
                self.phase as f32 / 100.0
            ))
            .size(13),
            quad::CustomQuad {
                border: Border::default()
                    .rounded(self.radius)
                    .smoothing(self.smoothing)
                    .width(self.border_width)
                    .color(border_color),
                shadow: self.shadow,
                snap: self.snap,
                fill
            },
            row![self.poster(false, false), self.poster(false, true)].spacing(24),
            text("Scroll: truncation must not introduce corners"),
            scrollable(column![self.poster(true, false), self.poster(false, true)].spacing(24))
                .height(210),
        ]
        .spacing(24);
        container(
            row![
                scrollable(controls).height(Fill),
                container(previews).padding(Padding {
                    left: self.phase as f32 / 100.0,
                    top: self.phase as f32 / 100.0,
                    ..Padding::ZERO
                })
            ]
            .spacing(36),
        )
        .padding(24)
    }
}

mod quad {
    use iced::advanced::layout::{self, Layout};
    use iced::advanced::renderer;
    use iced::advanced::widget::{self, Widget};
    use iced::mouse;
    use iced::{Background, Border, Length, Rectangle, Shadow, Size};

    pub struct CustomQuad {
        pub border: Border,
        pub shadow: Shadow,
        pub snap: bool,
        pub fill: Background,
    }
    impl widget::Meta for CustomQuad {}

    impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer> for CustomQuad
    where
        Renderer: renderer::Renderer,
    {
        fn size(&self) -> Size<Length> {
            Size::new(Length::Shrink, Length::Shrink)
        }
        fn layout(
            &mut self,
            tree: &mut widget::Tree,
            _renderer: &Renderer,
            _limits: &layout::Limits,
        ) {
            tree.size = Size::new(460.0, 170.0);
        }
        fn draw(
            &self,
            _tree: &widget::Tree,
            renderer: &mut Renderer,
            _theme: &Theme,
            _style: &renderer::Style,
            layout: Layout,
            _cursor: mouse::Cursor,
            _viewport: &Rectangle,
        ) {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: layout.bounds(),
                    border: self.border,
                    shadow: self.shadow,
                    snap: self.snap,
                },
                self.fill,
            );
        }
    }
}

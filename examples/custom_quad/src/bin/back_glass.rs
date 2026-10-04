//! Reproduction of Jellypilot detail/chrome.rs::BackGlass::draw and the
//! resting detail_back catalog style. Production code is intentionally untouched.
//! Synthetic blue sources replace the unavailable original hero artwork.
use iced::advanced::{Layout, Renderer as _, Widget as CoreWidget, layout, renderer, widget};
use iced::widget::{Image, button, column, container, image, row, scrollable, slider, svg, text};
use iced::{
    Background, Border, Color, ContentFit, Degrees, Element, Fill, Length, Point, Rectangle, Size,
    Task, Theme, Vector, Widget, gradient,
};

const CANVAS: Color = Color::from_rgb8(0x0a, 0x0b, 0x0e);
const RADIUS: f32 = 12.0;
const SMOOTHING: f32 = 0.6;
const FRAME: Size = Size::new(960.0, 520.0);
const SAMPLE: Size = Size::new(220.0, 96.0);
// Reicon chevron-left, MIT, https://reicon.dev; same asset as Jellypilot.
const CHEVRON: &str = "<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none'><polyline points='15.3333 20.3333 7 12 15.3333 3.6667' stroke='white' stroke-linecap='round' stroke-linejoin='round' stroke-width='1.5'/></svg>";

fn main() -> iced::Result {
    let mut app = iced::application(Reproduction::new, Reproduction::update, Reproduction::view)
        .title("Jellypilot BackGlass / exact layer reproduction")
        .theme(Theme::Light)
        .window_size((850, 680));
    if let Some(path) =
        std::env::args().find_map(|arg| arg.strip_prefix("--font=").map(str::to_owned))
    {
        // Register the variable font's 300-weight matching descriptor, as in
        // Jellypilot's fonts::configure, without vendoring its licensed asset.
        let bytes = std::fs::read(path).expect("read supplied Manrope V5 font");
        let mut system = iced::advanced::graphics::text::font_system()
            .write()
            .expect("font system lock");
        system.load_font(bytes.into());
        let db = system.raw().db_mut();
        let mut face = db
            .faces()
            .find(|face| face.families.iter().any(|(name, _)| name == "Manrope V5"))
            .expect("Manrope V5 face")
            .clone();
        face.id = iced::advanced::graphics::text::cosmic_text::fontdb::ID::dummy();
        face.weight = iced::advanced::graphics::text::cosmic_text::fontdb::Weight(300);
        db.push_face_info(face);
        drop(system);
        app = app.font(iced::Font {
            weight: iced::font::Weight::Light,
            ..iced::Font::new("Manrope V5")
        });
    }
    app.run()
}

struct Reproduction {
    sources: [image::Handle; 2],
    icon: svg::Handle,
    sigma: f32,
    capture: Option<String>,
    status: String,
}

#[derive(Debug, Clone)]
enum Message {
    Sigma(f32),
    Clicked,
    Capture,
    Captured(iced::window::screenshot::Screenshot),
    Saved(Result<String, String>),
}

impl Reproduction {
    fn new() -> (Self, Task<Message>) {
        let state = Self {
            sources: [source(false), source(true)],
            icon: svg::Handle::from_memory(CHEVRON.as_bytes()),
            sigma: 10.0,
            capture: std::env::args()
                .find_map(|arg| arg.strip_prefix("--capture=").map(str::to_owned)),
            status: "Diagnostic controls change this example only, not Jellypilot or the renderer."
                .into(),
        };
        let task = if state.capture.is_some() {
            Task::perform(
                async {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                },
                |_| Message::Capture,
            )
        } else {
            Task::none()
        };
        (state, task)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Sigma(value) => self.sigma = value,
            Message::Clicked => {
                self.status = "Back clicked. No navigation in this isolated reproduction.".into()
            }
            Message::Capture => {
                return iced::window::latest()
                    .and_then(iced::window::screenshot)
                    .map(Message::Captured);
            }
            Message::Captured(screenshot) => {
                let path = self
                    .capture
                    .clone()
                    .unwrap_or_else(|| "back-glass.png".into());
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
                self.status = match result {
                    Ok(path) => format!("Saved {path}"),
                    Err(error) => format!("Capture failed: {error}"),
                };
                println!("{}", self.status);
                if self.capture.is_some() {
                    return iced::exit();
                }
            }
        }
        Task::none()
    }

    fn scene(&self, source: usize, variant: Variant) -> impl Widget<Message> {
        let content = button(
            row![
                svg(self.icon.clone()).width(16).height(16),
                text("Back").size(12).line_height(1.3),
            ]
            .spacing(6)
            .align_y(iced::Alignment::Center),
        )
        .padding([8, 14])
        .style(|_, status| button::Style {
            background: Some(
                CANVAS
                    .scale_alpha(match status {
                        button::Status::Hovered => 0.5,
                        button::Status::Pressed => 0.6,
                        button::Status::Disabled => 0.2,
                        button::Status::Active => 0.4,
                    })
                    .into(),
            ),
            text_color: Color::WHITE,
            border: Border::default().rounded(RADIUS).smoothing(SMOOTHING),
            ..Default::default()
        })
        .on_press(Message::Clicked);
        Scene {
            content: content.boxed(),
            source: self.sources[source].clone(),
            variant,
            sigma: self.sigma,
        }
    }

    fn view(&self) -> impl Widget<Message> {
        let mut samples: iced::widget::Column<Element<'_, Message>> = column![].spacing(16);
        for (index, label) in [
            "Flat blue source / removes texture as a variable",
            "Textured blue source / exposes blur sampling",
        ]
        .into_iter()
        .enumerate()
        {
            samples = samples.push(text(label).size(16).boxed());
            samples = samples.push(
                row![
                    self.scene(index, Variant::Original),
                    self.scene(index, Variant::Aligned),
                    self.scene(index, Variant::WithoutLeft),
                ]
                .spacing(20)
                .boxed(),
            );
        }
        container(scrollable(column![
            text("BackGlass: where does the blue rim come from?").size(25),
            text("Same draw order and parameters as chrome.rs. No actual border in the resting style.").size(14),
            row![
                column![text("Original code").size(20), text("Left fade: circular (s = 0)").size(13)].width(SAMPLE.width),
                column![text("Diagnostic A").size(20), text("Only left fade changed to s = 0.6").size(13)].width(SAMPLE.width),
                column![text("Diagnostic B").size(20), text("Only left fade omitted").size(13)].width(SAMPLE.width),
            ].spacing(20),
            samples,
            text(format!("Blur sigma: {:.1} px (original: 10)", self.sigma)),
            slider(0.0..=20.0, self.sigma, Message::Sigma).step(0.5).width(440),
            button("Save window PNG").on_press(Message::Capture),
            text("radius 12 / smoothing 0.6 / fill alpha 0.4 / border width 0 / padding 8 x 14 / hero frame 960 x 520").size(12),
            text("Blue artwork is synthetic, not the original poster. Font can be supplied with --font=<ManropeV5VF.ttf>.").size(12),
            text(&self.status).size(13),
        ].spacing(20))).padding(28).width(Fill).height(Fill)
    }
}

#[derive(Clone, Copy)]
enum Variant {
    Original,
    Aligned,
    WithoutLeft,
}

struct Scene<'a> {
    content: Element<'a, Message>,
    source: image::Handle,
    variant: Variant,
    sigma: f32,
}

impl widget::Meta for Scene<'_> {}

impl CoreWidget<Message, Theme, iced::Renderer> for Scene<'_> {
    fn diff(&mut self, tree: &mut widget::Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }
    fn size(&self) -> Size<Length> {
        Size::new(SAMPLE.width.into(), SAMPLE.height.into())
    }
    fn layout(
        &mut self,
        tree: &mut widget::Tree,
        renderer: &iced::Renderer,
        _limits: &layout::Limits,
    ) {
        self.content.layout(
            &mut tree.children[0],
            renderer,
            &layout::Limits::new(Size::ZERO, SAMPLE),
        );
        tree.children[0].translation = Vector::new(18.0, 18.0);
        tree.size = SAMPLE;
    }
    fn update(
        &mut self,
        tree: &mut widget::Tree,
        event: &iced::Event,
        layout: Layout,
        cursor: iced::mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut iced::advanced::Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let child_layout = layout.iter(&tree.children).next().unwrap().0;
        self.content.update(
            &mut tree.children[0],
            event,
            child_layout,
            cursor,
            renderer,
            shell,
            viewport,
        );
    }
    fn mouse_interaction(
        &self,
        tree: &widget::Tree,
        layout: Layout,
        cursor: iced::mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> iced::mouse::Interaction {
        self.content.mouse_interaction(
            &tree.children[0],
            layout.iter(&tree.children).next().unwrap().0,
            cursor,
            viewport,
            renderer,
        )
    }
    fn draw(
        &self,
        tree: &widget::Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout,
        cursor: iced::mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let Some(sample_clip) = layout.bounds().intersection(viewport) else {
            return;
        };
        let frame = Rectangle::new(layout.position(), FRAME);
        let full_layout = Layout::new(FRAME).move_to(layout.position());
        let bottom = gradient::Linear::new(Degrees(180.0))
            .add_stop(0.0, CANVAS.scale_alpha(0.30))
            .add_stop(0.45, CANVAS.scale_alpha(0.28))
            .add_stop(0.78, CANVAS.scale_alpha(0.72))
            .add_stop(1.0, CANVAS);
        let left = gradient::Linear::new(Degrees(90.0))
            .add_stop(0.0, CANVAS.scale_alpha(0.55))
            .add_stop(0.55, Color::TRANSPARENT);
        // Match hero_banner's order: backdrop, left scrim, bottom scrim.
        renderer.with_layer(sample_clip, |renderer| {
            let backdrop = Image::new(self.source.clone()).content_fit(ContentFit::Cover);
            <Image as CoreWidget<Message, Theme, iced::Renderer>>::draw(
                &backdrop,
                tree,
                renderer,
                theme,
                style,
                full_layout,
                cursor,
                &sample_clip,
            );
        });
        renderer.with_layer(sample_clip, |renderer| {
            renderer.fill_quad(
                renderer::Quad {
                    bounds: frame,
                    ..Default::default()
                },
                Background::Gradient(left.into()),
            );
            renderer.fill_quad(
                renderer::Quad {
                    bounds: frame,
                    ..Default::default()
                },
                Background::Gradient(bottom.into()),
            );
        });
        let button_layout = layout.iter(&tree.children).next().unwrap().0;
        let bounds = button_layout.bounds();
        let Some(visible) = bounds.intersection(&sample_clip) else {
            return;
        };
        // The following glass layers mirror chrome.rs:129-172, including the
        // original mismatch in the left-fade contour for Variant::Original.
        let glass = Image::new(self.source.clone())
            .content_fit(ContentFit::Cover)
            .display_frame(Rectangle::new(Point::new(-18.0, -18.0), FRAME))
            .mask_frame(Rectangle::new(Point::ORIGIN, bounds.size()))
            .border_radius(RADIUS)
            .border_smoothing(SMOOTHING)
            .blur(self.sigma)
            .tint(Background::Gradient(bottom.into()));
        <Image as CoreWidget<Message, Theme, iced::Renderer>>::draw(
            &glass,
            tree,
            renderer,
            theme,
            style,
            button_layout,
            cursor,
            &visible,
        );
        if !matches!(self.variant, Variant::WithoutLeft) {
            let left_alpha = |x: f32| 0.55 * (1.0 - x / (FRAME.width * 0.55).max(1.0)).max(0.0);
            let left_fade = gradient::Linear::new(Degrees(90.0))
                .add_stop(0.0, CANVAS.scale_alpha(left_alpha(18.0)))
                .add_stop(1.0, CANVAS.scale_alpha(left_alpha(18.0 + bounds.width)));
            renderer.with_layer(visible, |renderer| {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds,
                        border: Border {
                            radius: RADIUS.into(),
                            smoothing: if matches!(self.variant, Variant::Aligned) {
                                SMOOTHING
                            } else {
                                0.0
                            },
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    Background::Gradient(left_fade.into()),
                );
            });
        }
        renderer.with_layer(visible, |renderer| {
            self.content.draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                button_layout,
                cursor,
                &visible,
            );
        });
    }
}

fn source(textured: bool) -> image::Handle {
    let (width, height) = (FRAME.width as u32, FRAME.height as u32);
    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let wave = if textured {
                ((x as f32 * 0.11).sin() * (y as f32 * 0.08).cos()
                    + (x as f32 * 0.035 + y as f32 * 0.06).sin())
                    * 8.0
            } else {
                0.0
            };
            rgba.extend_from_slice(&[20, (90.0 + wave) as u8, (150.0 + wave) as u8, 255]);
        }
    }
    image::Handle::from_rgba(width, height, rgba)
}

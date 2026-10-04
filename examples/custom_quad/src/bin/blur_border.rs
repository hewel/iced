//! Visual reproduction only: keep the current renderer and corner geometry.
//! Uses the same-source glass composition from examples/native_blur. Switch
//! tint placement to compare two existing APIs, not an implemented fix.
use iced::widget::{
    button, center, column, container, image, pin, row, scrollable, slider, stack, text, toggler,
};
use iced::{Border, Color, ContentFit, Fill, Rectangle, Task, Theme, Widget};

const TILE_W: f32 = 224.0;
const TILE_H: f32 = 144.0;
const BUTTON_W: f32 = 184.0;
const BUTTON_H: f32 = 64.0;
const BUTTON_X: f32 = 20.0;
const BUTTON_Y: f32 = 40.0;
const RADIUS: f32 = 18.0;
const SMOOTHING: f32 = 0.6;

fn main() -> iced::Result {
    iced::application(Examples::new, Examples::update, Examples::view)
        .title("Blurred buttons / translucent border examples")
        .theme(Theme::Light)
        .window_size((1080, 1020))
        .run()
}

struct Examples {
    sources: [image::Handle; 3],
    sigma: f32,
    alpha: f32,
    width: f32,
    tint: f32,
    image_tint: bool,
    capture: Option<String>,
    status: String,
    clicks: u32,
}

#[derive(Debug, Clone)]
enum Message {
    Sigma(f32),
    Alpha(f32),
    Width(f32),
    Tint(f32),
    ImageTint(bool),
    Pressed(usize, usize),
    Reset,
    Capture,
    Captured(iced::window::screenshot::Screenshot),
    Saved(Result<String, String>),
}

impl Examples {
    fn new() -> (Self, Task<Message>) {
        let args: Vec<String> = std::env::args().collect();
        let state = Self {
            sources: std::array::from_fn(source),
            sigma: 10.0,
            alpha: 0.35,
            width: if args.iter().any(|arg| arg == "--thick") { 6.0 } else { 2.0 },
            tint: 0.25,
            image_tint: args.iter().any(|arg| arg == "--image-tint"),
            capture: args.iter().find_map(|arg| arg.strip_prefix("--capture=").map(str::to_owned)),
            status: "Compare the straight edge and curved corner; button hover does not change its style.".into(),
            clicks: 0,
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
            Message::Alpha(value) => self.alpha = value,
            Message::Width(value) => self.width = value,
            Message::Tint(value) => self.tint = value,
            Message::ImageTint(value) => self.image_tint = value,
            Message::Pressed(scene, variant) => {
                self.clicks += 1;
                self.status = format!(
                    "Clicked scene {}, column {}. Total clicks: {}. Appearance is unchanged.",
                    scene + 1,
                    variant + 1,
                    self.clicks
                );
            }
            Message::Reset => {
                self.sigma = 10.0;
                self.alpha = 0.35;
                self.width = 2.0;
                self.tint = 0.25;
                self.image_tint = false;
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
                    .unwrap_or_else(|| "blur-border.png".into());
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
                return Task::none();
            }
        }
        println!(
            "sigma={:.1} border_alpha={:.2} width={:.1} tint={:.2} image_tint={}",
            self.sigma, self.alpha, self.width, self.tint, self.image_tint
        );
        Task::none()
    }

    fn tile(&self, scene: usize, variant: usize) -> impl Widget<Message> {
        let sigma = if variant == 0 { 0.0 } else { self.sigma };
        let (width, alpha) = match variant {
            1 => (0.0, 0.0),
            3 => (self.width, 1.0),
            _ => (self.width, self.alpha),
        };
        let light = scene == 1;
        let border_color = if light {
            Color::from_rgba(0.0, 0.0, 0.0, alpha)
        } else {
            Color::from_rgba(1.0, 1.0, 1.0, alpha)
        };
        let tint = if scene == 0 {
            Color::from_rgba(1.0, 1.0, 1.0, self.tint)
        } else {
            Color::from_rgba(0.0, 0.0, 0.0, self.tint)
        };
        let mut glass = image(self.sources[scene].clone())
            .width(BUTTON_W)
            .height(BUTTON_H)
            .content_fit(ContentFit::Fill)
            .display_frame(Rectangle {
                x: -BUTTON_X,
                y: -BUTTON_Y,
                width: TILE_W,
                height: TILE_H,
            })
            .mask_frame(Rectangle {
                x: 0.0,
                y: 0.0,
                width: BUTTON_W,
                height: BUTTON_H,
            })
            .border_radius(RADIUS)
            .border_smoothing(SMOOTHING)
            .blur(sigma)
            .snap(false);
        if self.image_tint {
            glass = glass.tint(tint);
        }
        let background = (!self.image_tint).then_some(tint.into());
        let control = button(center(text("Glass button").size(16)))
            .width(BUTTON_W)
            .height(BUTTON_H)
            .padding(0)
            .style(move |_, _| button::Style {
                background,
                text_color: if light { Color::BLACK } else { Color::WHITE },
                border: Border::default()
                    .rounded(RADIUS)
                    .smoothing(SMOOTHING)
                    .width(width)
                    .color(border_color),
                snap: false,
                ..Default::default()
            })
            .on_press(Message::Pressed(scene, variant));
        stack![
            image(self.sources[scene].clone())
                .width(TILE_W)
                .height(TILE_H)
                .content_fit(ContentFit::Fill)
                .snap(false),
            pin(stack![glass, control]).x(BUTTON_X).y(BUTTON_Y),
        ]
    }

    fn view(&self) -> impl Widget<Message> {
        let controls = row![
            column![
                text(format!("Blur sigma: {:.1} px", self.sigma)),
                slider(0.0..=24.0, self.sigma, Message::Sigma).step(0.5)
            ]
            .spacing(8),
            column![
                text(format!("Border opacity: {:.0}%", self.alpha * 100.0)),
                slider(0.0..=1.0, self.alpha, Message::Alpha).step(0.01)
            ]
            .spacing(8),
            column![
                text(format!("Border width: {:.1} px", self.width)),
                slider(0.5..=8.0, self.width, Message::Width).step(0.5)
            ]
            .spacing(8),
            column![
                text(format!("Surface tint: {:.0}%", self.tint * 100.0)),
                slider(0.0..=0.6, self.tint, Message::Tint).step(0.01)
            ]
            .spacing(8),
        ]
        .spacing(24)
        .width(932);
        let headers = row![
            text(format!("Sharp / {:.0}% border", self.alpha * 100.0)).width(TILE_W),
            text("Blur / no border").width(TILE_W),
            text(format!("Blur / {:.0}% border", self.alpha * 100.0)).width(TILE_W),
            text("Blur / opaque border").width(TILE_W),
        ]
        .spacing(12);
        let mut scenes = column![headers].spacing(10);
        for (scene, label) in [
            "Dark stripes / white border + white tint",
            "Light stripes / black border + black tint",
            "High-contrast checks / white border + black tint",
        ]
        .into_iter()
        .enumerate()
        {
            scenes = scenes.push(text(label).size(15).boxed());
            scenes = scenes.push(
                row![
                    self.tile(scene, 0),
                    self.tile(scene, 1),
                    self.tile(scene, 2),
                    self.tile(scene, 3)
                ]
                .spacing(12)
                .boxed(),
            );
        }
        let placement = if self.image_tint {
            "Tint layer: Image::tint (full rounded mask), with a transparent button above."
        } else {
            "Tint layer: button background (same quad as the border), over the blurred image."
        };
        container(scrollable(column![
            text("Blurred buttons, translucent borders").size(30),
            text("Visual reproduction only. Existing corners and renderer are unchanged.").size(16),
            controls,
            row![
                toggler(self.image_tint).label("Put surface tint on image").on_toggle(Message::ImageTint),
                button("Reset").on_press(Message::Reset),
                button("Save PNG").on_press(Message::Capture),
            ].spacing(24),
            text(placement).size(14),
            scenes,
            text("Each row repeats the identical source at the identical coordinates. Radius 18 px / smoothing 60% / pixel snap off.").size(12),
            text("Try 6 px borders to magnify the band, then return to 1-2 px. At sigma 0, the first and third columns should match.").size(12),
            text(&self.status).size(13),
        ].spacing(18).width(932))).padding(28).width(Fill).height(Fill)
    }
}

fn source(scene: usize) -> image::Handle {
    let width = TILE_W as u32;
    let height = TILE_H as u32;
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let alternate = if scene == 2 {
                (x / 10 + y / 10) % 2 == 0
            } else {
                (x / 16) % 2 == 0
            };
            let rgb = match (scene, alternate) {
                (0, true) => [20, 27, 39],
                (0, false) => [73, 88, 111],
                (1, true) => [240, 243, 247],
                (1, false) => [174, 190, 209],
                (_, true) => [31, 70, 130],
                (_, false) => [232, 137, 72],
            };
            pixels.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
        }
    }
    image::Handle::from_rgba(width, height, pixels)
}

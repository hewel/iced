//! Run with `cargo run -p native_blur` (choose a backend with ICED_BACKEND).
//! Check the 0/mid/full strip at fractional desktop scale, scroll the Hero,
//! and open the dialog. Its background counter stays live. Pause it and edit
//! dialog text to exercise lower-scene reuse. The toast is drawn after blur.
use iced::widget::{
    button, center, column, container, image, modal, mouse_area, opaque, pin, row, scrollable,
    slider, stack, text, text_input,
};
use iced::{Color, ContentFit, Element, Fill, Rectangle, Subscription};
use std::time::Duration;

const WIDTH: f32 = 640.0;
const HEIGHT: f32 = 320.0;

fn main() -> iced::Result {
    iced::application(App::default, App::update, App::view)
        .subscription(App::subscription)
        .run()
}

struct App {
    art: image::Handle,
    progress: f32,
    sigma: f32,
    tick: u64,
    live: bool,
    dialog: bool,
    toast: bool,
    entry: String,
}

impl Default for App {
    fn default() -> Self {
        let mut rgba = Vec::with_capacity(800 * 400 * 4);
        for y in 0..400 {
            for x in 0..800 {
                let checker = ((x / 12 + y / 12) % 2) as u8;
                rgba.extend_from_slice(&[
                    (x * 255 / 800) as u8,
                    40 + checker * 140,
                    (y * 255 / 400) as u8,
                    255,
                ]);
            }
        }
        Self {
            art: image::Handle::from_rgba(800, 400, rgba),
            progress: 0.5,
            sigma: 12.0,
            tick: 0,
            live: true,
            dialog: false,
            toast: false,
            entry: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Progress(f32),
    Sigma(f32),
    Tick,
    Live,
    Open,
    Close,
    Toast,
    Entry(String),
}

impl App {
    fn subscription(&self) -> Subscription<Message> {
        if self.live {
            iced::time::every(Duration::from_millis(100)).map(|_| Message::Tick)
        } else {
            Subscription::none()
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Progress(value) => self.progress = value,
            Message::Sigma(value) => self.sigma = value,
            Message::Tick => self.tick += 1,
            Message::Live => self.live = !self.live,
            Message::Open => self.dialog = true,
            Message::Close => self.dialog = false,
            Message::Toast => self.toast = !self.toast,
            Message::Entry(value) => self.entry = value,
        }
    }

    fn art(&self) -> image::Image {
        image(self.art.clone())
            .width(WIDTH)
            .height(HEIGHT)
            .crop(Rectangle {
                x: 80,
                y: 40,
                width: 640,
                height: 320,
            })
            .content_fit(ContentFit::Cover)
            .border_radius(24)
            .border_smoothing(0.6)
    }

    fn glass_button(&self, x: f32, label: &'static str) -> Element<'_, Message> {
        let glass = self
            .art()
            .width(160)
            .height(44)
            .display_frame(Rectangle {
                x: -x,
                y: -240.0,
                width: WIDTH,
                height: HEIGHT,
            })
            .mask_frame(Rectangle {
                x: 0.0,
                y: 0.0,
                width: 160.0,
                height: 44.0,
            })
            .border_radius(14)
            .blur(self.sigma)
            .tint(Self::fade());
        pin(stack![
            glass,
            button(center(text(label)))
                .width(160)
                .height(44)
                .padding(0)
                .style(|theme, status| button::Style {
                    background: None,
                    ..button::primary(theme, status)
                })
                .on_press(Message::Toast)
        ])
        .x(x)
        .y(240)
        .into()
    }

    fn fade() -> iced::gradient::Linear {
        iced::gradient::Linear::new(iced::Radians(std::f32::consts::PI))
            .add_stop(0.0, Color::TRANSPARENT)
            .add_stop(1.0, Color::from_rgba(0.02, 0.03, 0.07, 0.8))
    }

    fn view(&self) -> Element<'_, Message> {
        let fade = Self::fade();
        let hero = stack![
            self.art().tint(fade),
            self.art()
                .blur(self.sigma)
                .tint(fade)
                .visible_region(Rectangle {
                    x: 0.0,
                    y: HEIGHT - 4.0,
                    width: WIDTH,
                    height: 4.0
                }),
            // Reuse the full-card mask for a color-only played layer.
            self.art()
                .opacity(0.0_f32)
                .tint(Color::from_rgba(0.2, 0.65, 1.0, 0.85))
                .visible_region(Rectangle {
                    x: 0.0,
                    y: HEIGHT - 4.0,
                    width: WIDTH * self.progress,
                    height: 4.0
                }),
            pin(text("Native image glass").size(34).color(Color::WHITE))
                .x(24)
                .y(24),
            self.glass_button(24.0, "Same-source glass A"),
            self.glass_button(200.0, "Same-source glass B"),
        ];
        let base = container(
            column![
                text("Native blur: image rendition + live scene").size(26),
                row![
                    button("0%").on_press(Message::Progress(0.0)),
                    button("50%").on_press(Message::Progress(0.5)),
                    button("100%").on_press(Message::Progress(1.0))
                ]
                .spacing(8),
                slider(0.0..=1.0, self.progress, Message::Progress).step(0.01),
                text(format!("Sigma: {:.1} logical px", self.sigma)),
                slider(0.0..=32.0, self.sigma, Message::Sigma).step(0.5),
                row![
                    button("Open modal").on_press(Message::Open),
                    button(if self.live {
                        "Pause lower scene"
                    } else {
                        "Resume lower scene"
                    })
                    .on_press(Message::Live),
                    button("Toggle toast").on_press(Message::Toast)
                ]
                .spacing(8),
                text(format!("Live lower scene: {}", self.tick)).size(28),
                scrollable(
                    column![
                        hero,
                        text("Scroll: image and sampled buttons share local coordinates."),
                        container(text("More lower-scene content")).height(260)
                    ]
                    .spacing(16)
                )
                .height(360),
            ]
            .spacing(12)
            .width(WIDTH),
        )
        .padding(24);
        let mut layers = stack![base];
        if self.dialog {
            let dialog = container(
                column![
                    text("Live scene backdrop").size(26),
                    text("Pause the lower scene, then type: only this foreground changes."),
                    text_input("Crisp editable foreground", &self.entry).on_input(Message::Entry),
                    row![
                        button("Pause / resume background").on_press(Message::Live),
                        button("Toast above modal").on_press(Message::Toast),
                        button("Close").on_press(Message::Close)
                    ]
                    .spacing(8),
                ]
                .spacing(16),
            )
            .padding(24)
            .style(container::rounded_box);
            let scrim = opaque(
                mouse_area(center(opaque(dialog)).style(|_| container::Style {
                    background: Some(Color::from_rgba(0.0, 0.0, 0.0, 0.25).into()),
                    ..Default::default()
                }))
                .on_press(Message::Close),
            );
            layers = layers.push(modal(self.sigma, scrim));
        }
        if self.toast {
            layers = layers.push(
                pin(opaque(
                    container(
                        row![
                            text("Crisp toast, after Modal"),
                            button("Dismiss").on_press(Message::Toast)
                        ]
                        .spacing(12),
                    )
                    .padding(12)
                    .style(container::rounded_box),
                ))
                .x(24)
                .y(12),
            );
        }
        container(layers).width(Fill).height(Fill).into()
    }
}

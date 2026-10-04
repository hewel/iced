//! Image gradients and live, overlapping backdrop panels.
use iced::widget::{
    backdrop, button, checkbox, column, container, image, pin, row, scrollable, slider, stack,
    text, text_input,
};
use iced::{Blur, Border, Color, ContentFit, Fill, Font, Subscription, Theme, Widget};
use std::time::Duration;

const MUTED: Color = Color::from_rgb8(169, 178, 198);
const INK: Color = Color::from_rgb8(22, 30, 46);

fn main() -> iced::Result {
    iced::application(Playground::default, Playground::update, Playground::view)
        .title("Progressive blur")
        .theme(Theme::Dark)
        .subscription(Playground::subscription)
        .window_size((1080, 1040))
        .run()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Vertical,
    Horizontal,
}

struct Playground {
    art: image::Handle,
    strength: f32,
    radius: f32,
    axis: Axis,
    reverse: bool,
    middle: bool,
    live: bool,
    tick: u64,
    note: String,
    taps: u32,
}

impl Default for Playground {
    fn default() -> Self {
        Self {
            art: artwork(),
            strength: 24.0,
            radius: 28.0,
            axis: Axis::Vertical,
            reverse: false,
            middle: false,
            live: true,
            tick: 0,
            note: String::new(),
            taps: 0,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Strength(f32),
    Radius(f32),
    Vertical,
    Horizontal,
    Reverse(bool),
    Middle(bool),
    ToggleMotion,
    Tick,
    Note(String),
    Tap,
}

impl Playground {
    fn subscription(&self) -> Subscription<Message> {
        if self.live {
            iced::time::every(Duration::from_millis(50)).map(|_| Message::Tick)
        } else {
            Subscription::none()
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Strength(value) => self.strength = value,
            Message::Radius(value) => self.radius = value,
            Message::Vertical => self.axis = Axis::Vertical,
            Message::Horizontal => self.axis = Axis::Horizontal,
            Message::Reverse(value) => self.reverse = value,
            Message::Middle(value) => self.middle = value,
            Message::ToggleMotion => self.live = !self.live,
            Message::Tick => self.tick = self.tick.wrapping_add(1),
            Message::Note(value) => self.note = value,
            Message::Tap => self.taps = self.taps.wrapping_add(1),
        }
    }

    fn profile(&self) -> Blur {
        let (start, end) = if self.reverse {
            (self.strength, 0.0)
        } else {
            (0.0, self.strength)
        };
        let blur = match self.axis {
            Axis::Vertical => Blur::vertical_gradient(start, end),
            Axis::Horizontal => Blur::horizontal_gradient(start, end),
        };
        if self.middle {
            blur.range(0.25, 0.75)
        } else {
            blur
        }
    }

    fn direction_label(&self) -> &'static str {
        match (self.axis, self.reverse) {
            (Axis::Vertical, false) => "Clear at the top, soft at the bottom",
            (Axis::Vertical, true) => "Soft at the top, clear at the bottom",
            (Axis::Horizontal, false) => "Clear on the left, soft on the right",
            (Axis::Horizontal, true) => "Soft on the left, clear on the right",
        }
    }

    fn sample(&self, title: &'static str, subtitle: String, blur: Blur) -> impl Widget<Message> {
        column![
            image(&self.art)
                .width(Fill)
                .height(158)
                .content_fit(ContentFit::Cover)
                .border_radius(self.radius)
                .border_smoothing(0.6)
                .blur(blur),
            text(title).size(18),
            text(subtitle).size(13).color(MUTED),
        ]
        .spacing(8)
        .width(Fill)
    }

    fn scene_card(&self, index: usize) -> impl Widget<Message> {
        let colors = [
            Color::from_rgb8(223, 176, 137),
            Color::from_rgb8(154, 187, 171),
            Color::from_rgb8(158, 179, 218),
            Color::from_rgb8(209, 169, 187),
        ];
        let titles = [
            "Morning light",
            "A slower afternoon",
            "Blue hour",
            "After the rain",
        ];
        container(
            row![
                image(&self.art)
                    .width(128)
                    .height(92)
                    .content_fit(ContentFit::Cover)
                    .border_radius(12),
                column![
                    text(format!("{:02}  /  FIELD NOTES", index + 1))
                        .size(12)
                        .font(Font::MONOSPACE),
                    text(titles[index % titles.len()]).size(29),
                    text("Small details, fine lines, and a change of scenery.").size(16),
                ]
                .spacing(8),
            ]
            .spacing(20)
            .align_y(iced::Center),
        )
        .padding(18)
        .width(Fill)
        .style(move |_| container::Style {
            background: Some(colors[index % colors.len()].into()),
            text_color: Some(INK),
            border: Border::default().rounded(20),
            ..Default::default()
        })
    }

    fn live_scene(&self) -> impl Widget<Message> {
        let cards = column((0..8).map(|index| self.scene_card(index))).spacing(14);
        let lower_scene = scrollable(container(cards).padding(16))
            .width(Fill)
            .height(Fill);
        let phase = self.tick as f32 * 0.035;
        let moving_tile = container(
            column![
                text("IN MOTION").size(13).font(Font::MONOSPACE),
                text(format!("{:04}", self.tick % 10_000))
                    .size(38)
                    .font(Font::MONOSPACE),
            ]
            .spacing(5),
        )
        .padding(18)
        .width(180)
        .height(108)
        .style(|_| container::Style {
            background: Some(Color::from_rgb8(236, 201, 102).into()),
            text_color: Some(INK),
            border: Border::default().rounded(20),
            ..Default::default()
        });

        // Each panel samples everything already drawn, including the lower
        // panel where they overlap. Its own controls are drawn afterward.
        let rear_panel = backdrop(
            self.profile(),
            container(
                column![
                    text("A clear place to think").size(25),
                    text("The scene keeps moving behind this panel.").size(14),
                    text_input("Write a note…", &self.note)
                        .on_input(Message::Note)
                        .padding(12),
                    text("Pause motion, then keep typing.")
                        .size(13)
                        .color(MUTED),
                ]
                .spacing(16),
            )
            .padding(24)
            .width(392)
            .height(242),
        )
        .border_radius(self.radius)
        .border_smoothing(0.6)
        .tint(Color::from_rgba(0.035, 0.05, 0.08, 0.48));
        let front_panel = backdrop(
            self.profile(),
            container(
                column![
                    text("Another layer").size(21),
                    text("Overlapping glass, crisp controls.").size(13),
                    button(text(format!("Tap me  ·  {}", self.taps)))
                        .on_press(Message::Tap)
                        .height(40)
                        .padding([8, 16]),
                ]
                .spacing(12),
            )
            .padding(20)
            .width(302)
            .height(168),
        )
        .border_radius(self.radius)
        .border_smoothing(0.6)
        .tint(Color::from_rgba(0.08, 0.13, 0.18, 0.50));

        stack![
            lower_scene,
            pin(moving_tile)
                .x(210.0 + phase.sin() * 170.0)
                .y(132.0 + phase.cos() * 80.0),
            pin(rear_panel).x(28).y(42),
            pin(front_panel).x(330).y(232),
        ]
        .width(Fill)
        .height(424)
        .clip(true)
    }

    fn view(&self) -> impl Widget<Message> {
        let controls = row![
            column![
                text(format!("Maximum blur  ·  {:.0} px", self.strength)).size(14),
                slider(0.0..=32.0, self.strength, Message::Strength)
                    .step(1.0)
                    .height(40),
            ]
            .width(230),
            column![
                text(format!("Corner radius  ·  {:.0} px", self.radius)).size(14),
                slider(0.0..=40.0, self.radius, Message::Radius)
                    .step(1.0)
                    .height(40),
            ]
            .width(230),
            column![
                text("Direction").size(14),
                row![
                    button("Vertical")
                        .on_press(Message::Vertical)
                        .height(40)
                        .style(if self.axis == Axis::Vertical {
                            button::primary
                        } else {
                            button::secondary
                        }),
                    button("Horizontal")
                        .on_press(Message::Horizontal)
                        .height(40)
                        .style(if self.axis == Axis::Horizontal {
                            button::primary
                        } else {
                            button::secondary
                        }),
                ]
                .spacing(8),
            ],
        ]
        .spacing(24)
        .wrap();
        let options = row![
            checkbox(self.reverse)
                .label("Reverse direction")
                .on_toggle(Message::Reverse)
                .line_height(iced::Pixels(40.0)),
            checkbox(self.middle)
                .label("Transition in middle half")
                .on_toggle(Message::Middle)
                .line_height(iced::Pixels(40.0)),
        ]
        .spacing(28)
        .wrap();
        let comparison = row![
            self.sample(
                "Original",
                "Fine detail throughout".into(),
                Blur::Uniform(0.0)
            ),
            self.sample(
                "Uniform",
                format!("{:.0} px everywhere", self.strength),
                Blur::Uniform(self.strength),
            ),
            self.sample("Progressive", self.direction_label().into(), self.profile()),
        ]
        .spacing(18);

        container(scrollable(
            column![
                text("Progressive blur").size(36),
                text("Compare images, then explore glass over a moving scene.")
                    .size(16)
                    .color(MUTED),
                controls,
                options,
                comparison,
                row![
                    column![
                        text("Glass over a live scene").size(25),
                        text("Scroll inside the scene. The panels stay in place.")
                            .size(14)
                            .color(MUTED),
                    ]
                    .spacing(6)
                    .width(Fill),
                    button(if self.live {
                        "Pause motion"
                    } else {
                        "Resume motion"
                    })
                    .on_press(Message::ToggleMotion)
                    .height(40)
                    .padding([8, 16]),
                ]
                .spacing(16)
                .align_y(iced::Center),
                self.live_scene(),
                text("Try zero blur, reverse the direction, or resize the window to clip a panel.")
                    .size(13)
                    .color(MUTED),
            ]
            .spacing(20)
            .padding(28)
            .width(Fill),
        ))
        .width(Fill)
        .height(Fill)
    }
}

fn artwork() -> image::Handle {
    let (width, height) = (720_u32, 360_u32);
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let stripe = (x / 9 + y / 9) % 2 == 0;
            let color = if x % 90 < 2 || y % 90 < 2 {
                [249, 240, 210]
            } else if stripe {
                [42 + (x * 100 / width) as u8, 112, 156]
            } else {
                [222, 149 + (y * 60 / height) as u8, 106]
            };
            pixels.extend_from_slice(&[color[0], color[1], color[2], 255]);
        }
    }
    image::Handle::from_rgba(width, height, pixels)
}

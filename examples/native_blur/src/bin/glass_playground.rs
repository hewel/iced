//! Regular and clear materials over native scrolling and animated content.
use iced::glass::Quality;
use iced::widget::glass::{Glass, Style as GlassStyle};
use iced::widget::{
    button, checkbox, column, container, glass, image, pick_list, pin, responsive, row, scrollable,
    slider, stack, text, text_input,
};
use iced::{Border, Color, ContentFit, Fill, Font, Subscription, Theme, Vector, Widget};
use std::time::Duration;

const INK: Color = Color::from_rgb8(29, 43, 49);
const MUTED: Color = Color::from_rgb8(93, 106, 109);

fn main() -> iced::Result {
    iced::application(Playground::default, Playground::update, Playground::view)
        .title("Glass materials")
        .theme(Theme::Light)
        .subscription(Playground::subscription)
        .window_size((1100, 1010))
        .run()
}

#[derive(Clone, Copy, Debug)]
enum Preset {
    Regular,
    Clear,
}

impl Preset {
    fn index(self) -> usize {
        match self {
            Self::Regular => 0,
            Self::Clear => 1,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Regular => "Regular",
            Self::Clear => "Clear",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenderQuality {
    Quality,
    Balanced,
    Performance,
    Adaptive,
    HalfResolution,
}

impl RenderQuality {
    const ALL: [Self; 5] = [
        Self::Quality,
        Self::Balanced,
        Self::Performance,
        Self::Adaptive,
        Self::HalfResolution,
    ];

    fn label(&self) -> &'static str {
        match self {
            Self::Quality => "Quality",
            Self::Balanced => "Balanced",
            Self::Performance => "Performance",
            Self::Adaptive => "Adaptive",
            Self::HalfResolution => "Fixed 50%",
        }
    }

    fn material(self) -> Quality {
        match self {
            Self::Quality => Quality::Quality,
            Self::Balanced => Quality::Balanced,
            Self::Performance => Quality::Performance,
            Self::Adaptive => Quality::Adaptive,
            Self::HalfResolution => Quality::Fixed(0.5),
        }
    }
}

struct Playground {
    art: image::Handle,
    refraction: f32,
    depth: f32,
    light_angle: f32,
    tint: f32,
    quality: RenderQuality,
    interactive: bool,
    reduce_motion: bool,
    reduced_transparency: bool,
    high_contrast: bool,
    live: bool,
    tick: u64,
    notes: [String; 2],
    marks: [u32; 2],
}

impl Default for Playground {
    fn default() -> Self {
        Self {
            art: artwork(),
            refraction: 1.0,
            depth: 1.0,
            light_angle: -125.0,
            tint: 1.0,
            quality: RenderQuality::Balanced,
            interactive: true,
            reduce_motion: false,
            reduced_transparency: false,
            high_contrast: false,
            live: true,
            tick: 0,
            notes: [String::new(), String::new()],
            marks: [0; 2],
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Refraction(f32),
    Depth(f32),
    LightAngle(f32),
    Tint(f32),
    Quality(RenderQuality),
    Interactive(bool),
    ReduceMotion(bool),
    ReducedTransparency(bool),
    HighContrast(bool),
    ToggleMotion,
    Tick,
    Note(Preset, String),
    Mark(Preset),
}

impl Playground {
    fn subscription(&self) -> Subscription<Message> {
        if self.live && !self.reduce_motion {
            iced::time::every(Duration::from_millis(50)).map(|_| Message::Tick)
        } else {
            Subscription::none()
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::Refraction(value) => self.refraction = value,
            Message::Depth(value) => self.depth = value,
            Message::LightAngle(value) => self.light_angle = value,
            Message::Tint(value) => self.tint = value,
            Message::Quality(value) => self.quality = value,
            Message::Interactive(value) => self.interactive = value,
            Message::ReduceMotion(value) => self.reduce_motion = value,
            Message::ReducedTransparency(value) => self.reduced_transparency = value,
            Message::HighContrast(value) => self.high_contrast = value,
            Message::ToggleMotion => self.live = !self.live,
            Message::Tick => self.tick = self.tick.wrapping_add(1),
            Message::Note(preset, value) => self.notes[preset.index()] = value,
            Message::Mark(preset) => {
                self.marks[preset.index()] = self.marks[preset.index()].wrapping_add(1);
            }
        }
    }

    fn material_style(&self, preset: Preset) -> GlassStyle {
        let mut style = match preset {
            Preset::Regular => GlassStyle::regular(),
            Preset::Clear => GlassStyle::clear(),
        };
        // Multipliers retain the optical differences between the presets.
        style.optics.refraction *= self.refraction;
        style.optics.depth *= self.depth;
        style.optics.tint = style.optics.tint.scale_alpha(self.tint);
        let angle = self.light_angle.to_radians();
        style.optics.light = Vector::new(angle.cos(), angle.sin());
        style
    }

    fn surface<W>(&self, preset: Preset, content: W) -> Glass<W> {
        glass(content)
            .style(self.material_style(preset))
            .quality(self.quality.material())
            .border_radius(28)
            .border_smoothing(0.6)
            .interactive(self.interactive)
            .reduce_motion(self.reduce_motion)
            .reduced_transparency(self.reduced_transparency)
            .high_contrast(self.high_contrast)
    }

    fn scenery(&self) -> impl Widget<Message> {
        let cards = column((0..5).map(|index| {
            let colors = [
                Color::from_rgb8(203, 215, 182),
                Color::from_rgb8(233, 213, 168),
                Color::from_rgb8(181, 210, 215),
            ];
            let labels = ["Along the coast", "In the garden", "By the lake"];
            container(
                column![
                    text(format!("{:02}  /  PLACES TO PAUSE", index + 1))
                        .size(12)
                        .font(Font::MONOSPACE),
                    text(labels[index % labels.len()]).size(33),
                    image(&self.art)
                        .width(Fill)
                        .height(112)
                        .content_fit(ContentFit::Cover)
                        .border_radius(12),
                    text("A little light. A longer walk.").size(16),
                ]
                .spacing(12),
            )
            .padding(22)
            .width(Fill)
            .style(move |_| container::Style {
                background: Some(colors[index % colors.len()].into()),
                text_color: Some(INK),
                border: Border::default().rounded(20),
                ..Default::default()
            })
        }))
        .spacing(12);
        scrollable(container(cards).padding(12))
            .width(Fill)
            .height(Fill)
    }

    fn specimen(&self, preset: Preset, width: f32) -> impl Widget<Message> {
        let panel_width = (width - 48.0).max(160.0);
        let phase = self.tick as f32 * 0.025;
        let sun = container(text("SUNLIGHT").font(Font::MONOSPACE).size(12))
            .center_x(144)
            .center_y(144)
            .style(|_| container::Style {
                background: Some(Color::from_rgb8(246, 199, 103).into()),
                text_color: Some(INK),
                border: Border::default().rounded(72),
                ..Default::default()
            });
        let panel = self.surface(
            preset,
            container(
                column![
                    text("Keep this moment").size(26),
                    text("Let the view come through.").size(15),
                    text_input("Give this place a name…", &self.notes[preset.index()])
                        .on_input(move |value| Message::Note(preset, value))
                        .padding(12),
                    text("Click or Tab into the field to focus.")
                        .size(12)
                        .color(MUTED),
                ]
                .spacing(16),
            )
            .padding(24)
            .width(panel_width)
            .height(232)
            .style(|_| container::Style {
                text_color: Some(INK),
                ..Default::default()
            }),
        );
        let marks = self.marks[preset.index()];
        let mark = self
            .surface(
                preset,
                button(
                    text(if marks == 0 {
                        "Mark this view".to_owned()
                    } else {
                        format!("Marked  ·  {marks}")
                    })
                    .size(16),
                )
                .on_press(Message::Mark(preset))
                .width(200)
                .height(48)
                .padding([10, 22])
                .style(|_, _| button::Style {
                    background: None,
                    text_color: INK,
                    ..Default::default()
                }),
            )
            .border_radius(24);
        // Background widgets render first; neither the panel's input nor the
        // button label is part of its own optical capture.
        stack![
            self.scenery(),
            pin(sun)
                .x(30.0 + (width - 190.0).max(0.0) * (phase.sin() * 0.5 + 0.5))
                .y(150.0 + phase.cos() * 75.0),
            pin(panel).x(24).y(66),
            pin(mark).x(24).y(320),
        ]
        .width(Fill)
        .height(Fill)
        .clip(true)
    }

    fn comparison(&self, preset: Preset) -> impl Widget<Message> {
        column![
            row![
                text(preset.label()).size(23),
                text(match preset {
                    Preset::Regular => "Soft separation",
                    Preset::Clear => "More of the view",
                })
                .size(14)
                .color(MUTED),
            ]
            .spacing(12)
            .align_y(iced::Center),
            responsive(move |size| self.specimen(preset, size.width)).height(402),
        ]
        .spacing(12)
        .width(Fill)
    }

    fn view(&self) -> impl Widget<Message> {
        let optics = row![
            column![
                text(format!("Refraction  ·  {:.0}%", self.refraction * 100.0)).size(14),
                slider(0.0..=2.0, self.refraction, Message::Refraction)
                    .step(0.05)
                    .height(40),
            ]
            .width(210),
            column![
                text(format!("Depth  ·  {:.0}%", self.depth * 100.0)).size(14),
                slider(0.0..=2.0, self.depth, Message::Depth)
                    .step(0.05)
                    .height(40),
            ]
            .width(210),
            column![
                text(format!("Light angle  ·  {:.0}°", self.light_angle)).size(14),
                slider(-180.0..=180.0, self.light_angle, Message::LightAngle)
                    .step(5.0)
                    .height(40),
            ]
            .width(210),
            column![
                text(format!("Tint strength  ·  {:.0}%", self.tint * 100.0)).size(14),
                slider(0.0..=2.0, self.tint, Message::Tint)
                    .step(0.05)
                    .height(40),
            ]
            .width(210),
        ]
        .spacing(26)
        .wrap();
        let rendering = row![
            text("Rendering").size(14),
            pick_list(Some(self.quality), RenderQuality::ALL, |quality| quality
                .label()
                .to_owned())
            .on_select(Message::Quality)
            .padding([12, 16]),
            checkbox(self.interactive)
                .label("Respond to hover and press")
                .on_toggle(Message::Interactive)
                .line_height(iced::Pixels(40.0)),
            button(if self.reduce_motion {
                "Motion reduced"
            } else if self.live {
                "Pause scenery"
            } else {
                "Resume scenery"
            })
            .on_press_maybe((!self.reduce_motion).then_some(Message::ToggleMotion))
            .height(40)
            .padding([8, 16])
            .style(button::secondary),
        ]
        .spacing(18)
        .align_y(iced::Center)
        .wrap();
        let accessibility = row![
            checkbox(self.reduce_motion)
                .label("Reduce motion")
                .on_toggle(Message::ReduceMotion)
                .line_height(iced::Pixels(40.0)),
            checkbox(self.reduced_transparency)
                .label("Reduce transparency")
                .on_toggle(Message::ReducedTransparency)
                .line_height(iced::Pixels(40.0)),
            checkbox(self.high_contrast)
                .label("Increase contrast")
                .on_toggle(Message::HighContrast)
                .line_height(iced::Pixels(40.0)),
        ]
        .spacing(26)
        .wrap();

        container(scrollable(
            column![
                text("Glass materials").size(36),
                text("Two ways to frame a view. Move, scroll, and make yourself a note.")
                    .size(16)
                    .color(MUTED),
                optics,
                rendering,
                row![
                    self.comparison(Preset::Regular),
                    self.comparison(Preset::Clear)
                ]
                .spacing(20),
                text("Scroll either scene. Hover or hold a glass button, then try the keyboard.")
                    .size(14)
                    .color(MUTED),
                text("Comfort & clarity").size(22),
                accessibility,
                text("Reduce motion pauses the scenery too. Notes and buttons remain usable.")
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
    let (width, height) = (640_u32, 240_u32);
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let ridge = 95.0 + (x as f32 * 0.014).sin() * 32.0;
            let shoreline = 158.0 + (x as f32 * 0.023).cos() * 16.0;
            let color = if (y as f32) < ridge {
                [177, 207, 215]
            } else if (y as f32) < shoreline {
                [91, 134, 119]
            } else if y % 13 < 2 {
                [231, 224, 181]
            } else {
                [123, 164, 175]
            };
            pixels.extend_from_slice(&[color[0], color[1], color[2], 255]);
        }
    }
    image::Handle::from_rgba(width, height, pixels)
}

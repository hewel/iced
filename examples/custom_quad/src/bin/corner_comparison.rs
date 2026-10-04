//! Visual prototype, not a renderer change. Figma-style geometry follows
//! https://www.figma.com/blog/desperately-seeking-squircles/ and Lisse's
//! packages/core/src/corner-params.ts. Only the unconstrained, uniform-radius
//! case is shown; specimen sizes keep (1 + smoothing) * radius within budget.
use iced::widget::{button, column, container, row, scrollable, slider, stack, svg, text, toggler};
use iced::{Border, Color, Element, Fill, Task, Theme, Widget};
use std::fmt::Write as _;

const WIDTH: f32 = 280.0;
const HEIGHT: f32 = 180.0;
const INK: Color = Color::from_rgb(0.08, 0.14, 0.20);

fn main() -> iced::Result {
    iced::application(Comparison::new, Comparison::update, Comparison::view)
        .title("Corner comparison | Iced / Figma")
        .theme(|state: &Comparison| {
            if state.dark {
                Theme::Dark
            } else {
                Theme::Light
            }
        })
        .window_size((1120, 900))
        .run()
}

struct Comparison {
    radius: f32,
    smoothing: f32,
    matched: bool,
    dark: bool,
    outline: bool,
    figma_card: svg::Handle,
    figma_button: svg::Handle,
    overlay: svg::Handle,
    status: String,
    capture: Option<String>,
}

#[derive(Debug, Clone)]
enum Message {
    Radius(f32),
    Smoothing(f32),
    Matched(bool),
    Dark(bool),
    Outline(bool),
    Reset,
    Capture,
    Captured(iced::window::screenshot::Screenshot),
    Saved(Result<String, String>),
}

impl Comparison {
    fn new() -> (Self, Task<Message>) {
        let args: Vec<String> = std::env::args().collect();
        let empty =
            || svg::Handle::from_memory(b"<svg xmlns='http://www.w3.org/2000/svg'/>".to_vec());
        let mut state = Self {
            radius: 32.0,
            smoothing: 0.6,
            matched: args.iter().any(|arg| arg == "--footprint"),
            dark: args.iter().any(|arg| arg == "--dark"),
            outline: args.iter().any(|arg| arg == "--outline"),
            figma_card: empty(),
            figma_button: empty(),
            overlay: empty(),
            status: "Prototype: compare the silhouette, not backend antialiasing.".into(),
            capture: args
                .iter()
                .find_map(|arg| arg.strip_prefix("--capture=").map(str::to_owned)),
        };
        state.rebuild();
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

    fn figma_radius(&self, radius: f32) -> f32 {
        if self.matched {
            radius / (1.0 + self.smoothing)
        } else {
            radius
        }
    }

    fn rebuild(&mut self) {
        let fill = if self.outline {
            "none"
        } else if self.dark {
            "#aac8ee"
        } else {
            "#bfd4ef"
        };
        let stroke = if self.outline {
            if self.dark { "#c4d8ee" } else { "#344d67" }
        } else {
            "none"
        };
        let specimen = |w, h, radius| {
            let d = figma_path(w, h, self.figma_radius(radius), self.smoothing);
            // SVG strokes are centered; keep only the inward half to match
            // the native quad's 0.5 px inside border, including at the corners.
            svg::Handle::from_memory(format!(
                "<svg xmlns='http://www.w3.org/2000/svg' width='{w}' height='{h}' viewBox='0 0 {w} {h}'><defs><path id='shape' d='{d}'/><clipPath id='inside'><use href='#shape'/></clipPath></defs><use href='#shape' fill='{fill}'/><use href='#shape' fill='none' stroke='{stroke}' stroke-width='1' clip-path='url(#inside)'/></svg>"
            ).into_bytes())
        };
        (self.figma_card, self.figma_button) = (
            specimen(WIDTH, HEIGHT, self.radius),
            specimen(220.0, 64.0, self.radius * 0.35),
        );
        let ours = superellipse_path(WIDTH, HEIGHT, self.radius, self.smoothing);
        let figma = figma_path(
            WIDTH,
            HEIGHT,
            self.figma_radius(self.radius),
            self.smoothing,
        );
        let grid = if self.dark { "#3c414b" } else { "#dce1e6" };
        let mut guides = String::new();
        for i in 0..=8 {
            let x = 200 + i * 10;
            let y = i * 10;
            write!(guides, "<path d='M{x} 0V80 M200 {y}H280' />").unwrap();
        }
        self.overlay = svg::Handle::from_memory(format!(
            "<svg xmlns='http://www.w3.org/2000/svg' width='300' height='300' viewBox='199 -1 82 82'><g fill='none' stroke='{grid}' stroke-width='.2'>{guides}</g><path d='{ours}' fill='none' stroke='#238a72' stroke-width='.65'/><path d='{figma}' fill='none' stroke='#db704a' stroke-width='.65' stroke-dasharray='1.8 1.1'/></svg>"
        ).into_bytes());
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Radius(value) => self.radius = value,
            Message::Smoothing(value) => self.smoothing = value,
            Message::Matched(value) => self.matched = value,
            Message::Dark(value) => self.dark = value,
            Message::Outline(value) => self.outline = value,
            Message::Reset => {
                self.radius = 32.0;
                self.smoothing = 0.6;
                self.matched = false;
                self.outline = false;
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
                    .unwrap_or_else(|| "corner-comparison.png".into());
                return Task::perform(
                    async move {
                        image::save_buffer(
                            &path,
                            &screenshot.rgba,
                            screenshot.size.width,
                            screenshot.size.height,
                            image::ColorType::Rgba8,
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
        self.rebuild();
        println!(
            "radius={:.1} smoothing={:.2} matched={} dark={} outline={}",
            self.radius, self.smoothing, self.matched, self.dark, self.outline
        );
        Task::none()
    }

    fn specimen(&self, figma: bool, smoothing: f32, compact: bool) -> impl Widget<Message> {
        let (w, h, radius) = if compact {
            (220.0, 64.0, self.radius * 0.35)
        } else {
            (WIDTH, HEIGHT, self.radius)
        };
        let background: Element<'_, Message> = if figma {
            svg(if compact {
                self.figma_button.clone()
            } else {
                self.figma_card.clone()
            })
            .width(w)
            .height(h)
            .boxed()
        } else {
            let dark = self.dark;
            let outline = self.outline;
            container(text(""))
                .width(w)
                .height(h)
                .style(move |_| container::Style {
                    background: (!outline).then_some(
                        if dark {
                            Color::from_rgb8(170, 200, 238)
                        } else {
                            Color::from_rgb8(191, 212, 239)
                        }
                        .into(),
                    ),
                    border: Border::default()
                        .rounded(radius)
                        .smoothing(smoothing)
                        .width(if outline { 0.5 } else { 0.0 })
                        .color(if dark {
                            Color::from_rgb8(196, 216, 238)
                        } else {
                            Color::from_rgb8(52, 77, 103)
                        }),
                    ..Default::default()
                })
                .boxed()
        };
        let foreground: Element<'_, Message> = if compact {
            container(text("Continue").size(17))
                .center_x(w)
                .center_y(h)
                .boxed()
        } else {
            container(
                column![
                    text("A quieter workspace").size(23),
                    text("Make room for what matters.").size(15)
                ]
                .spacing(12),
            )
            .padding(26)
            .center_y(h)
            .width(w)
            .boxed()
        };
        let label_color = if self.outline && self.dark {
            Color::WHITE
        } else {
            INK
        };
        stack![
            background,
            container(foreground).style(move |_| container::Style {
                text_color: Some(label_color),
                ..Default::default()
            })
        ]
    }

    fn view(&self) -> impl Widget<Message> {
        let captions = [
            ("Circular", "Baseline / native quad", false, 0.0),
            (
                "Our superellipse",
                "Current renderer / native quad",
                false,
                self.smoothing,
            ),
            (
                "Figma-style",
                "Cubic shoulders + circular arc / SVG",
                true,
                self.smoothing,
            ),
        ];
        let mut previews: iced::widget::Row<Element<'_, Message>> = row![].spacing(40);
        for (title, subtitle, figma, smoothing) in captions {
            let radius = if figma {
                self.figma_radius(self.radius)
            } else {
                self.radius
            };
            let footprint = if figma {
                (1.0 + self.smoothing) * radius
            } else {
                radius
            };
            previews = previews.push(
                column![
                    text(title).size(23),
                    text(subtitle).size(12),
                    self.specimen(figma, smoothing, false),
                    container(self.specimen(figma, smoothing, true)).center_x(WIDTH),
                    text(format!(
                        "r = {radius:.1}   edge footprint = {footprint:.1} px"
                    ))
                    .size(13),
                ]
                .spacing(18)
                .width(WIDTH)
                .boxed(),
            );
        }
        let controls = column![
            text("Adjust the comparison").size(21),
            text(format!("Radius   {:.0} px", self.radius)).size(15),
            slider(8.0..=44.0, self.radius, Message::Radius).step(1.0),
            text(format!(
                "Smoothing   {:.0}%    /    our exponent n = {:.2}",
                self.smoothing * 100.0,
                2.0 + 2.0 * self.smoothing
            ))
            .size(15),
            slider(0.0..=1.0, self.smoothing, Message::Smoothing).step(0.01),
            toggler(self.matched)
                .label("Match edge footprint, not radius")
                .on_toggle(Message::Matched),
            toggler(self.outline)
                .label("Outline only")
                .on_toggle(Message::Outline),
            toggler(self.dark)
                .label("Dark background")
                .on_toggle(Message::Dark),
            row![
                button("Reset 60%").on_press(Message::Reset),
                button("Save PNG").on_press(Message::Capture)
            ]
            .spacing(12),
        ]
        .spacing(13)
        .width(430);
        let detail = column![
            text("Same corner, overlaid").size(21),
            text("Green: ours   /   dashed orange: Figma-style").size(13),
            svg(self.overlay.clone()).width(240).height(240),
        ]
        .spacing(12);
        let mode_note = if self.matched {
            "Equal footprint: Figma radius is divided by (1 + smoothing). Compare curvature distribution."
        } else {
            "Equal radius: Figma uses a longer transition. Compare the complete silhouette first."
        };
        container(scrollable(column![
            text("Which corner feels right?").size(34),
            text("Identical content and color. Only the corner construction changes.").size(16),
            container(previews).padding(iced::Padding { top: 22.0, bottom: 18.0, ..iced::Padding::ZERO }),
            text(mode_note).size(14),
            container(row![controls, detail].spacing(100)).padding(iced::Padding { top: 22.0, ..iced::Padding::ZERO }),
            text("Button radius = 35% of card radius. Overlay samples our analytic contour; cards use the actual native renderer.").size(12),
            text(&self.status).size(12),
        ].spacing(12))).padding(32).width(Fill).height(Fill)
    }
}

// Canonical corner enters along +x and exits along +y. Its starting point is
// (width - p, 0); rotate the same geometry around the four rectangle corners.
fn corner_point(w: f32, h: f32, p: f32, corner: usize, x: f32, y: f32) -> (f32, f32) {
    match corner {
        0 => (w - p + x, y),
        1 => (w - y, h - p + x),
        2 => (p - x, h - y),
        _ => (y, p - x),
    }
}

fn figma_path(w: f32, h: f32, r: f32, s: f32) -> String {
    let p = (1.0 + s) * r;
    let beta = std::f32::consts::FRAC_PI_4 * s;
    let arc = (std::f32::consts::FRAC_PI_4 * (1.0 - s)).sin() * r * 2.0_f32.sqrt();
    let c = r * (beta / 2.0).tan() * beta.cos();
    let d = c * beta.tan();
    let b = (p - arc - c - d) / 3.0;
    let a = 2.0 * b;
    let mut path = format!("M {} 0", w - p);
    for corner in 0..4 {
        let point = |x, y| corner_point(w, h, p, corner, x, y);
        let (x, y) = point(0.0, 0.0);
        write!(path, " L {x} {y}").unwrap();
        let (x1, y1) = point(a, 0.0);
        let (x2, y2) = point(a + b, 0.0);
        let (x3, y3) = point(a + b + c, d);
        write!(path, " C {x1} {y1} {x2} {y2} {x3} {y3}").unwrap();
        if s < 1.0 {
            let (x, y) = point(p - d, p - a - b - c);
            write!(path, " A {r} {r} 0 0 1 {x} {y}").unwrap();
        }
        let (x1, y1) = point(p, p - a - b);
        let (x2, y2) = point(p, p - a);
        let (x3, y3) = point(p, p);
        write!(path, " C {x1} {y1} {x2} {y2} {x3} {y3}").unwrap();
    }
    path.push_str(" Z");
    path
}

fn superellipse_path(w: f32, h: f32, r: f32, s: f32) -> String {
    let power = 2.0 / (2.0 + 2.0 * s);
    let mut path = format!("M {} 0", w - r);
    for corner in 0..4 {
        for i in 0..=256 {
            let (x, y) = if i == 0 {
                (0.0, 0.0)
            } else if i == 256 {
                (r, r)
            } else {
                let angle = std::f32::consts::FRAC_PI_2 * i as f32 / 256.0;
                (
                    r * angle.sin().powf(power),
                    r * (1.0 - angle.cos().powf(power)),
                )
            };
            let (x, y) = corner_point(w, h, r, corner, x, y);
            write!(path, " L {x} {y}").unwrap();
        }
    }
    path.push_str(" Z");
    path
}

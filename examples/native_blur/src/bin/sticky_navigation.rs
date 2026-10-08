//! Sticky navigation and a clear-to-soft blur along the bottom viewport edge.
use iced::widget::{
    backdrop, button, checkbox, column, container, image, operation, pin, responsive, row,
    scrollable, slider, space, stack, text,
};
use iced::{Blur, Border, Center, Color, ContentFit, Fill, Font, Task, Theme, Widget, gradient};

const PAGE: &str = "sticky-page";
const HERO_HEIGHT: f32 = 260.0;
const NAV_HEIGHT: f32 = 72.0;
const FADE_HEIGHT: f32 = 160.0;
const BOTTOM_FADE_HEIGHT: f32 = 260.0;
const BOTTOM_CONTROLS_HEIGHT: f32 = 88.0;
const SCROLLBAR_WIDTH: f32 = 14.0;
const CARD_HEIGHT: f32 = 300.0;
const GAP: f32 = 24.0;
const INK: Color = Color::from_rgb8(30, 49, 48);
const MUTED: Color = Color::from_rgb8(95, 113, 110);
const PAPER: Color = Color::from_rgb8(241, 244, 235);

fn main() -> iced::Result {
    iced::application(Notebook::default, Notebook::update, Notebook::view)
        .title("Sticky navigation · progressive backdrop")
        .theme(Theme::Light)
        .window_size((1040, 820))
        .run()
}

struct Notebook {
    artwork: Vec<image::Handle>,
    offset: f32,
    strength: f32,
    blur: bool,
}

impl Default for Notebook {
    fn default() -> Self {
        Self {
            artwork: (0..3).map(artwork).collect(),
            offset: 0.0,
            strength: 20.0,
            blur: true,
        }
    }
}

#[derive(Debug, Clone)]
enum Message {
    Scrolled(scrollable::Scroll),
    Jump(usize),
    Top,
    Blur(bool),
    Strength(f32),
}

impl Notebook {
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Scrolled(scroll) => self.offset = scroll.viewport.absolute_offset().y,
            Message::Blur(enabled) => self.blur = enabled,
            Message::Strength(strength) => self.strength = strength,
            Message::Jump(section) => {
                // The target card starts just below the sharp navigation row.
                return operation::scrollable::scroll_to(
                    PAGE,
                    scrollable::AbsoluteOffset {
                        x: 0.0,
                        y: HERO_HEIGHT + GAP + section as f32 * 3.0 * (CARD_HEIGHT + GAP),
                    },
                    operation::Animation::Instant,
                );
            }
            Message::Top => {
                return operation::scrollable::snap_to(
                    PAGE,
                    scrollable::RelativeOffset::START,
                    operation::Animation::Instant,
                );
            }
        }
        Task::none()
    }

    fn hero(&self) -> impl Widget<Message> {
        container(
            column![
                text("FIELDNOTES / A SMALL COLLECTION")
                    .size(12)
                    .font(Font::MONOSPACE)
                    .color(MUTED),
                text("Take the scenic route.").size(44),
                text("Scroll down. The navigation follows, then stays at the top.")
                    .size(17)
                    .color(MUTED),
                text(
                    "Watch the bottom edge: the page becomes softer, while the slider stays sharp."
                )
                .size(15)
                .color(MUTED),
            ]
            .spacing(16),
        )
        .padding(32)
        .width(Fill)
        .height(HERO_HEIGHT)
        .center_y(HERO_HEIGHT)
    }

    fn card(&self, index: usize) -> impl Widget<Message> {
        let titles = [
            "A quieter kind of morning",
            "Where the river bends",
            "Collected along the way",
            "Somewhere beyond the city",
            "The long way home",
            "Room for a little detour",
            "Light worth remembering",
            "Small things, kept close",
            "Until next time",
        ];
        let sections = ["JOURNAL", "PLACES", "SAVED"];
        container(column![
            image(&self.artwork[index % self.artwork.len()])
                .width(Fill)
                .height(180)
                .content_fit(ContentFit::Cover)
                .border_radius(iced::border::top(16)),
            container(
                column![
                    text(format!("{} / {:02}", sections[index / 3], index + 1))
                        .size(12)
                        .font(Font::MONOSPACE)
                        .color(MUTED),
                    text(titles[index]).size(27),
                    text("Color, lines, and a fresh perspective.")
                        .size(15)
                        .color(MUTED),
                ]
                .spacing(8),
            )
            .padding([16, 24]),
        ])
        .width(Fill)
        .height(CARD_HEIGHT)
        .clip(true)
        .style(|_| container::Style {
            background: Some(Color::WHITE.into()),
            border: Border::default().rounded(16),
            ..Default::default()
        })
    }

    fn navigation(&self) -> impl Widget<Message> {
        let selected =
            ((self.offset - HERO_HEIGHT - GAP).max(0.0) / (3.0 * (CARD_HEIGHT + GAP))) as usize;
        let links =
            row(["Journal", "Places", "Saved"]
                .into_iter()
                .enumerate()
                .map(|(index, label)| {
                    button(text(label).size(14))
                        .on_press(Message::Jump(index))
                        .height(40)
                        .padding([8, 16])
                        .style(if index == selected.min(2) {
                            button::primary
                        } else {
                            button::text
                        })
                }))
            .spacing(4);

        container(
            row![
                button(text("FIELDNOTES").size(18))
                    .on_press(Message::Top)
                    .height(40)
                    .style(button::text),
                space().width(Fill),
                links,
                checkbox(self.blur)
                    .label("Blur")
                    .on_toggle(Message::Blur)
                    .line_height(iced::Pixels(40.0)),
            ]
            .spacing(20)
            .align_y(Center),
        )
        .padding([16, 24])
        .width(Fill)
        .height(NAV_HEIGHT)
    }

    fn view(&self) -> impl Widget<Message> {
        responsive(|size| self.viewport(size))
    }

    fn viewport(&self, size: iced::Size) -> impl Widget<Message> {
        let cards = column((0..9).map(|index| self.card(index))).spacing(GAP);
        let content = column![
            self.hero(),
            // Reserve the original row's space, so pinning never shifts the page.
            space().height(NAV_HEIGHT),
            container(cards).padding(GAP).width(Fill),
            // Let the final card scroll above the foreground controls.
            space().height(BOTTOM_CONTROLS_HEIGHT),
        ];

        let y = (HERO_HEIGHT - self.offset).max(0.0);
        let strength = if self.blur { self.strength } else { 0.0 };
        // Iced measures gradient angles clockwise from up: 180° points down.
        let tint = gradient::Linear::new(iced::Degrees(180.0))
            .add_stop(0.0, Color::from_rgba(0.945, 0.957, 0.922, 0.45))
            .add_stop(0.5, Color::from_rgba(0.945, 0.957, 0.922, 0.12))
            .add_stop(1.0, Color::TRANSPARENT);
        let fade = backdrop(
            Blur::vertical_gradient(strength, 0.0),
            space().width(Fill).height(FADE_HEIGHT),
        )
        .tint(tint);
        // Zero sigma at the entry edge joins the sharp page continuously.
        // No tint or opaque fill: only the underlying scene becomes softer.
        let bottom_height = BOTTOM_FADE_HEIGHT.min(size.height);
        let bottom_fade = backdrop(
            Blur::vertical_gradient(0.0, strength),
            space().width(Fill).height(bottom_height),
        );
        // These effects belong to the scrollable's content. It draws its native
        // scrollbar afterward, so the bar is neither sampled nor replaced.
        // Positions are in content coordinates; scrolling subtracts `offset`.
        let page = scrollable(stack![
            content,
            pin(bottom_fade).y(self.offset + size.height - bottom_height),
            pin(fade).y(self.offset + y),
        ])
        .direction(scrollable::Direction::Vertical(
            scrollable::Scrollbar::new()
                .width(SCROLLBAR_WIDTH)
                .spacing(0),
        ))
        .id(PAGE)
        .on_scroll(Message::Scrolled)
        .width(Fill)
        .height(Fill);
        let bottom_controls = container(
            container(
                row![
                    text("Edge blur").size(15),
                    slider(0.0..=32.0, self.strength, Message::Strength)
                        .step(1.0)
                        .width(Fill),
                    text(format!("{:02.0} px", self.strength))
                        .size(14)
                        .font(Font::MONOSPACE),
                ]
                .spacing(20)
                .align_y(Center),
            )
            .padding([24, 32])
            .width(Fill)
            .height(BOTTOM_CONTROLS_HEIGHT),
        )
        .width(Fill)
        .align_bottom(Fill);

        // Keep foreground controls out of the embedded scrollbar's hit area.
        let foreground = container(stack![pin(self.navigation()).y(y), bottom_controls])
            .padding(iced::Padding {
                right: SCROLLBAR_WIDTH,
                ..iced::Padding::ZERO
            })
            .width(Fill)
            .height(Fill);
        container(stack![page, foreground].width(Fill).height(Fill).clip(true)).style(|_| {
            container::Style {
                background: Some(PAPER.into()),
                text_color: Some(INK),
                ..Default::default()
            }
        })
    }
}

// A self-contained patterned landscape; no network or external image assets.
fn artwork(variant: usize) -> image::Handle {
    let palettes = [
        [
            [186, 215, 192],
            [44, 116, 106],
            [24, 65, 63],
            [244, 190, 104],
        ],
        [
            [229, 191, 160],
            [183, 95, 70],
            [97, 63, 68],
            [247, 220, 170],
        ],
        [
            [187, 207, 226],
            [83, 128, 168],
            [38, 64, 104],
            [234, 208, 151],
        ],
    ];
    let palette = palettes[variant];
    let (width, height) = (960_u32, 360_u32);
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let u = x as f32 / width as f32;
            let v = y as f32 / height as f32;
            let ridge = 0.40 + (u * 9.0 + variant as f32).sin() * 0.12;
            let front = 0.70 + (u * 7.0 + 2.0).cos() * 0.14;
            let sun = (u - 0.76).powi(2) + ((v - 0.25) * 0.45).powi(2) < 0.003;
            let mut color = palette[if v > front {
                2
            } else if v > ridge {
                1
            } else if sun {
                3
            } else {
                0
            }];
            // Fine contour lines make the clear-to-soft transition visible.
            if v > ridge && (y + x / 5) % 18 < 2 {
                color = color.map(|channel| (channel as f32 * 0.75) as u8);
            }
            pixels.extend_from_slice(&[color[0], color[1], color[2], 255]);
        }
    }
    image::Handle::from_rgba(width, height, pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_core::layout::{Layout, Limits};
    use iced_core::mouse::Cursor;
    use iced_core::renderer::{Headless, Renderer as _, Style};
    use iced_core::widget::Tree;
    use iced_core::{Rectangle, Size, Widget as _};

    fn render(notebook: &Notebook, renderer: &mut iced::Renderer, size: Size<u32>) -> Vec<u8> {
        let size = Size::new(size.width as f32, size.height as f32);
        let bounds = Rectangle::with_size(size);
        renderer.reset(bounds);
        let mut view = notebook.view();
        let mut tree = Tree::new(&view);
        tree.diff(&mut view);
        view.layout(&mut tree, renderer, &Limits::new(Size::ZERO, size));
        view.draw(
            &tree,
            renderer,
            &Theme::Light,
            &Style { text_color: INK },
            Layout::new(tree.size),
            Cursor::Unavailable,
            &bounds,
        );
        renderer.screenshot(Size::new(size.width as u32, size.height as u32), 1.0, PAPER)
    }

    #[test]
    fn progressive_effects_leave_native_scrollbar_pixels_unchanged() {
        let backend = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "tiny-skia".into());
        let mut renderer = iced::futures::executor::block_on(iced::Renderer::new(
            Default::default(),
            Some(&backend),
        ))
        .expect("requested headless renderer");
        assert_eq!(renderer.name(), backend, "no silent backend fallback");
        let mut notebook = Notebook::default();
        for size in [Size::new(800, 600), Size::new(1040, 820)] {
            notebook.blur = false;
            let sharp = render(&notebook, &mut renderer, size);
            notebook.blur = true;
            let blurred = render(&notebook, &mut renderer, size);
            let mut changed_scene_pixels = 0;
            let mut changed_scrollbar_pixels = 0;
            for y in 0..size.height {
                for x in 0..size.width {
                    let index = ((y * size.width + x) * 4) as usize;
                    if sharp[index..index + 4] != blurred[index..index + 4] {
                        if x >= size.width - SCROLLBAR_WIDTH as u32 {
                            changed_scrollbar_pixels += 1;
                        } else if y > size.height - BOTTOM_FADE_HEIGHT as u32 {
                            changed_scene_pixels += 1;
                        }
                    }
                }
            }
            assert!(
                changed_scene_pixels > 64,
                "the live scene must actually blur"
            );
            assert_eq!(
                changed_scrollbar_pixels, 0,
                "blur changed scrollbar pixels at {size:?}"
            );
        }
    }
}

//! Image-source glass composition used by JellyPilot's measured Hero controls.
//! Numerical headless checks only; no snapshots are written or visually inspected.
#![cfg(all(feature = "image", any(feature = "tiny-skia", feature = "wgpu")))]

use iced::widget::{button, column, container, image, row, text};
use iced_core::image::Renderer as _;
use iced_core::renderer::{Headless, Quad, Renderer as _};
use iced_core::widget::{Id, Operation, Tree};
use iced_core::{
    Color, ContentFit, Degrees, Layout, Point, Rectangle, Size, Vector, Widget, gradient, mouse,
};

const HERO: Size = Size::new(512.0, 256.0);
const SCREEN: Size = Size::new(560.0, 300.0);
const IDS: [&str; 5] = ["details", "favorite", "watchlist", "previous", "next"];

fn renderer() -> iced::Renderer {
    let backend = std::env::var("ICED_TEST_BACKEND").unwrap_or_else(|_| "tiny-skia".into());
    let renderer =
        iced::futures::executor::block_on(iced::Renderer::new(Default::default(), Some(&backend)))
            .expect("requested renderer must be available");
    assert_eq!(renderer.name(), backend, "no silent backend fallback");
    renderer
}

#[derive(Default)]
struct ButtonBounds([Option<Rectangle>; 5]);

impl Operation for ButtonBounds {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle, _viewport: &Rectangle) {
        for (index, name) in IDS.iter().enumerate() {
            if id == Some(&Id::new(*name)) {
                self.0[index] = Some(bounds);
            }
        }
    }
}

fn measured_buttons(renderer: &mut iced::Renderer, details: &str) -> [Rectangle; 5] {
    // The varying label controls its own width. Coordinates come from the real
    // layout operation, as in HeroGlass, instead of estimating text metrics.
    let control = |label, id| {
        container(
            button(text(label))
                .padding([10, 16])
                .height(44)
                .on_press(()),
        )
        .id(id)
    };
    let controls = container(
        column![
            row![
                control(details, IDS[0]),
                control("+", IDS[1]),
                control("Saved", IDS[2]),
            ]
            .spacing(12),
            row![control("<", IDS[3]), control(">", IDS[4])].spacing(16),
        ]
        .spacing(36),
    )
    .padding([42, 28]);
    let mut ui: iced_runtime::UserInterface<'_, (), iced::Theme, iced::Renderer> =
        iced_runtime::UserInterface::build(controls, HERO, Default::default(), renderer);
    let mut bounds = ButtonBounds::default();
    ui.operate(renderer, &mut bounds);
    bounds
        .0
        .map(|bounds| bounds.expect("every laid-out control must be measured"))
}

fn draw_widget(
    renderer: &mut iced::Renderer,
    image: image::Image,
    bounds: Rectangle,
    viewport: &Rectangle,
) {
    <image::Image as Widget<(), iced::Theme, iced::Renderer>>::draw(
        &image,
        &Tree::empty(),
        renderer,
        &iced::Theme::Dark,
        &iced_core::renderer::Style {
            text_color: Color::WHITE,
        },
        Layout::new(bounds.size()).move_to(bounds.position()),
        mouse::Cursor::Unavailable,
        viewport,
    );
}

fn fade() -> gradient::Linear {
    let canvas = Color::from_rgb(0.04, 0.06, 0.1);
    gradient::Linear::new(Degrees(180.0))
        .add_stop(0.0, canvas.scale_alpha(0.0))
        .add_stop(0.72, canvas.scale_alpha(0.97))
        .add_stop(1.0, canvas)
}

fn pixel(bytes: &[u8], size: Size<u32>, point: Point, scale: f32) -> [u8; 4] {
    let x = (point.x * scale).floor() as usize;
    let y = (point.y * scale).floor() as usize;
    bytes[(y * size.width as usize + x) * 4..][..4]
        .try_into()
        .unwrap()
}

#[test]
fn measured_hero_controls_share_one_image_rendition_and_keep_global_sampling() {
    let mut renderer = renderer();
    let source = (0..512 * 256)
        .flat_map(|i| {
            let x = i % 512;
            let y = i / 512;
            if x % 64 < 32 {
                [236, (32 + y / 2) as u8, 24, 255]
            } else {
                [24, (48 + y / 2) as u8, 237, 255]
            }
        })
        .collect::<Vec<_>>();
    let handle = image::Handle::from_rgba(512, 256, source);
    let _allocation = renderer
        .load_image(&handle)
        .expect("load textured Hero source");
    let screen = Rectangle::with_size(SCREEN);
    let mut narrow_width = None;

    for scale in [1.0_f32, 1.5] {
        let physical = Size::new(
            (SCREEN.width * scale) as u32,
            (SCREEN.height * scale) as u32,
        );
        for details in ["Details", "Details and episodes"] {
            let masks = measured_buttons(&mut renderer, details);
            if scale == 1.0 {
                eprintln!("Measured Hero controls ({details}): {masks:?}");
            }
            if details == "Details" {
                narrow_width = Some(masks[0].width);
            } else {
                assert!(masks[0].width > narrow_width.unwrap() + 20.0);
            }
            for (origin, clip) in [
                (Vector::new(12.25, 10.5), screen),
                (
                    Vector::new(12.25, -30.5),
                    Rectangle {
                        x: 65.0,
                        y: 20.0,
                        width: 430.0,
                        height: 160.0,
                    },
                ),
            ] {
                // A complete blurred/tinted Hero is the reference for each
                // button's interior, independent of its local size or position.
                renderer.reset(screen);
                draw_widget(
                    &mut renderer,
                    image(handle.clone())
                        .content_fit(ContentFit::Contain)
                        .blur(10.0)
                        .tint(fade()),
                    Rectangle::new(Point::new(origin.x, origin.y), HERO),
                    &screen,
                );
                let reference = renderer.screenshot(physical, scale, Color::TRANSPARENT);
                let cold = renderer.blur_statistics();
                assert!(cold.image_misses > 0);

                renderer.reset(screen);
                for mask in masks {
                    let image = image(handle.clone())
                        .content_fit(ContentFit::Contain)
                        .display_frame(Rectangle::new(Point::new(-mask.x, -mask.y), HERO))
                        .mask_frame(Rectangle::with_size(mask.size()))
                        .border_radius(14)
                        .border_smoothing(0.6)
                        .blur(10.0)
                        .tint(fade());
                    draw_widget(&mut renderer, image, mask + origin, &clip);
                }
                let actual = renderer.screenshot(physical, scale, Color::TRANSPARENT);
                let warm = renderer.blur_statistics();
                assert_eq!(
                    warm.image_misses, cold.image_misses,
                    "changing five masks, labels, or clip must reuse the full-Hero rendition"
                );
                assert!(warm.image_hits > cold.image_hits);

                let mut compared = 0;
                for mask in masks {
                    let mask = mask + origin;
                    for fraction in [0.25, 0.5, 0.75] {
                        let point = Point::new(mask.x + mask.width * fraction, mask.center_y());
                        if !clip.contains(point) {
                            continue;
                        }
                        let expected = pixel(&reference, physical, point, scale);
                        let found = pixel(&actual, physical, point, scale);
                        let error = expected
                            .iter()
                            .zip(found)
                            .map(|(a, b)| a.abs_diff(b))
                            .max()
                            .unwrap();
                        assert!(
                            error <= 3,
                            "Hero sampling/tint moved with its mask: scale={scale}, label={details}, point={point:?}, expected={expected:?}, actual={found:?}"
                        );
                        compared += 1;
                    }
                    let corner = Point::new(mask.x + 1.0, mask.y + 1.0);
                    if clip.contains(corner) {
                        assert_eq!(
                            pixel(&actual, physical, corner, scale)[3],
                            0,
                            "rounded image/tint must not paint the square corner"
                        );
                    }
                }
                assert!(
                    compared >= 10,
                    "each layout must expose multiple independently placed masks"
                );
                for y in 0..physical.height {
                    for x in 0..physical.width {
                        let point = Point::new((x as f32 + 0.5) / scale, (y as f32 + 0.5) / scale);
                        // The consumer keeps CRISP's physical-pixel snapping;
                        // exclude its one-pixel boundary allowance.
                        if !clip.expand(1.0 / scale).contains(point)
                            || !masks
                                .iter()
                                .any(|mask| (*mask + origin).expand(1.0 / scale).contains(point))
                        {
                            assert_eq!(
                                actual[((y * physical.width + x) * 4 + 3) as usize],
                                0,
                                "drawing leaked outside the visible controls at {point:?}"
                            );
                        }
                    }
                }

                // Catalog fills/labels are submitted afterward in a new layer.
                // This marker must stay sharp and must not trigger more blur work.
                let center = (masks[2] + origin).center();
                let foreground = Rectangle {
                    x: center.x.floor() - 3.0,
                    y: center.y.floor() - 3.0,
                    width: 6.0,
                    height: 6.0,
                };
                renderer.with_layer(clip, |renderer| {
                    renderer.fill_quad(
                        Quad {
                            bounds: foreground,
                            ..Default::default()
                        },
                        Color::WHITE,
                    );
                });
                let foreground_pixels = renderer.screenshot(physical, scale, Color::TRANSPARENT);
                assert_eq!(
                    pixel(&foreground_pixels, physical, foreground.center(), scale),
                    [255; 4]
                );
                assert_eq!(renderer.blur_statistics().image_misses, warm.image_misses);
            }
        }
    }
}

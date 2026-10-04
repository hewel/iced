# Examples
__Iced moves fast and the `master` branch can contain breaking changes!__ If you want to browse examples that are compatible with the latest release,
then [switch to the `latest` branch](https://github.com/iced-rs/iced/tree/latest/examples#examples).

## [Tour](tour)
A simple UI tour that can run both on native platforms and the web! It showcases different widgets that can be built using Iced.

The __[`main`](tour/src/main.rs)__ file contains all the code of the example! All the cross-platform GUI is defined in terms of __state__, __messages__, __update logic__ and __view logic__.

<div align="center">
  <a href="https://iced.rs/examples/tour.mp4">
    <img src="https://iced.rs/examples/tour.gif">
  </a>
</div>

[`iced_winit`]: ../winit
[`iced_native`]: ../native
[`iced_wgpu`]: ../wgpu
[`iced_web`]: https://github.com/iced-rs/iced_web
[`winit`]: https://github.com/rust-windowing/winit
[`wgpu`]: https://github.com/gfx-rs/wgpu

You can run the native version with `cargo run`:
```
cargo run --package tour
```

## [Todos](todos)
A todos tracker inspired by [TodoMVC]. It showcases dynamic layout, text input, checkboxes, scrollables, icons, and async actions! It automatically saves your tasks in the background, even if you did not finish typing them.

The example code is located in the __[`main`](todos/src/main.rs)__ file.

<div align="center">
  <a href="https://iced.rs/examples/todos.mp4">
    <img src="https://iced.rs/examples/todos.gif" height="400px">
  </a>
</div>

You can run the native version with `cargo run`:
```
cargo run --package todos
```

[TodoMVC]: http://todomvc.com/

## [Game of Life](game_of_life)
An interactive version of the [Game of Life], invented by [John Horton Conway].

It runs a simulation in a background thread while allowing interaction with a `Canvas` that displays an infinite grid with zooming, panning, and drawing support.

The relevant code is located in the __[`main`](game_of_life/src/main.rs)__ file.

<div align="center">
  <img src="https://iced.rs/examples/game_of_life.gif">
</div>

You can run it with `cargo run`:
```
cargo run --package game_of_life
```

[Game of Life]: https://en.wikipedia.org/wiki/Conway%27s_Game_of_Life
[John Horton Conway]: https://en.wikipedia.org/wiki/John_Horton_Conway

## [Styling](styling)
An example showcasing custom styling with a light and dark theme.

The example code is located in the __[`main`](styling/src/main.rs)__ file.

<div align="center">
  <img src="https://iced.rs/examples/styling.gif">
</div>

You can run it with `cargo run`:
```
cargo run --package styling
```

## Corner comparison

Run the isolated native visual prototype:

```sh
cargo run --package custom_quad --bin corner_comparison
```

Compare circular corners, the current fixed-footprint superellipse, and a
Figma-style cubic-shoulder / circular-arc construction. Radius and smoothing
sliders, equal-radius / equal-footprint comparison, outline mode, dark mode,
and window-only PNG capture are available. The first two columns use native
quads; the Figma-style reference uses SVG. This compares silhouettes, not
backend antialiasing or pixel-perfect parity with a Figma export.

`--footprint`, `--dark`, and `--outline` set the initial comparison state.
`--capture=/tmp/corners.png` saves the initial window and exits; pass these
options after Cargo's `--`. The prototype intentionally stays within the
available corner space and does not model saturated radii or capsule blending.
The original `cargo run --package custom_quad` still opens its existing demo.

## Blurred button borders

```sh
cargo run --package custom_quad --bin blur_border
```

This visual reproduction leaves the renderer unchanged. Three identical-source
background rows (dark stripes, light stripes, and high-contrast checks) compare
sharp translucent borders, blur without borders, blurred translucent borders,
and blurred opaque borders. Adjust blur, border opacity/width, and surface tint.
The tint-placement switch compares the existing button-background and
`Image::tint` compositions; it does not select a renderer fix.

Use `--thick` to start with 6 px borders, `--image-tint` for the full-mask tint
composition, or `--capture=/tmp/blur-border.png` to save the initial window and
exit. Pass these options after Cargo's `--`. These examples exercise same-source
image-backed glass, not live-scene backdrop blur. Hover does not alter specimen
styles, and each specimen remains a clickable native button.

## Jellypilot BackGlass reproduction

```sh
cargo run --package custom_quad --bin back_glass
```

Reproduces the resting `detail/chrome.rs` glass-button layers with radius 12,
smoothing 0.6, blur sigma 10, a 40% dark fill, and no stroke. The original
left-fade mask remains circular; adjacent diagnostic samples change only that
mask to smoothing 0.6 or omit it. Flat and textured synthetic blue sources
replace the original hero artwork; the hero sampling frame is 960 x 520.
This does not modify Jellypilot or the renderer.

Optional `--font=<ManropeV5VF.ttf>` loads the application's font at weight 300.
`--capture=/tmp/back-glass.png` saves the initial window and exits. Pass options
after Cargo's `--`; the blur slider and Save PNG button work interactively.

## Extras
A bunch of simpler examples exist:

- [`bezier_tool`](bezier_tool), a Paint-like tool for drawing Bézier curves using the `Canvas` widget.
- [`clock`](clock), an application that uses the `Canvas` widget to draw a clock and its hands to display the current time.
- [`color_palette`](color_palette), a color palette generator based on a user-defined root color.
- [`counter`](counter), the classic counter example explained in the [`README`](../README.md).
- [`custom_widget`](custom_widget), a demonstration of how to build a custom widget that draws a circle.
- [`download_progress`](download_progress), a basic application that asynchronously downloads a dummy file of 100 MB and tracks the download progress.
- [`events`](events), a log of native events displayed using a conditional `Subscription`.
- [`geometry`](geometry), a custom widget showcasing how to draw geometry with the `Mesh2D` primitive in [`iced_wgpu`](../wgpu).
- [`integration`](integration), a demonstration of how to integrate Iced in an existing [`wgpu`] application.
- [`pane_grid`](pane_grid), a grid of panes that can be split, resized, and reorganized.
- [`pick_list`](pick_list), a dropdown list of selectable options.
- [`pokedex`](pokedex), an application that displays a random Pokédex entry (sprite included!) by using the [PokéAPI].
- [`progress_bar`](progress_bar), a simple progress bar that can be filled by using a slider.
- [`scrollable`](scrollable), a showcase of various scrollable content configurations.
- [`sierpinski_triangle`](sierpinski_triangle), a [sierpiński triangle](https://en.wikipedia.org/wiki/Sierpi%C5%84ski_triangle) Emulator, use `Canvas` and `Slider`.
- [`solar_system`](solar_system), an animated solar system drawn using the `Canvas` widget and showcasing how to compose different transforms.
- [`stopwatch`](stopwatch), a watch with start/stop and reset buttons showcasing how to listen to time.
- [`svg`](svg), an application that renders the [Ghostscript Tiger] by leveraging the `Svg` widget.

All of them are packaged in their own crate and, therefore, can be run using `cargo`:
```
cargo run --package <example>
```

[`lyon`]: https://github.com/nical/lyon
[PokéAPI]: https://pokeapi.co/
[Ghostscript Tiger]: https://commons.wikimedia.org/wiki/File:Ghostscript_Tiger.svg
[`wgpu`]: https://github.com/gfx-rs/wgpu

## [Coffee]
Since [Iced was born in May 2019], it has been powering the user interfaces in
[Coffee], an experimental 2D game engine.


<div align="center">
  <img src="https://iced.rs/examples/coffee.gif">
</div>

[Iced was born in May 2019]: https://github.com/hecrj/coffee/pull/35
[`ui` module]: https://docs.rs/coffee/0.3.2/coffee/ui/index.html
[Coffee]: https://github.com/hecrj/coffee

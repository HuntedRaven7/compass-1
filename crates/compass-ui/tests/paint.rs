//! The launcher, PAINTED, and the pixels checked.
//!
//! # The tier this fills
//!
//! `docs/rust-engine/RENDER-HARNESSES.md` names three ways to see the launcher
//! and marked the middle one "not built": the browser surrogate is HTML and
//! proves design intent; `iced_test` runs the real widget tree but asserts
//! only structure and layout bounds; pixels existed only in the VM tier,
//! nightly. So nothing on a pull request could catch **a tree that lays out
//! correctly and paints wrong** — a selection colour that stopped applying, a
//! theme that no longer reaches the surface, text drawn in the fill colour.
//!
//! This renders `LauncherApp::view()` through Iced's real renderer and reads
//! the pixels back. It runs in well under a second.
//!
//! # Which backend, stated rather than assumed
//!
//! Iced's renderer is wgpu with a tiny-skia fallback. With no GPU adapter —
//! this container, a stock CI runner — it falls back to tiny-skia, Iced's
//! software rasteriser: the same widget `draw` calls, the same text shaping
//! through cosmic-text, the same primitives, rasterised on the CPU. Every
//! frame records which backend produced it, and [`renderer_is_reported`]
//! prints it, so a run never silently changes what it proved. CI installs a
//! software Vulkan driver so the wgpu path is exercised too.
//!
//! # Invariants, not images
//!
//! A committed PNG compared byte-for-byte reddens whenever a runner changes
//! its font package, and `app/view.rs`'s view tests already reject that for
//! exactly this reason. These assertions are about COLOUR IN REGIONS, tied to the layout
//! bounds the widget tree reports: glyph shapes can move freely without
//! touching them, and a selection that stops painting cannot hide from them.
//!
//! Thresholds were measured before they were set (ADR-0010), on this commit:
//! selection share 0.777 in the selected row against 0.000 in an unselected
//! one; surface share 0.782 in an unselected row; 600+ distinct colours.

use compass_core::apps::AppIndex;
use compass_ui::LauncherApp;
use compass_ui::design::{Appearance, Rgb};
use compass_ui::message::Message;
use compass_ui::theme::Theme;

/// `Simulator::snapshot` renders at a fixed 2x.
const SCALE: f32 = 2.0;
/// Per-channel slack for "this pixel is that colour". Solid fills land
/// exactly; the slack is for the edge of a fill meeting anti-aliased text.
const TOLERANCE: i16 = 6;

struct Frame {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    renderer: String,
}

impl Frame {
    fn px(&self, x: u32, y: u32) -> [u8; 3] {
        let i = ((y * self.width + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2]]
    }

    fn is(p: [u8; 3], c: Rgb) -> bool {
        (i16::from(p[0]) - i16::from(c.r)).abs() <= TOLERANCE
            && (i16::from(p[1]) - i16::from(c.g)).abs() <= TOLERANCE
            && (i16::from(p[2]) - i16::from(c.b)).abs() <= TOLERANCE
    }

    /// The fraction of pixels inside a LOGICAL rectangle that are `colour`.
    fn share(&self, r: iced::Rectangle, colour: Rgb) -> f32 {
        let (x0, y0) = ((r.x * SCALE) as u32, (r.y * SCALE) as u32);
        let x1 = (((r.x + r.width) * SCALE) as u32).min(self.width);
        let y1 = (((r.y + r.height) * SCALE) as u32).min(self.height);
        let (mut hit, mut all) = (0u32, 0u32);
        for y in y0..y1 {
            for x in x0..x1 {
                all += 1;
                if Self::is(self.px(x, y), colour) {
                    hit += 1;
                }
            }
        }
        if all == 0 {
            0.0
        } else {
            hit as f32 / all as f32
        }
    }

    fn whole(&self) -> iced::Rectangle {
        iced::Rectangle {
            x: 0.0,
            y: 0.0,
            width: self.width as f32 / SCALE,
            height: self.height as f32 / SCALE,
        }
    }

    fn distinct_colours(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        for y in 0..self.height {
            for x in 0..self.width {
                seen.insert(self.px(x, y));
            }
        }
        seen.len()
    }
}

/// A launcher showing four results, the first selected.
fn launcher(appearance: Appearance) -> (tempfile::TempDir, LauncherApp) {
    launcher_with(appearance, 4)
}

/// A launcher showing `results` results, the first selected.
fn launcher_with(appearance: Appearance, results: usize) -> (tempfile::TempDir, LauncherApp) {
    let dir = tempfile::tempdir().expect("tempdir");
    for i in 0..results {
        std::fs::write(
            dir.path().join(format!("app-{i:02}.desktop")),
            format!("[Desktop Entry]\nType=Application\nName=Application {i:02}\nExec=/bin/true\n"),
        )
        .expect("write a desktop entry");
    }
    let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir.path()).build());
    let _ = app.update(Message::AppearanceChanged(appearance));
    let _ = app.update(Message::QueryChanged("Application".to_owned()));
    (dir, app)
}

/// Paint `app` and return the frame plus the bounds of the named labels.
///
/// Pixels come back through `Snapshot::matches_image`, which writes the frame
/// as PNG when the path is new. That is the public route to the RGBA the
/// snapshot holds privately, and its filename carries the renderer's name.
fn paint(app: &LauncherApp, labels: &[&str]) -> (Frame, Vec<iced::Rectangle>) {
    let out = tempfile::tempdir().expect("tempdir");
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        iced::Size::new(600.0, 400.0),
        app.view(),
    );
    let bounds = labels
        .iter()
        .map(|label| {
            ui.find(*label)
                .unwrap_or_else(|_| panic!("{label:?} is on screen"))
                .bounds()
        })
        .collect();

    // THE APP'S OWN THEME, as the running launcher would use. A fixed
    // `iced::Theme::Dark` here painted the area outside the card in iced's
    // dark base, which sits within tolerance of the dark surface -- so a
    // LIGHT frame measured 6.5% "dark surface" and the palette cross-check
    // failed on the harness rather than on the launcher.
    let snapshot = ui.snapshot(&app.theme()).expect("the frame renders");
    snapshot
        .matches_image(out.path().join("frame"))
        .expect("the frame is written");

    let file = std::fs::read_dir(out.path())
        .expect("the snapshot directory")
        .find_map(|entry| {
            let path = entry.ok()?.path();
            path.extension().is_some_and(|x| x == "png").then_some(path)
        })
        .expect("the snapshot wrote a PNG");
    let renderer = file
        .file_stem()
        .expect("a file stem")
        .to_string_lossy()
        .trim_start_matches("frame-")
        .to_owned();
    let image = image::open(&file).expect("the PNG decodes").to_rgba8();
    let (width, height) = image.dimensions();

    (
        Frame {
            width,
            height,
            rgba: image.into_raw(),
            renderer,
        },
        bounds,
    )
}

/// The backend is printed on every run, and CHECKED when a run asks for one.
///
/// Iced tries wgpu first and falls back to tiny-skia without a word. That
/// is the right behaviour for an application and the wrong one for a test:
/// a job meant to prove the wgpu path could pass on tiny-skia and report
/// nothing amiss. `ICED_TEST_BACKEND` forces the choice (and wgpu with no
/// adapter then panics, loudly); `PAINT_EXPECT_BACKEND` makes this test
/// confirm the backend it got is the one it asked for, so a future change
/// to Iced's fallback cannot quietly reintroduce the substitution.
#[test]
fn renderer_is_reported() {
    let (_dir, app) = launcher(Appearance::Light);
    let (frame, _) = paint(&app, &[]);
    println!("painted by: {}", frame.renderer);
    assert!(
        ["tiny-skia", "wgpu"].contains(&frame.renderer.as_str()),
        "an unrecognised renderer means this harness is measuring something new: {}",
        frame.renderer
    );
    if let Ok(expected) = std::env::var("PAINT_EXPECT_BACKEND") {
        assert_eq!(
            frame.renderer, expected,
            "this run asked for {expected} and was painted by {} -- it proved nothing about {expected}",
            frame.renderer
        );
    }
}

/// Something was actually drawn, and it included anti-aliased text.
///
/// A blank or flat-filled frame — a paint that crashed, a surface drawn
/// with no children — has a handful of colours. Rasterised glyphs have
/// hundreds: every edge pixel is a blend.
#[test]
fn the_frame_is_really_painted() {
    for appearance in [Appearance::Light, Appearance::Dark] {
        let (_dir, app) = launcher(appearance);
        let (frame, _) = paint(&app, &[]);
        let colours = frame.distinct_colours();
        assert!(
            colours > 100,
            "{appearance:?}: {colours} distinct colours is a flat fill, not a launcher with text"
        );
    }
}

/// THE SELECTED ROW PAINTS THE SELECTION COLOUR, AND ONLY IT DOES.
///
/// The case this tier exists for. `iced_test` can prove the row is selected;
/// only pixels can prove the highlight is drawn. Measured: 0.777 selected
/// against 0.000 unselected.
#[test]
fn the_selected_row_paints_the_selection_colour() {
    for appearance in [Appearance::Light, Appearance::Dark] {
        let palette = Theme::System.palette(appearance);
        let (_dir, app) = launcher(appearance);
        let (frame, bounds) = paint(&app, &["Application 00", "Application 02"]);
        let (selected, unselected) = (bounds[0], bounds[1]);

        let on = frame.share(selected, palette.selection);
        let off = frame.share(unselected, palette.selection);
        assert!(
            on > 0.5,
            "{appearance:?}: the selected row is only {on:.3} selection colour"
        );
        assert!(
            off < 0.05,
            "{appearance:?}: an UNSELECTED row is {off:.3} selection colour"
        );
    }
}

/// An unselected row paints the card surface, not something else.
#[test]
fn an_unselected_row_paints_the_surface() {
    for appearance in [Appearance::Light, Appearance::Dark] {
        let palette = Theme::System.palette(appearance);
        let (_dir, app) = launcher(appearance);
        let (frame, bounds) = paint(&app, &["Application 02"]);
        let share = frame.share(bounds[0], palette.surface);
        assert!(
            share > 0.5,
            "{appearance:?}: an unselected row is only {share:.3} surface colour"
        );
    }
}

/// A label paints glyphs: part of its box is NOT the fill behind it.
///
/// Catches text drawn in the fill colour — present in the tree, invisible
/// on screen. Measured: about 22% of a label's box is glyph.
#[test]
fn labels_paint_visible_text() {
    let palette = Theme::System.palette(Appearance::Light);
    let (_dir, app) = launcher(Appearance::Light);
    let (frame, bounds) = paint(&app, &["Application 02"]);
    let glyph = 1.0 - frame.share(bounds[0], palette.surface);
    assert!(
        (0.05..0.6).contains(&glyph),
        "{glyph:.3} of the label box differs from the surface; text is missing or the box is wrong"
    );
}

/// Light and dark each paint THEIR OWN palette.
///
/// Each frame is checked against the other palette's surface too, so a
/// theme that stopped reaching the painter — both frames drawn the same —
/// fails here rather than passing on whichever colour happened to be used.
#[test]
fn each_appearance_paints_its_own_palette() {
    let light = Theme::System.palette(Appearance::Light);
    let dark = Theme::System.palette(Appearance::Dark);
    assert_ne!(
        (light.surface.r, light.surface.g, light.surface.b),
        (dark.surface.r, dark.surface.g, dark.surface.b),
        "the check needs the two surfaces to differ"
    );

    for (appearance, own, other) in [
        (Appearance::Light, light, dark),
        (Appearance::Dark, dark, light),
    ] {
        let (_dir, app) = launcher(appearance);
        let (frame, _) = paint(&app, &[]);
        let whole = frame.whole();
        let mine = frame.share(whole, own.surface);
        let theirs = frame.share(whole, other.surface);
        assert!(
            mine > 0.3,
            "{appearance:?}: only {mine:.3} of the frame is its own surface colour"
        );
        assert!(
            theirs < 0.05,
            "{appearance:?}: {theirs:.3} of the frame is the OTHER appearance's surface"
        );
    }
}

/// `LauncherApp::view()` painted the way the runtime paints a window: the
/// surface cleared to `background`, then the view drawn over it, at the
/// window's own size. Raw RGBA, alpha included, which the PNG route above
/// does not keep apart from the clear colour `Simulator::snapshot` picks
/// (the theme's base, never the app's `style`).
fn paint_surface(app: &LauncherApp, size: iced::Size, background: iced::Color) -> (u32, Vec<u8>) {
    use iced_test::core::renderer::Headless;
    use iced_test::core::{clipboard, mouse, renderer, time, window};
    use iced_test::runtime::user_interface::{Cache, UserInterface};

    let backend = std::env::var("ICED_TEST_BACKEND").ok();
    let mut painter = iced::futures::executor::block_on(iced_test::renderer::Renderer::new(
        iced::Font::with_name("Fira Sans"),
        iced::Pixels(16.0),
        backend.as_deref(),
    ))
    .expect("a headless renderer");
    let theme = app.theme();
    let style = app.style(&theme);
    let mut ui = UserInterface::build(app.view(), size, Cache::default(), &mut painter);
    let mut messages = Vec::new();
    let _ = ui.update(
        &[iced::Event::Window(window::Event::RedrawRequested(
            time::Instant::now(),
        ))],
        mouse::Cursor::Unavailable,
        &mut painter,
        &mut clipboard::Null,
        &mut messages,
    );
    ui.draw(
        &mut painter,
        &theme,
        &renderer::Style {
            text_color: style.text_color,
        },
        mouse::Cursor::Unavailable,
    );
    let width = (size.width * SCALE).round() as u32;
    let height = (size.height * SCALE).round() as u32;
    let rgba = painter.screenshot(iced::Size::new(width, height), SCALE, background);
    (width, rgba)
}

/// THE WINDOW AROUND THE CARD IS TRANSPARENT.
///
/// The window is the card plus room for its shadow, and the card shrinks to
/// its rows, so most of a short list's window is neither. Left to the theme,
/// the runtime cleared it to the palette's background and a light rectangle
/// the size of the window stood behind the card on the desktop. Every pixel
/// below the card's shadow must have alpha 0, in both appearances, while the
/// card itself stays opaque. The control paints the same view over the
/// theme's base colour, which is what the window got before, and must fail
/// the same check.
#[test]
fn the_surface_around_the_card_is_transparent() {
    let size = compass_ui::AppFlags::default().window_config.size;
    for appearance in [Appearance::Light, Appearance::Dark] {
        let (_dir, app) = launcher(appearance);
        let theme = app.theme();
        let clear = app.style(&theme).background_color;
        assert!(clear.a.abs() < f32::EPSILON, "{appearance:?}: {clear:?}");

        let (width, rgba) = paint_surface(&app, size, clear);
        let height = rgba.len() as u32 / 4 / width;
        let alpha = |rgba: &[u8], x: u32, y: u32| rgba[((y * width + x) * 4 + 3) as usize];
        let row_max =
            |rgba: &[u8], y: u32| (0..width).map(|x| alpha(rgba, x, y)).max().unwrap_or(0);

        // The card is the opaque part; its last opaque line is its bottom.
        let card_bottom = (0..height)
            .rev()
            .find(|&y| row_max(&rgba, y) == 255)
            .expect("the card is painted opaque");
        // Past the shadow's offset and twice its blur, nothing of the card
        // reaches.
        let shadow_end = card_bottom
            + ((compass_ui::design::SHADOW_OFFSET_Y + 2.0 * compass_ui::design::SHADOW_BLUR)
                * SCALE) as u32;
        assert!(
            shadow_end + (40.0 * SCALE) as u32 <= height,
            "{appearance:?}: the card fills the window; no band left to check"
        );
        let below = |rgba: &[u8]| {
            (shadow_end..height)
                .map(|y| row_max(rgba, y))
                .max()
                .unwrap_or(0)
        };
        assert_eq!(
            below(&rgba),
            0,
            "{appearance:?}: the window below the card's shadow is not transparent"
        );

        let base = iced::theme::Base::base(&theme).background_color;
        let (_, control) = paint_surface(&app, size, base);
        assert_eq!(
            below(&control),
            255,
            "{appearance:?}: the control (the theme's background) must be opaque there"
        );
    }
}

/// THE CARD'S SHADOW FADES OUT BEFORE EVERY EDGE OF THE WINDOW (#251).
///
/// The window is the card plus `SHADOW_PADDING` on each side, and a long list
/// grows the card to its full height, so the padding is all the room the
/// shadow has below it. A 16 px offset and a 32 px blur needed 48 px there and
/// got 24: the surface's last rows still carried alpha 29 of the shadow, and
/// on the desktop the shadow ended in a hard band along the bottom edge
/// instead of fading out. So, for a short card and a full-height one, in both
/// appearances: the outermost row and column on every side are clear, and
/// below the card, the shadow only ever gets lighter.
#[test]
fn the_card_shadow_fades_out_before_every_edge() {
    let size = compass_ui::AppFlags::default().window_config.size;
    let padding = (f32::from(compass_ui::design::SHADOW_PADDING) * SCALE) as u32;
    for appearance in [Appearance::Light, Appearance::Dark] {
        for results in [4, 30] {
            let (_dir, app) = launcher_with(appearance, results);
            let (width, rgba) = paint_surface(&app, size, iced::Color::TRANSPARENT);
            let height = rgba.len() as u32 / 4 / width;
            let alpha = |x: u32, y: u32| rgba[((y * width + x) * 4 + 3) as usize];
            let centre = width / 2;
            let card_bottom = (0..height)
                .rev()
                .find(|&y| alpha(centre, y) == 255)
                .expect("the card is painted opaque");
            if results > 4 {
                assert!(
                    card_bottom + padding + 2 >= height,
                    "{appearance:?}: a long list must grow the card to the window's padding \
                     (card ends at {card_bottom} of {height}), or this checks the easy case"
                );
            }

            let edges = (0..width)
                .flat_map(|x| [alpha(x, 0), alpha(x, height - 1)])
                .chain((0..height).flat_map(|y| [alpha(0, y), alpha(width - 1, y)]))
                .max()
                .unwrap_or(0);
            assert!(
                edges <= 1,
                "{appearance:?}, {results} results: the shadow reaches the window's edge \
                 (alpha {edges}) and is cut off there"
            );

            let below: Vec<u8> = (card_bottom + 1..height)
                .map(|y| alpha(centre, y))
                .collect();
            assert!(
                below.first().is_some_and(|&first| first >= 20),
                "{appearance:?}, {results} results: no shadow under the card: {below:?}"
            );
            assert!(
                below.windows(2).all(|pair| pair[1] <= pair[0]),
                "{appearance:?}, {results} results: the shadow under the card does not fade \
                 out monotonically: {below:?}"
            );
        }
    }
}

/// EVERY ONBOARDING STEP PAINTS ITS HEADING AND ITS BUTTONS.
///
/// The first-run flow is the first thing a new user sees, and the steps hold
/// buttons that `iced_test` can find without their being visible. Each
/// step's heading and primary button must paint glyphs over the surface, in
/// both appearances. With `COMPASS_UI_SCREENSHOT_DIR` set, every frame is
/// also written there, for review.
#[test]
fn every_onboarding_step_paints_its_heading_and_buttons() {
    use compass_core::onboarding::{Flow, Step};
    let steps = Flow::new(false).count();
    for appearance in [Appearance::Light, Appearance::Dark] {
        let palette = Theme::System.palette(appearance);
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir.path()).build());
        let _ = app.update(Message::AppearanceChanged(appearance));
        app.open_onboarding(dir.path().join("onboarding.json"));
        for position in 0..steps {
            let step = app.onboarding_step().expect("the flow is on screen");
            let primary = if position + 1 == steps {
                "Finish"
            } else {
                "Continue"
            };
            let mut ui = iced_test::Simulator::with_size(
                iced::Settings::default(),
                iced::Size::new(800.0, 600.0),
                app.view(),
            );
            let heading = ui
                .find(step.heading())
                .unwrap_or_else(|_| panic!("{step:?}: no heading"))
                .bounds();
            let button = ui
                .find(primary)
                .unwrap_or_else(|_| panic!("{step:?}: no {primary}"))
                .bounds();
            let snapshot = ui.snapshot(&app.theme()).expect("the frame renders");
            let out = tempfile::tempdir().expect("tempdir");
            snapshot
                .matches_image(out.path().join("frame"))
                .expect("the frame is written");
            if let Some(directory) = std::env::var_os("COMPASS_UI_SCREENSHOT_DIR") {
                let name = format!("onboarding-{}-{}-{step:?}", position + 1, appearance.name());
                let _ = snapshot.matches_image(std::path::PathBuf::from(&directory).join(name));
            }
            drop(ui);
            let file = std::fs::read_dir(out.path())
                .expect("the snapshot directory")
                .find_map(|entry| {
                    let path = entry.ok()?.path();
                    path.extension().is_some_and(|x| x == "png").then_some(path)
                })
                .expect("the snapshot wrote a PNG");
            let image = image::open(&file).expect("the PNG decodes").to_rgba8();
            let (width, height) = image.dimensions();
            let frame = Frame {
                width,
                height,
                rgba: image.into_raw(),
                renderer: String::new(),
            };
            let glyph = 1.0 - frame.share(heading, palette.surface);
            assert!(
                (0.05..0.7).contains(&glyph),
                "{appearance:?} {step:?}: {glyph:.3} of the heading box is not surface"
            );
            let fill = frame.share(button, palette.surface);
            assert!(
                fill < 0.5,
                "{appearance:?} {step:?}: {primary} is {fill:.3} surface, so it is not drawn"
            );
            if step == Step::Extensions {
                // No engine here, so Install fails as it does offline: the
                // reason shows and Continue stays on the card.
                let _ = app.update(Message::OnboardingInstall(0));
                let mut ui = iced_test::Simulator::with_size(
                    iced::Settings::default(),
                    iced::Size::new(800.0, 600.0),
                    app.view(),
                );
                assert!(ui.find("Try Again").is_ok(), "{appearance:?}");
                let continue_button = ui.find("Continue").expect("Continue").bounds();
                assert!(
                    continue_button.y + continue_button.height <= 600.0,
                    "{appearance:?}: the notice pushed Continue off the card"
                );
                // The whole notice, under every recommendation, ends above
                // the footer rather than running off the step's area.
                let notice = app
                    .onboarding_install_notice()
                    .expect("the failure is said")
                    .to_owned();
                let notice = ui.find(notice.as_str()).expect("the notice").bounds();
                assert!(
                    notice.y + notice.height <= continue_button.y - 16.0,
                    "{appearance:?}: the notice ({notice:?}) runs into the footer \
                     ({continue_button:?})"
                );
                if let Some(directory) = std::env::var_os("COMPASS_UI_SCREENSHOT_DIR") {
                    let name = format!(
                        "onboarding-{}-{}-Extensions-failed",
                        position + 1,
                        appearance.name()
                    );
                    let snapshot = ui.snapshot(&app.theme()).expect("the frame renders");
                    let _ = snapshot.matches_image(std::path::PathBuf::from(&directory).join(name));
                }
            }
            if step != Step::Complete {
                let _ = app.update(Message::OnboardingContinue);
            }
        }
    }
}

/// The pixels of `snapshot`, read back through the PNG it writes.
fn frame_of(snapshot: &iced_test::simulator::Snapshot) -> Frame {
    let out = tempfile::tempdir().expect("tempdir");
    snapshot
        .matches_image(out.path().join("frame"))
        .expect("the frame is written");
    let file = std::fs::read_dir(out.path())
        .expect("the snapshot directory")
        .find_map(|entry| {
            let path = entry.ok()?.path();
            path.extension().is_some_and(|x| x == "png").then_some(path)
        })
        .expect("the snapshot wrote a PNG");
    let image = image::open(&file).expect("the PNG decodes").to_rgba8();
    let (width, height) = image.dimensions();
    Frame {
        width,
        height,
        rgba: image.into_raw(),
        renderer: String::new(),
    }
}

/// THE THEME PICKED DURING ONBOARDING IS THE ONE THE PAGE PAINTS.
///
/// No engine here to keep it: the choice still shows at once, behind the
/// heading, in place of the theme the flow opened with.
#[test]
fn the_onboarding_theme_picker_repaints_the_page() {
    use compass_ui::onboarding_page::ThemeOption;
    for appearance in [Appearance::Light, Appearance::Dark] {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir.path()).build());
        let _ = app.update(Message::AppearanceChanged(appearance));
        app.open_onboarding(dir.path().join("onboarding.json"));
        let _ = app.update(Message::OnboardingJump(1));
        let _ = app.update(Message::OnboardingTheme(ThemeOption(Theme::Nord)));
        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            iced::Size::new(800.0, 600.0),
            app.view(),
        );
        let heading = ui.find("Make it your own").expect("the heading").bounds();
        let snapshot = ui.snapshot(&app.theme()).expect("the frame renders");
        if let Some(directory) = std::env::var_os("COMPASS_UI_SCREENSHOT_DIR") {
            let name = format!("onboarding-2-{}-Personalize-nord", appearance.name());
            let _ = snapshot.matches_image(std::path::PathBuf::from(&directory).join(name));
        }
        drop(ui);
        let frame = frame_of(&snapshot);
        let nord = Theme::Nord.palette(appearance).surface;
        let system = Theme::System.palette(appearance).surface;
        let (now, before) = (frame.share(heading, nord), frame.share(heading, system));
        assert!(
            now > 0.3 && before < 0.05,
            "{appearance:?}: the page is {now:.3} Nord and {before:.3} the theme it opened with"
        );
    }
}

/// THE CALCULATOR ANSWER'S ACTION PANEL PAINTS ITS ACTIONS.
///
/// Ctrl+B over the answer at the root opens upstream's panel
/// (`RootCalculatorSection::actionPanel`), and each action's title paints
/// glyphs, in both appearances. With `COMPASS_UI_SCREENSHOT_DIR` set, each
/// frame is also written there.
#[test]
fn the_calculator_answers_panel_paints_its_actions() {
    const TITLES: [&str; 4] = [
        "Copy Result",
        "Copy Question And Answer",
        "Put answer in search bar",
        "Open Calculator History",
    ];
    for appearance in [Appearance::Light, Appearance::Dark] {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir.path()).build());
        let _ = app.update(Message::AppearanceChanged(appearance));
        let _ = app.update(Message::QueryChanged("12*7".to_owned()));
        let _ = app.update(Message::TogglePanel);
        let mut ui = iced_test::Simulator::with_size(
            iced::Settings::default(),
            iced::Size::new(800.0, 600.0),
            app.view(),
        );
        let snapshot = ui.snapshot(&app.theme()).expect("the frame renders");
        if let Some(directory) = std::env::var_os("COMPASS_UI_SCREENSHOT_DIR") {
            let name = format!("calculator-panel-{}", appearance.name());
            let _ = snapshot.matches_image(std::path::PathBuf::from(&directory).join(name));
        }
        let bounds: Vec<iced::Rectangle> = TITLES
            .iter()
            .map(|title| {
                ui.find(*title)
                    .unwrap_or_else(|_| panic!("{appearance:?}: no {title:?} in the panel"))
                    .bounds()
            })
            .collect();
        drop(ui);
        let frame = frame_of(&snapshot);
        let palette = Theme::System.palette(appearance);
        // The first is the selected row, on the selection fill.
        for (title, bounds) in TITLES.iter().zip(bounds).skip(1) {
            let glyph = 1.0 - frame.share(bounds, palette.surface);
            assert!(
                (0.05..0.7).contains(&glyph),
                "{appearance:?}: {glyph:.3} of {title:?} is not surface"
            );
        }
    }
}

/// THE LAUNCHER FITS A SMALL OUTPUT (TIL-04, P-01).
///
/// On a 1280×800 panel at 200 % (640×400 logical) the fixed 768×608 window
/// was larger than the screen, and the compositor centred it, so the search
/// field went off the top. The window is fitted to 624×384 now
/// (`surface::fit`), and in it the card keeps clear of every edge, its
/// shadow included, the search field and the first row are on screen, and
/// the thirty rows scroll rather than push the card past the bottom.
#[test]
fn the_launcher_in_a_window_fitted_to_a_small_output_keeps_inside_it_and_scrolls() {
    let available = iced::Size::new(640.0, 400.0);
    let wanted = compass_ui::AppFlags::default().window_config.size;
    let size = compass_ui::surface::fit(wanted, Some(available));
    assert_eq!(size, iced::Size::new(624.0, 384.0));

    for appearance in [Appearance::Light, Appearance::Dark] {
        let (_dir, app) = launcher_with(appearance, 30);
        let theme = app.theme();
        let clear = app.style(&theme).background_color;
        let (width, rgba) = paint_surface(&app, size, clear);
        let height = rgba.len() as u32 / 4 / width;
        let alpha = |x: u32, y: u32| rgba[((y * width + x) * 4 + 3) as usize];
        let edges = (0..height)
            .flat_map(|y| [(0, y), (width - 1, y)])
            .chain((0..width).flat_map(|x| [(x, 0), (x, height - 1)]));
        let opaque_edge = edges.into_iter().find(|&(x, y)| alpha(x, y) == 255);
        assert_eq!(
            opaque_edge, None,
            "{appearance:?}: the card reaches the window's edge"
        );

        let mut ui = iced_test::Simulator::with_size(iced::Settings::default(), size, app.view());
        for label in ["Application 00", "Application 01"] {
            let found = ui.find(label).expect(label).bounds();
            assert!(
                found.x >= 0.0
                    && found.y >= 0.0
                    && found.x + found.width <= size.width
                    && found.y + found.height <= size.height,
                "{appearance:?}: {label:?} at {found:?} is outside {size:?}"
            );
        }
        let last = ui.find("Application 29").expect("the last row").bounds();
        assert!(
            last.y >= size.height,
            "{appearance:?}: the last row is laid out at {last:?}, inside the window: \
             the list did not scroll"
        );
    }
}

/// A LONG SUBTITLE STAYS ON ITS ROW'S ONE LINE (P-07).
///
/// The row is `row_height` tall and Iced's text wraps, so LibreOffice's
/// comment took two lines and ran into the row below. It is elided now: the
/// subtitle is laid out one line tall, and the title and subtitle together fit
/// the row.
#[test]
fn a_long_subtitle_is_one_line_and_stays_in_its_row() {
    let dir = tempfile::tempdir().expect("tempdir");
    let comment = "Launch applications to create text documents, spreadsheets, presentations, \
                   drawings, formulas, and databases, or open recently used documents.";
    std::fs::write(
        dir.path().join("office.desktop"),
        format!(
            "[Desktop Entry]\nType=Application\nName=Office Suite\nComment={comment}\nExec=/bin/true\n"
        ),
    )
    .expect("write a desktop entry");
    let mut app = LauncherApp::with_index(AppIndex::builder().dir(dir.path()).build());
    let _ = app.update(Message::QueryChanged("Office".to_owned()));
    let mut ui = iced_test::Simulator::with_size(
        iced::Settings::default(),
        iced::Size::new(768.0, 608.0),
        app.view(),
    );
    let title = ui.find("Office Suite").expect("the title").bounds();
    let subtitle = ui.find(comment).expect("the subtitle").bounds();
    let geometry = compass_ui::design::GEOMETRY;
    let line = f32::from(geometry.subtitle_size) * 1.3;
    assert!(
        subtitle.height <= line + 1.0,
        "the subtitle is {} tall: more than one line of {line}",
        subtitle.height
    );
    assert!(
        subtitle.y + subtitle.height - title.y <= f32::from(geometry.row_height),
        "title and subtitle overflow the row: {title:?} {subtitle:?}"
    );
}

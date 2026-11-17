//! What each desktop is told when the wallpaper changes.
//!
//! Read off `WallpaperManager` and its backends
//! (`src/server/src/services/wallpaper/`).

use std::path::Path;

use compass_core::wallpaper::{
    Backend, CANDIDATE_ORDER, CINNAMON_SCHEMA, GNOME_SCHEMA, MATE_SCHEMA, PLASMA_SERVICE,
    SWWW_BINARIES, UNSUPPORTED_MESSAGE, WallpaperEnvironment, WallpaperFit, WallpaperRequest,
    cinnamon_commands, file_uri, gnome_commands, gsettings_picture_option, hyprpaper_command,
    hyprpaper_mode, hyprpaper_spec, kde_fill_mode, mate_commands, no_such_file_message, prepare,
    resolve_backend, swww_command, swww_resize_mode,
};

/// Every fit, for tests that must cover all of them.
const ALL_FITS: &[WallpaperFit] = &[
    WallpaperFit::Cover,
    WallpaperFit::Contain,
    WallpaperFit::Stretch,
    WallpaperFit::Center,
    WallpaperFit::Tile,
];

/// An environment where the named backends work and the file is there.
struct World {
    available: Vec<Backend>,
    file_exists: bool,
}

impl WallpaperEnvironment for World {
    fn is_activatable(&self, backend: Backend) -> bool {
        self.available.contains(&backend)
    }
    fn is_regular_file(&self, _path: &Path) -> bool {
        self.file_exists
    }
}

/// A request for a picture.
fn request(fit: WallpaperFit) -> WallpaperRequest {
    WallpaperRequest {
        path: "/home/ada/Pictures/hills.jpg".to_owned(),
        screen: None,
        fit,
    }
}

#[test]
fn the_default_fit_is_cover() {
    assert_eq!(WallpaperRequest::default().fit, WallpaperFit::Cover);
}

#[test]
fn the_gsettings_vocabulary_is_the_gnome_one() {
    // Shared by GNOME, Cinnamon and MATE, so a wrong word here is three
    // desktops' worth of wrong.
    assert_eq!(gsettings_picture_option(WallpaperFit::Cover), "zoom");
    assert_eq!(gsettings_picture_option(WallpaperFit::Contain), "scaled");
    assert_eq!(gsettings_picture_option(WallpaperFit::Stretch), "stretched");
    assert_eq!(gsettings_picture_option(WallpaperFit::Center), "centered");
    assert_eq!(gsettings_picture_option(WallpaperFit::Tile), "wallpaper");
}

#[test]
fn every_fit_maps_to_a_distinct_gsettings_option() {
    // gsettings is the one backend with a name for all five, so two fits
    // collapsing here would be a mistake rather than a documented degradation.
    let mut options: Vec<_> = ALL_FITS
        .iter()
        .map(|f| gsettings_picture_option(*f))
        .collect();
    options.sort_unstable();
    let count = options.len();
    options.dedup();
    assert_eq!(options.len(), count);
}

#[test]
fn the_kde_fill_modes_are_qts_enum_not_kdes_names() {
    // org.kde.image writes Qt's Image.fillMode values, so these have to match
    // Qt's enum rather than anything in KDE's own documentation.
    assert_eq!(kde_fill_mode(WallpaperFit::Stretch), 0);
    assert_eq!(kde_fill_mode(WallpaperFit::Contain), 1, "PreserveAspectFit");
    assert_eq!(kde_fill_mode(WallpaperFit::Cover), 2, "PreserveAspectCrop");
    assert_eq!(kde_fill_mode(WallpaperFit::Tile), 3);
    assert_eq!(kde_fill_mode(WallpaperFit::Center), 6, "Pad");
}

#[test]
fn swww_degrades_tiling_to_cropping() {
    // swww has no tile mode. Documented in the C++, and a person asking for
    // tile gets a crop rather than an error.
    assert_eq!(swww_resize_mode(WallpaperFit::Tile), "crop");
    assert_eq!(swww_resize_mode(WallpaperFit::Cover), "crop");
    assert_eq!(swww_resize_mode(WallpaperFit::Contain), "fit");
    assert_eq!(swww_resize_mode(WallpaperFit::Stretch), "stretch");
    assert_eq!(swww_resize_mode(WallpaperFit::Center), "no", "native size");
}

#[test]
fn hyprpaper_has_no_name_for_centring() {
    assert_eq!(hyprpaper_mode(WallpaperFit::Center), "");
    assert_eq!(hyprpaper_mode(WallpaperFit::Cover), "cover");
    assert_eq!(hyprpaper_mode(WallpaperFit::Contain), "contain");
    assert_eq!(hyprpaper_mode(WallpaperFit::Stretch), "fill");
    assert_eq!(hyprpaper_mode(WallpaperFit::Tile), "tile");
}

#[test]
fn hyprpaper_spells_stretch_as_fill() {
    // Its "fill" is everyone else's "stretch", and its own "cover" is what
    // most other backends call fill. Getting these two the wrong way round
    // distorts the image with nothing reporting it.
    assert_eq!(hyprpaper_mode(WallpaperFit::Stretch), "fill");
    assert_ne!(hyprpaper_mode(WallpaperFit::Cover), "fill");
}

#[test]
fn a_local_path_becomes_a_file_uri() {
    assert_eq!(
        file_uri("/home/ada/Pictures/hills.jpg"),
        "file:///home/ada/Pictures/hills.jpg"
    );
}

#[test]
fn gnome_sets_the_light_wallpaper_the_dark_one_and_the_fit() {
    let commands = gnome_commands(&request(WallpaperFit::Contain));
    assert_eq!(commands.len(), 3);
    assert_eq!(
        commands[0].args,
        vec![
            "set".to_owned(),
            GNOME_SCHEMA.to_owned(),
            "picture-uri".to_owned(),
            "file:///home/ada/Pictures/hills.jpg".to_owned(),
        ]
    );
    assert_eq!(commands[1].args[2], "picture-uri-dark");
    assert_eq!(commands[2].args[3], "scaled");
}

#[test]
fn only_the_gnome_dark_key_is_allowed_to_fail() {
    // picture-uri-dark exists only from GNOME 42, and treating its absence as
    // an error would fail the whole operation on an older desktop where the
    // wallpaper was actually set.
    let commands = gnome_commands(&request(WallpaperFit::Cover));
    assert!(!commands[0].may_fail);
    assert!(commands[1].may_fail, "the dark key is optional");
    assert!(!commands[2].may_fail);
}

#[test]
fn cinnamon_has_no_dark_key_and_so_nothing_optional() {
    let commands = cinnamon_commands(&request(WallpaperFit::Cover));
    assert_eq!(commands.len(), 2);
    assert!(commands.iter().all(|command| !command.may_fail));
    assert_eq!(commands[0].args[1], CINNAMON_SCHEMA);
}

#[test]
fn mate_takes_a_path_and_not_a_uri() {
    // The one place in this family where passing file:// would silently set
    // nothing: MATE's key is picture-filename.
    let commands = mate_commands(&request(WallpaperFit::Cover));
    assert_eq!(commands[0].args[1], MATE_SCHEMA);
    assert_eq!(commands[0].args[2], "picture-filename");
    assert_eq!(commands[0].args[3], "/home/ada/Pictures/hills.jpg");
    assert!(!commands[0].args[3].starts_with("file://"));
}

#[test]
fn the_three_gsettings_backends_use_three_different_schemas() {
    assert_eq!(GNOME_SCHEMA, "org.gnome.desktop.background");
    assert_eq!(CINNAMON_SCHEMA, "org.cinnamon.desktop.background");
    assert_eq!(MATE_SCHEMA, "org.mate.background");
    assert_eq!(PLASMA_SERVICE, "org.kde.plasmashell");
}

#[test]
fn swww_forces_the_transition_off() {
    // swww fades by default and the other five change instantly, so a person
    // switching desktops would otherwise see a different thing happen.
    let command = swww_command("swww", &request(WallpaperFit::Cover));
    let position = command
        .args
        .iter()
        .position(|arg| arg == "--transition-type")
        .expect("the transition is set");
    assert_eq!(command.args[position + 1], "none");
}

#[test]
fn swww_puts_the_path_last() {
    let command = swww_command("swww", &request(WallpaperFit::Cover));
    assert_eq!(command.args[0], "img");
    assert_eq!(
        command.args.last().map(String::as_str),
        Some("/home/ada/Pictures/hills.jpg")
    );
}

#[test]
fn swww_targets_a_screen_only_when_one_was_asked_for() {
    let mut targeted = request(WallpaperFit::Cover);
    targeted.screen = Some("DP-1".to_owned());

    let command = swww_command("swww", &targeted);
    let position = command
        .args
        .iter()
        .position(|arg| arg == "--outputs")
        .expect("the output is set");
    assert_eq!(command.args[position + 1], "DP-1");

    let untargeted = swww_command("swww", &request(WallpaperFit::Cover));
    assert!(!untargeted.args.iter().any(|arg| arg == "--outputs"));
}

#[test]
fn awww_is_preferred_over_swww() {
    // A fork that is a drop-in replacement; whoever installed it meant to use
    // it.
    assert_eq!(SWWW_BINARIES, &["awww", "swww"]);
}

#[test]
fn a_hyprpaper_spec_is_monitor_path_mode() {
    let mut targeted = request(WallpaperFit::Contain);
    targeted.screen = Some("DP-1".to_owned());
    assert_eq!(
        hyprpaper_spec(&targeted),
        "DP-1,/home/ada/Pictures/hills.jpg,contain"
    );
}

#[test]
fn an_absent_monitor_and_mode_are_written_as_empty_fields() {
    // Commas and all: the fields are positional, so leaving one out would
    // shift the others.
    assert_eq!(
        hyprpaper_spec(&request(WallpaperFit::Center)),
        ",/home/ada/Pictures/hills.jpg,"
    );
}

#[test]
fn the_hyprpaper_command_goes_through_hyprctl() {
    let command = hyprpaper_command(&request(WallpaperFit::Cover));
    assert_eq!(command.program, "hyprctl");
    assert_eq!(command.args[0], "hyprpaper");
    assert_eq!(command.args[1], "wallpaper");
}

#[test]
fn running_daemons_are_tried_before_desktop_environments() {
    // Someone running swww on GNOME chose swww; setting the GNOME key would
    // put the wallpaper somewhere the daemon paints over.
    let daemons = [Backend::Hyprpaper, Backend::Swww];
    let first_desktop = CANDIDATE_ORDER
        .iter()
        .position(|backend| !daemons.contains(backend))
        .expect("there are desktops");
    assert!(
        CANDIDATE_ORDER[..first_desktop]
            .iter()
            .all(|backend| daemons.contains(backend)),
        "{CANDIDATE_ORDER:?}"
    );
    assert_eq!(CANDIDATE_ORDER[0], Backend::Hyprpaper);
}

#[test]
fn the_first_activatable_backend_wins() {
    let world = World {
        available: vec![Backend::Gnome, Backend::Swww],
        file_exists: true,
    };
    assert_eq!(resolve_backend(&world), Some(Backend::Swww));
}

#[test]
fn a_desktop_with_no_backend_resolves_to_nothing() {
    let world = World {
        available: Vec::new(),
        file_exists: true,
    };
    assert_eq!(resolve_backend(&world), None);
}

#[test]
fn an_unsupported_environment_is_reported_before_a_missing_file() {
    // Otherwise a person on an unsupported desktop is sent to look for a file
    // that is sitting right there.
    let world = World {
        available: Vec::new(),
        file_exists: false,
    };
    assert_eq!(
        prepare(&world, &request(WallpaperFit::Cover)),
        Err(UNSUPPORTED_MESSAGE.to_owned())
    );
}

#[test]
fn a_missing_file_is_reported_by_name() {
    let world = World {
        available: vec![Backend::Gnome],
        file_exists: false,
    };
    assert_eq!(
        prepare(&world, &request(WallpaperFit::Cover)),
        Err(no_such_file_message("/home/ada/Pictures/hills.jpg"))
    );
    assert!(no_such_file_message("/a/b").contains("/a/b"));
}

#[test]
fn a_good_request_names_the_backend_that_will_serve_it() {
    let world = World {
        available: vec![Backend::Gnome],
        file_exists: true,
    };
    assert_eq!(
        prepare(&world, &request(WallpaperFit::Cover)),
        Ok(Backend::Gnome)
    );
}

#[test]
fn the_kde_script_sets_the_image_plugin_image_and_fill_on_every_desktop() {
    let script = compass_core::wallpaper::kde_script(&WallpaperRequest {
        path: "/home/ada/Pictures/hills.jpg".into(),
        screen: None,
        fit: WallpaperFit::Tile,
    });
    assert!(script.contains(r#"d.wallpaperPlugin = "org.kde.image";"#));
    assert!(script.contains(r#"d.writeConfig("Image", "file:///home/ada/Pictures/hills.jpg");"#));
    assert!(script.contains(r#"d.writeConfig("FillMode", 3);"#));
    assert!(script.contains("for (var i = 0; i < ds.length; i++)"));
}

#[test]
fn a_kde_wallpaper_path_cannot_end_the_script_s_string() {
    let path = r#"/tmp/a");evil();("b.png"#;
    let script = compass_core::wallpaper::kde_script(&WallpaperRequest {
        path: path.into(),
        screen: None,
        fit: WallpaperFit::Cover,
    });
    // `evil();` may appear inside the URI, but never right after a closed
    // string, where Plasma would run it.
    assert!(!script.contains(r#"");evil"#), "{script}");
    assert_eq!(script.matches(r#"d.writeConfig("Image", "#).count(), 1);
    let start =
        script.find(r#"d.writeConfig("Image", "#).unwrap() + r#"d.writeConfig("Image", "#.len();
    let rest = &script[start..];
    let end = rest.find(");\n").unwrap();
    let literal: String = serde_json::from_str(&rest[..end]).expect("one JSON string literal");
    assert_eq!(
        url::Url::parse(&literal).unwrap().to_file_path().unwrap(),
        std::path::PathBuf::from(path)
    );
}

#[test]
fn desktops_are_recognised_as_the_cpp_environment_helpers_do() {
    use compass_core::wallpaper::desktop_matches;
    assert_eq!(
        desktop_matches(Backend::Gnome, "ubuntu:GNOME", ""),
        Some(true)
    );
    assert_eq!(
        desktop_matches(Backend::Gnome, "GNOME-Classic:GNOME", ""),
        Some(true)
    );
    assert_eq!(
        desktop_matches(Backend::Gnome, "", "gnome-xorg"),
        Some(true)
    );
    assert_eq!(
        desktop_matches(Backend::Gnome, "sway:wlroots", ""),
        Some(false)
    );
    assert_eq!(
        desktop_matches(Backend::Cinnamon, "X-Cinnamon", ""),
        Some(true)
    );
    assert_eq!(
        desktop_matches(Backend::Cinnamon, "cinnamon", ""),
        Some(true)
    );
    assert_eq!(desktop_matches(Backend::Mate, "MATE", ""), Some(true));
    assert_eq!(desktop_matches(Backend::Mate, "MATEY", ""), Some(false));
    assert_eq!(desktop_matches(Backend::Swww, "GNOME", ""), None);
    assert_eq!(desktop_matches(Backend::Kde, "KDE", ""), None);
}

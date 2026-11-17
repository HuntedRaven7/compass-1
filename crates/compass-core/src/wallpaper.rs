//! Setting the desktop wallpaper, on whichever desktop this is.
//!
//! A port of `WallpaperManager` and its six Linux backends
//! (`src/server/src/services/wallpaper/`), minus the process spawning and the
//! D-Bus calls.
//!
//! # Six desktops, six vocabularies for the same five words
//!
//! Every backend supports roughly the same five fits and every one calls them
//! something different — GNOME says `zoom`, KDE says `2`, swww says `crop`,
//! hyprpaper says `cover`. Getting one of those wrong does not fail: it sets
//! the wallpaper stretched when the person asked for it cropped, and nothing
//! anywhere reports a problem. So the tables are what port, and the tests are
//! a table each.
//!
//! Two of the five are not supported everywhere and degrade rather than
//! failing, which the C++ comments call out and which the tests name: swww has
//! no tile mode and falls back to cropping, and hyprpaper has no centre mode
//! and sends an empty one.

use std::path::Path;

/// How a wallpaper is fitted to the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WallpaperFit {
    /// Fill the screen, cropping the overflow.
    #[default]
    Cover,
    /// Fit the whole image, leaving bars.
    Contain,
    /// Distort to fill exactly.
    Stretch,
    /// Native size, centred.
    Center,
    /// Repeat at native size.
    Tile,
}

/// A request to set the wallpaper.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WallpaperRequest {
    /// The image.
    pub path: String,
    /// Which screen, where the backend can target one.
    pub screen: Option<String>,
    /// How to fit it.
    pub fit: WallpaperFit,
}

/// The `picture-options` value GNOME, Cinnamon and MATE share.
///
/// One function for three desktops because they share the schema vocabulary,
/// which is also why a change here is three desktops' worth of wrong.
#[must_use]
pub const fn gsettings_picture_option(fit: WallpaperFit) -> &'static str {
    match fit {
        WallpaperFit::Cover => "zoom",
        WallpaperFit::Contain => "scaled",
        WallpaperFit::Stretch => "stretched",
        WallpaperFit::Center => "centered",
        WallpaperFit::Tile => "wallpaper",
    }
}

/// KDE's `FillMode`, which is Qt's `Image.fillMode` enum.
///
/// Numbers rather than names because that is what `org.kde.image` writes into
/// its config, so these have to match Qt's enum and not KDE's documentation.
#[must_use]
pub const fn kde_fill_mode(fit: WallpaperFit) -> i32 {
    match fit {
        WallpaperFit::Stretch => 0,
        WallpaperFit::Contain => 1,
        WallpaperFit::Cover => 2,
        WallpaperFit::Tile => 3,
        WallpaperFit::Center => 6,
    }
}

/// swww's `--resize` value.
///
/// There is no tile mode, so tiling degrades to cropping; `no` means native
/// size, which is the closest thing it has to centring.
#[must_use]
pub const fn swww_resize_mode(fit: WallpaperFit) -> &'static str {
    match fit {
        WallpaperFit::Cover | WallpaperFit::Tile => "crop",
        WallpaperFit::Contain => "fit",
        WallpaperFit::Stretch => "stretch",
        WallpaperFit::Center => "no",
    }
}

/// hyprpaper's mode prefix.
///
/// Empty for centring, which hyprpaper has no name for; cover is its implicit
/// default and is spelled out anyway.
#[must_use]
pub const fn hyprpaper_mode(fit: WallpaperFit) -> &'static str {
    match fit {
        WallpaperFit::Contain => "contain",
        WallpaperFit::Tile => "tile",
        WallpaperFit::Cover => "cover",
        WallpaperFit::Stretch => "fill",
        WallpaperFit::Center => "",
    }
}

/// The `org.gnome.desktop.background` schema.
pub const GNOME_SCHEMA: &str = "org.gnome.desktop.background";
/// The `org.cinnamon.desktop.background` schema.
pub const CINNAMON_SCHEMA: &str = "org.cinnamon.desktop.background";
/// The `org.mate.background` schema.
pub const MATE_SCHEMA: &str = "org.mate.background";
/// The Plasma D-Bus service the KDE backend looks for.
pub const PLASMA_SERVICE: &str = "org.kde.plasmashell";

/// A local path as a `file://` URI, which every gsettings backend wants.
#[must_use]
pub fn file_uri(path: &str) -> String {
    format!("file://{path}")
}

/// One command to run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// The program.
    pub program: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// Whether a non-zero exit should abort the rest.
    ///
    /// `picture-uri-dark` is the one that may fail: it only exists from GNOME
    /// 42, and treating its absence as an error would make setting a wallpaper
    /// fail outright on an older desktop where it otherwise worked.
    pub may_fail: bool,
}

impl Command {
    /// A command that must succeed.
    fn required(program: &str, args: &[&str]) -> Self {
        Self {
            program: program.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            may_fail: false,
        }
    }

    /// A command whose failure is ignored.
    fn optional(program: &str, args: &[&str]) -> Self {
        Self {
            may_fail: true,
            ..Self::required(program, args)
        }
    }
}

/// The commands the GNOME backend runs.
///
/// Three, in order: the light wallpaper, the dark one, then the fit. The dark
/// one is optional; the other two are not.
#[must_use]
pub fn gnome_commands(request: &WallpaperRequest) -> Vec<Command> {
    let uri = file_uri(&request.path);
    vec![
        Command::required("gsettings", &["set", GNOME_SCHEMA, "picture-uri", &uri]),
        Command::optional(
            "gsettings",
            &["set", GNOME_SCHEMA, "picture-uri-dark", &uri],
        ),
        Command::required(
            "gsettings",
            &[
                "set",
                GNOME_SCHEMA,
                "picture-options",
                gsettings_picture_option(request.fit),
            ],
        ),
    ]
}

/// The commands the Cinnamon backend runs.
///
/// Two: Cinnamon has no dark-wallpaper key, so there is nothing optional here.
#[must_use]
pub fn cinnamon_commands(request: &WallpaperRequest) -> Vec<Command> {
    let uri = file_uri(&request.path);
    vec![
        Command::required("gsettings", &["set", CINNAMON_SCHEMA, "picture-uri", &uri]),
        Command::required(
            "gsettings",
            &[
                "set",
                CINNAMON_SCHEMA,
                "picture-options",
                gsettings_picture_option(request.fit),
            ],
        ),
    ]
}

/// The commands the MATE backend runs.
///
/// MATE's key is `picture-filename` and takes a *path*, not a URI — the one
/// place in this family where passing `file://` would silently set nothing.
#[must_use]
pub fn mate_commands(request: &WallpaperRequest) -> Vec<Command> {
    vec![
        Command::required(
            "gsettings",
            &["set", MATE_SCHEMA, "picture-filename", &request.path],
        ),
        Command::required(
            "gsettings",
            &[
                "set",
                MATE_SCHEMA,
                "picture-options",
                gsettings_picture_option(request.fit),
            ],
        ),
    ]
}

/// The swww command.
///
/// The transition is forced to `none`: swww fades by default, and the other
/// five backends change instantly, so a person switching desktops would
/// otherwise see a different thing happen.
#[must_use]
pub fn swww_command(binary: &str, request: &WallpaperRequest) -> Command {
    let mut args = vec![
        "img".to_owned(),
        "--transition-type".to_owned(),
        "none".to_owned(),
        "--resize".to_owned(),
        swww_resize_mode(request.fit).to_owned(),
    ];
    if let Some(screen) = &request.screen {
        args.push("--outputs".to_owned());
        args.push(screen.clone());
    }
    args.push(request.path.clone());

    Command {
        program: binary.to_owned(),
        args,
        may_fail: false,
    }
}

/// The binaries the swww backend will use, in preference order.
pub const SWWW_BINARIES: &[&str] = &["awww", "swww"];

/// The hyprpaper wallpaper specification: `monitor,path,mode`.
///
/// An absent monitor and an absent mode are both written as empty fields
/// rather than being left out, so a centred wallpaper on every monitor is
/// `,path,` — commas and all.
#[must_use]
pub fn hyprpaper_spec(request: &WallpaperRequest) -> String {
    let monitor = request.screen.as_deref().unwrap_or("");
    format!("{monitor},{},{}", request.path, hyprpaper_mode(request.fit))
}

/// The hyprpaper command.
#[must_use]
pub fn hyprpaper_command(request: &WallpaperRequest) -> Command {
    Command {
        program: "hyprctl".to_owned(),
        args: vec![
            "hyprpaper".to_owned(),
            "wallpaper".to_owned(),
            hyprpaper_spec(request),
        ],
        may_fail: false,
    }
}

/// Which backend is being used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// hyprpaper, through `hyprctl`.
    Hyprpaper,
    /// swww or awww.
    Swww,
    /// GNOME, through gsettings.
    Gnome,
    /// KDE Plasma, through D-Bus.
    Kde,
    /// Cinnamon, through gsettings.
    Cinnamon,
    /// MATE, through gsettings.
    Mate,
}

/// The order backends are tried in.
///
/// # Running daemons come before desktop environments
///
/// The C++ comment says so and the order is the whole rule: someone running
/// swww on GNOME has chosen swww, and setting the GNOME key instead would put
/// the wallpaper somewhere the daemon is going to paint over. So the two
/// daemons are asked first and the desktops after.
pub const CANDIDATE_ORDER: &[Backend] = &[
    Backend::Hyprpaper,
    Backend::Swww,
    Backend::Gnome,
    Backend::Kde,
    Backend::Cinnamon,
    Backend::Mate,
];

/// What the environment can tell us about each backend.
pub trait WallpaperEnvironment {
    /// Whether `backend` can be used here.
    fn is_activatable(&self, backend: Backend) -> bool;

    /// Whether `path` names a regular file.
    fn is_regular_file(&self, path: &Path) -> bool;
}

/// What the manager says when no backend fits.
pub const UNSUPPORTED_MESSAGE: &str =
    "Setting the wallpaper is not supported in the current environment";

/// What it says when the image is not there.
#[must_use]
pub fn no_such_file_message(path: &str) -> String {
    format!("No such file: {path}")
}

/// The first backend that will work here, if any.
///
/// Resolved once and kept, as the C++ does with `m_resolved`: the answer
/// cannot change without the session changing, and asking six times per
/// wallpaper would mean six process spawns.
#[must_use]
pub fn resolve_backend(environment: &impl WallpaperEnvironment) -> Option<Backend> {
    CANDIDATE_ORDER
        .iter()
        .copied()
        .find(|backend| environment.is_activatable(*backend))
}

/// Check a request before any backend is asked.
///
/// # Errors
///
/// [`UNSUPPORTED_MESSAGE`] when nothing here can set a wallpaper, or
/// [`no_such_file_message`] when the image is missing — checked in that order,
/// so a person on an unsupported desktop is told the real problem rather than
/// being sent to look for a file that is present.
pub fn prepare(
    environment: &impl WallpaperEnvironment,
    request: &WallpaperRequest,
) -> Result<Backend, String> {
    let Some(backend) = resolve_backend(environment) else {
        return Err(UNSUPPORTED_MESSAGE.to_owned());
    };
    if !environment.is_regular_file(Path::new(&request.path)) {
        return Err(no_such_file_message(&request.path));
    }
    Ok(backend)
}

/// The Plasma script the KDE backend evaluates (`org.kde.PlasmaShell.evaluateScript`):
/// every desktop switched to the image plugin with this image and fill mode.
///
/// The path comes from an extension, and a file name may hold any byte but
/// `/` and NUL, so it never goes into the script as text: it becomes a
/// percent-encoded `file://` URI ([`kde_image_uri`]), and that URI a JSON
/// string, which is also a JavaScript string literal. A quote or a `)` in the
/// name stays inside the literal instead of ending it.
#[must_use]
pub fn kde_script(request: &WallpaperRequest) -> String {
    let image =
        serde_json::to_string(&kde_image_uri(&request.path)).unwrap_or_else(|_| "\"\"".to_owned());
    format!(
        r#"
    var ds = desktops();
    for (var i = 0; i < ds.length; i++) {{
      var d = ds[i];
      d.wallpaperPlugin = "org.kde.image";
      d.currentConfigGroup = Array("Wallpaper", "org.kde.image", "General");
      d.writeConfig("Image", {image});
      d.writeConfig("FillMode", {});
    }}
  "#,
        kde_fill_mode(request.fit)
    )
}

/// The image's URI for Plasma: percent-encoded by [`url::Url::from_file_path`]
/// for an absolute path, as the engine encodes paths elsewhere, and
/// [`file_uri`] otherwise.
#[must_use]
pub fn kde_image_uri(path: &str) -> String {
    url::Url::from_file_path(path).map_or_else(|()| file_uri(path), String::from)
}

/// Whether the session is the desktop `backend` belongs to, read from
/// `$XDG_CURRENT_DESKTOP` (colon-separated) and `$GDMSESSION` as the C++
/// `Environment` helpers read them. `None` for the two daemons and KDE,
/// which are found by asking them rather than by name.
///
/// GNOME is any desktop name *containing* `GNOME` (so `ubuntu:GNOME` and
/// `GNOME-Classic` count), or a GDM session containing `gnome`; Cinnamon is
/// `X-Cinnamon` (the pre-spec name Mint uses) or `Cinnamon`; MATE is `MATE`.
#[must_use]
pub fn desktop_matches(backend: Backend, current_desktop: &str, gdm_session: &str) -> Option<bool> {
    let named = |wanted: &str| {
        current_desktop
            .split(':')
            .any(|name| name.eq_ignore_ascii_case(wanted))
    };
    Some(match backend {
        Backend::Gnome => {
            current_desktop.to_ascii_lowercase().contains("gnome")
                || gdm_session.to_ascii_lowercase().contains("gnome")
        }
        Backend::Cinnamon => named("x-cinnamon") || named("cinnamon"),
        Backend::Mate => named("mate"),
        Backend::Hyprpaper | Backend::Swww | Backend::Kde => return None,
    })
}

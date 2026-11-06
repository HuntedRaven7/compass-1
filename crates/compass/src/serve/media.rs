//! The media and volume commands, over MPRIS and `pactl`, and the HUD they
//! answer in.

use std::sync::Arc;

use compass_ipc::{ErrorKind, ProtocolError, Request, Response, WindowCommand};
use tokio::sync::RwLock;

use super::EngineState;
use super::forward;
use super::shell::WindowSlot;

/// The media requests.
pub(super) async fn handle(state: &Arc<RwLock<EngineState>>, request: Request) -> Response {
    match request {
        Request::RunMediaCommand { id } => {
            run_media_command(state.read().await.window_slot(), &id, None).await
        }
        Request::RunMediaCommandWith { id, argument } => {
            run_media_command(state.read().await.window_slot(), &id, argument).await
        }
        Request::ListMediaPlayers => list_media_players().await,
        Request::ControlMediaPlayer { player, action } => {
            control_media_player(&player, action).await
        }
        _ => Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a media request",
        )),
    }
}

/// The player a media command last acted on, which the next one that names
/// none prefers while it is still running.
static LAST_PLAYER: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// `pactl` on the host, bounded by the C++'s timeout.
struct HostPactl;

impl compass_core::audio_control::Pactl for HostPactl {
    fn run(&self, args: &[&str]) -> Option<String> {
        use std::io::Read;
        use wait_timeout::ChildExt;
        let mut child = compass_platform_linux::host_command("pactl")
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let mut stdout = child.stdout.take()?;
        // Read while waiting, so a long sink list cannot fill the pipe.
        let reader = std::thread::spawn(move || {
            let mut out = String::new();
            stdout.read_to_string(&mut out).map(|_| out)
        });
        let timeout = std::time::Duration::from_millis(compass_core::audio_control::TIMEOUT_MS);
        match child.wait_timeout(timeout).ok()? {
            Some(status) if status.success() => reader.join().ok()?.ok(),
            Some(_) => None,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                None
            }
        }
    }
}

/// Runs a volume command through `pactl`, answering with the sentence the
/// C++ puts in its HUD, or refusing with its toast.
fn run_volume_command(id: &str, argument: Option<&str>) -> Result<String, &'static str> {
    use compass_core::media_commands::{
        VOLUME_DOWN_STEP, VOLUME_PRESETS, VOLUME_UP_STEP, mute_message, step_fraction,
        volume_hud_text, volume_step,
    };
    let audio = compass_core::audio_control::PactlAudioControl::new(HostPactl);
    match id {
        "volume-up" | "volume-down" => {
            let default = if id == "volume-up" {
                VOLUME_UP_STEP
            } else {
                VOLUME_DOWN_STEP
            };
            let step = volume_step(argument.map(str::trim), default)?;
            audio
                .adjust_volume(step_fraction(step))
                .map(volume_hud_text)
                .ok_or("Failed to adjust volume")
        }
        "toggle-mute" => {
            if !audio.toggle_mute() {
                return Err("Failed to toggle mute");
            }
            Ok(mute_message(audio.is_muted(), audio.volume()))
        }
        _ => {
            let percent = VOLUME_PRESETS
                .iter()
                .map(|(percent, _)| *percent)
                .find(|percent| id.strip_prefix("volume-") == Some(&percent.to_string()))
                .ok_or("Failed to set volume")?;
            audio
                .set_volume(step_fraction(percent))
                .map(volume_hud_text)
                .ok_or("Failed to set volume")
        }
    }
}

/// Shows what the C++ puts in its HUD: in the launcher's HUD where the
/// window has one (`WindowCommand::Hud`, a layer surface), otherwise as a
/// short transient notification, since an `xdg_toplevel` cannot appear
/// without taking the focus.
pub(super) async fn show_hud(slot: WindowSlot, text: String, icon: Option<&'static str>) {
    let command = WindowCommand::Hud {
        text: text.clone(),
        icon: icon.map(str::to_owned),
    };
    if matches!(forward(&slot, command, "show the HUD").await, Response::Ack) {
        return;
    }
    notify_hud(&text).await;
}

/// The HUD as a transient notification (1.5 s, the HUD's own timer).
async fn notify_hud(text: &str) {
    let shown = notify_rust::Notification::new()
        .appname("Compass")
        .summary(text)
        .hint(notify_rust::Hint::Transient(true))
        .timeout(notify_rust::Timeout::Milliseconds(1500))
        .show_async()
        .await;
    if let Err(err) = shown {
        tracing::info!(%err, text, "HUD not shown");
    }
}

/// Runs a media command over MPRIS, on the player `argument` fuzzy-matches or,
/// when it is empty, on the default one. What the C++ shows in its HUD goes
/// out as a short-lived notification, since the launcher has already hidden.
async fn run_media_command(slot: WindowSlot, id: &str, argument: Option<String>) -> Response {
    use compass_core::media_commands::{self, NoPlayer};
    let refuse =
        |message: String| Response::Error(ProtocolError::new(ErrorKind::Unsupported, message));
    let failed = |message: &str, err: &dyn std::fmt::Display| {
        tracing::warn!(%err, command = id, "media command failed");
        Response::Error(ProtocolError::new(ErrorKind::Internal, message))
    };
    let argument = argument.map(|text| text.trim().to_owned());
    // Without media control, the media extension registers only the volume
    // commands.
    if media_commands::registered_commands(false)
        .iter()
        .any(|command| command == id)
    {
        let owned = id.to_owned();
        let answer =
            tokio::task::spawn_blocking(move || run_volume_command(&owned, argument.as_deref()))
                .await;
        return match answer {
            Ok(Ok(hud)) => {
                show_hud(slot, hud, None).await;
                Response::Ack
            }
            Ok(Err(message)) => Response::Error(ProtocolError::new(ErrorKind::Internal, message)),
            Err(err) => failed("Failed to adjust volume", &err),
        };
    }
    let failure = match id {
        "play-pause" => "Failed to toggle playback",
        "next-track" => "Failed to skip to the next track",
        "previous-track" => "Failed to skip to the previous track",
        _ => {
            return Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                "no media command has that id",
            ));
        }
    };
    let control = match zbus::Connection::session().await {
        Ok(connection) => compass_media::MediaControl::new(connection),
        Err(err) => return failed(failure, &err),
    };
    let found = match control.players().await {
        Ok(found) => found,
        Err(err) => return failed(failure, &err),
    };
    let players: Vec<media_commands::MediaPlayer> = found.iter().map(command_player).collect();
    let query = argument.unwrap_or_default();
    let last = LAST_PLAYER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let default = compass_media::default_player(&found, last.as_deref());
    if players.is_empty() {
        return refuse(media_commands::no_player_message(&NoPlayer::NothingRunning));
    }
    let matches = if query.is_empty() {
        Vec::new()
    } else {
        media_commands::player_matches(&query, &players)
    };
    let index = match media_commands::resolve_player(&query, default, &matches) {
        Ok(index) => index,
        Err(reason) => return refuse(media_commands::no_player_message(&reason)),
    };
    let player = &players[index];
    let (result, hud) = match id {
        "play-pause" => (
            control.play_pause(&player.id).await,
            media_commands::play_pause_message(player),
        ),
        "next-track" => {
            if let Some(refusal) = media_commands::skip_refusal(player, true) {
                return refuse(refusal);
            }
            (control.next(&player.id).await, "Next Track".to_owned())
        }
        _ => {
            if let Some(refusal) = media_commands::skip_refusal(player, false) {
                return refuse(refusal);
            }
            (
                control.previous(&player.id).await,
                "Previous Track".to_owned(),
            )
        }
    };
    if let Err(err) = result {
        return failed(failure, &err);
    }
    *LAST_PLAYER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(player.id.clone());
    let icon = match id {
        "play-pause" if player.playing => "pause",
        "play-pause" => "play",
        "next-track" => "forward",
        _ => "rewind",
    };
    show_hud(slot, hud, Some(icon)).await;
    Response::Ack
}

/// A player as the media commands decide about it.
fn command_player(found: &compass_media::MediaPlayer) -> compass_core::media_commands::MediaPlayer {
    compass_core::media_commands::MediaPlayer {
        id: found.id.clone(),
        identity: found.identity.clone(),
        title: found.title.clone(),
        artist: found.artist.clone(),
        playing: found.status == compass_media::PlaybackStatus::Playing,
        can_go_next: found.can_go_next,
        can_go_previous: found.can_go_previous,
    }
}

/// Now Playing's list: every running player.
async fn list_media_players() -> Response {
    let failed = |err: &dyn std::fmt::Display| {
        tracing::warn!(%err, "listing media players failed");
        Response::Error(ProtocolError::new(
            ErrorKind::Internal,
            "Failed to list media players",
        ))
    };
    let control = match zbus::Connection::session().await {
        Ok(connection) => compass_media::MediaControl::new(connection),
        Err(err) => return failed(&err),
    };
    match control.players().await {
        Ok(players) => Response::MediaPlayers {
            players: players
                .into_iter()
                .map(|player| compass_ipc::MediaPlayerEntry {
                    playing: player.status == compass_media::PlaybackStatus::Playing,
                    paused: player.status == compass_media::PlaybackStatus::Paused,
                    id: player.id,
                    identity: player.identity,
                    app_id: player.app_id,
                    title: player.title,
                    artist: player.artist,
                    can_go_next: player.can_go_next,
                    can_go_previous: player.can_go_previous,
                })
                .collect(),
        },
        Err(err) => failed(&err),
    }
}

/// Now Playing's actions: one player, by its bus name, and no HUD, as the
/// C++'s panel actions call the provider directly.
async fn control_media_player(player: &str, action: compass_ipc::MediaPlayerAction) -> Response {
    use compass_ipc::MediaPlayerAction;
    let failure = match action {
        MediaPlayerAction::PlayPause => "Failed to toggle playback",
        MediaPlayerAction::Next => "Failed to skip to the next track",
        MediaPlayerAction::Previous => "Failed to skip to the previous track",
    };
    if !compass_media::is_player_name(player) {
        return Response::Error(ProtocolError::new(
            ErrorKind::BadRequest,
            "not a media player's bus name",
        ));
    }
    let control = match zbus::Connection::session().await {
        Ok(connection) => compass_media::MediaControl::new(connection),
        Err(err) => {
            tracing::warn!(%err, "media control failed");
            return Response::Error(ProtocolError::new(ErrorKind::Internal, failure));
        }
    };
    let done = match action {
        MediaPlayerAction::PlayPause => control.play_pause(player).await,
        MediaPlayerAction::Next => control.next(player).await,
        MediaPlayerAction::Previous => control.previous(player).await,
    };
    match done {
        Ok(()) => {
            *LAST_PLAYER
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(player.to_owned());
            Response::Ack
        }
        Err(err) => {
            tracing::warn!(%err, "media control failed");
            Response::Error(ProtocolError::new(ErrorKind::Internal, failure))
        }
    }
}

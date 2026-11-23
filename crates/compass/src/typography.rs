//! Feeding the launcher window the desktop's interface typeface.
//!
//! The desktop's font is `org.gnome.desktop.interface font-name` (a Pango
//! description such as `Cantarell 11`). Inside a Flatpak the portal
//! `org.freedesktop.portal.Settings` proxies that key; outside it
//! `gsettings get` reads it directly. The launcher follows the portal when
//! it can and falls back to the command when the portal is not available,
//! so a user who changes their interface font in GNOME Tweaks sees the
//! launcher match without a restart.
//!
//! The same 250 ms budget as `appearance::follow` applies: the first read is
//! a bus round trip, and the launcher must draw before it.

use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use compass_ui::typography::TypographyLink;

/// How long the window waits for the first read before drawing anyway.
const FIRST_READ_BUDGET: Duration = Duration::from_millis(250);

/// Try to read the interface font via `gsettings`.
///
/// Returns the raw Pango description (e.g. `Cantarell 11`) or `None` when
/// `gsettings` is not available or the key is unset. The output of
/// `gsettings get` is a quoted string like `'Cantarell 11'`.
fn gsettings_font() -> Option<String> {
    gsettings_string("org.gnome.desktop.interface", "font-name")
}

fn gsettings_string(schema: &str, key: &str) -> Option<String> {
    let output = Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8(output.stdout).ok()?;
    let trimmed = raw.trim().trim_matches('\'').trim_matches('"').trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

/// The desktop's interface font size in points, when a size was reported.
/// The family is followed live; the size is read once at startup.
#[must_use]
pub(crate) fn interface_font_size() -> Option<f32> {
    gsettings_font()
        .and_then(|description| compass_ui::typography::FontDescription::parse(&description).size)
}

/// The GNOME accent colour name (`blue`, `green`, …), when one is set.
#[must_use]
pub(crate) fn interface_accent() -> Option<String> {
    gsettings_string("org.gnome.desktop.interface", "accent-color")
}

/// The family extracted from a Pango description, if non-empty.
fn family_from_description(description: &str) -> Option<String> {
    let family = compass_ui::typography::family_from_description(description);
    let family = family.trim().to_owned();
    if family.is_empty() {
        None
    } else {
        Some(family)
    }
}

/// Start with the desktop's interface font, if one can be read quickly.
///
/// Returns the family to draw the first frame with, and (when the portal is
/// available) the link later changes arrive on. `None` means no family was
/// discovered in time and the launcher draws with its built-in fallback.
pub(crate) fn follow() -> (Option<String>, Option<TypographyLink>) {
    let (link, sender) = TypographyLink::new();
    let (first_tx, first_rx) = mpsc::sync_channel::<String>(1);

    let spawned = std::thread::Builder::new()
        .name("typography".to_owned())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    tracing::warn!(%error, "no runtime for the typography watcher");
                    // Fallback synchronously so the first frame still gets a font.
                    if let Some(desc) = gsettings_font()
                        && let Some(family) = family_from_description(&desc)
                    {
                        let _ = first_tx.try_send(family.clone());
                        let _ = sender.send(family);
                    }
                    return;
                }
            };
            runtime.block_on(pump(&first_tx, &sender));
        });

    if let Err(error) = spawned {
        tracing::warn!(%error, "could not start the typography watcher");
        // Synchronous gsettings fallback when the thread never started.
        if let Some(desc) = gsettings_font()
            && let Some(family) = family_from_description(&desc)
        {
            return (Some(family), None);
        }
        return (None, None);
    }

    // Give the portal a short budget; a wedged portal costs one default-font
    // frame, not a blocked startup. If it arrives late it is forwarded via
    // the link.
    let first = first_rx.recv_timeout(FIRST_READ_BUDGET).ok();
    // If nothing arrived in time, also try gsettings synchronously as a
    // fallback before giving up — this covers headless tests and machines
    // without a portal.
    let first = first.or_else(|| gsettings_font().and_then(|desc| family_from_description(&desc)));

    // If nothing was discovered but a link was created, keep it: later portal
    // changes will still arrive. If the portal is definitively unavailable the
    // pump will have already returned.
    let link = Some(link);
    // Deduplicate: when first is None we still return the link so a late
    // portal read can fix the font. The caller treats None + Some(link) as
    // "draw default for now, update later".
    (first, link)
}

/// Read once, then follow, sending each answer on.
async fn pump(first: &mpsc::SyncSender<String>, sender: &compass_ui::TypographySender) {
    // Try portal first.
    let portals =
        match compass_portals::Portals::connect(compass_portals::PortalConfig::default()).await {
            Ok(portals) => portals,
            Err(error) => {
                tracing::debug!(%error, "no session bus for typography; trying gsettings");
                if let Some(desc) = gsettings_font()
                    && let Some(family) = family_from_description(&desc)
                {
                    let _ = first.try_send(family.clone());
                    let _ = sender.send(family);
                }
                return;
            }
        };
    portals.probe().await;
    let settings = match portals.settings() {
        Ok(settings) => settings,
        Err(error) => {
            tracing::debug!(%error, "no Settings portal for typography; trying gsettings");
            if let Some(desc) = gsettings_font()
                && let Some(family) = family_from_description(&desc)
            {
                let _ = first.try_send(family.clone());
                let _ = sender.send(family);
            }
            return;
        }
    };

    // Subscribed before the read so a change between the two is not lost.
    let watch = match settings.watch_interface_font().await {
        Ok(watch) => Some(watch),
        Err(error) => {
            tracing::warn!(%error, "could not watch the desktop font");
            None
        }
    };

    let mut portal_family: Option<String> = None;
    match settings.interface_font().await {
        Ok(Some(desc)) => {
            if let Some(family) = family_from_description(&desc) {
                portal_family = Some(family.clone());
                let _ = first.try_send(family.clone());
                let _ = sender.send(family);
            }
        }
        Ok(None) => {
            tracing::debug!("portal returned no interface font; trying gsettings");
        }
        Err(error) => {
            tracing::debug!(%error, "could not read the desktop font via portal");
        }
    }

    // If portal returned nothing, fallback to gsettings for the first frame.
    if portal_family.is_none()
        && let Some(desc) = gsettings_font()
        && let Some(family) = family_from_description(&desc)
    {
        // Only use gsettings when portal gave no answer; portal remains
        // authoritative for live updates.
        if first.try_send(family.clone()).is_ok() {
            let _ = sender.send(family.clone());
        } else {
            // First already had a value (unlikely with None portal); still forward.
            let _ = sender.send(family.clone());
        }
    }

    let Some(watch) = watch else { return };
    let mut watch = Box::pin(watch);
    use futures_util::StreamExt;
    while let Some(desc) = watch.next().await {
        if let Some(family) = family_from_description(&desc)
            && sender.send(family).is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_extraction_strips_size_and_quotes() {
        assert_eq!(
            family_from_description("'Cantarell 11'"),
            Some("Cantarell".to_owned())
        );
        assert_eq!(
            family_from_description("Adwaita Sans 11"),
            Some("Adwaita Sans".to_owned())
        );
        assert_eq!(family_from_description(""), None);
        assert_eq!(family_from_description("   "), None);
    }
}

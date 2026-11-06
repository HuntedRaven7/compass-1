//! Now Playing: the running media players, filtered as the user types, each
//! with the actions `NowPlayingViewHost` offers.
//!
//! The filter uses the C++'s `FuzzySearchable<MediaPlayer>` weights (title
//! 1.0, artist 0.8, identity 0.6); an empty query keeps the bus's order.

use compass_search::{MIN_QUALITY, Query, WeightedField, score_weighted};

use crate::backend::{MediaAction, MediaPlayerRow};

/// What the view is showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// The players are on their way.
    Loading,
    /// Players have arrived (possibly none).
    Ready,
    /// They cannot be listed, and why.
    Failed(String),
}

/// Now Playing's state.
#[derive(Debug, Clone)]
pub struct NowPlayingPage {
    /// The filter text.
    pub query: String,
    /// Every player the engine reported.
    pub all: Vec<MediaPlayerRow>,
    /// Positions in `all` that match `query`, best first.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    /// What the view is showing.
    pub status: Status,
    /// Why the last action did not happen.
    pub notice: Option<String>,
}

impl Default for NowPlayingPage {
    fn default() -> Self {
        Self {
            query: String::new(),
            all: Vec::new(),
            shown: Vec::new(),
            selected: 0,
            status: Status::Loading,
            notice: None,
        }
    }
}

/// The section heading, as `sectionName`.
pub const SECTION: &str = "Players";

/// The search field's placeholder.
pub const PLACEHOLDER: &str = "Search players...";

impl NowPlayingPage {
    /// Takes the engine's answer, keeping the selected player selected when
    /// it is still there, since the list is reloaded after every action.
    pub fn apply(&mut self, result: Result<Vec<MediaPlayerRow>, String>) {
        let keep = self.selected_row().map(|row| row.id.clone());
        match result {
            Ok(rows) => {
                self.all = rows;
                self.status = Status::Ready;
            }
            Err(reason) => {
                self.all.clear();
                self.status = Status::Failed(reason);
            }
        }
        self.refilter();
        if let Some(keep) = keep
            && let Some(position) = self
                .shown
                .iter()
                .position(|&index| self.all[index].id == keep)
        {
            self.selected = position;
        }
    }

    /// Recomputes `shown` for the current query.
    pub fn refilter(&mut self) {
        self.selected = 0;
        let query = Query::new(&self.query);
        if query.is_empty() {
            self.shown = (0..self.all.len()).collect();
            return;
        }
        let mut scored: Vec<(u32, usize)> = self
            .all
            .iter()
            .enumerate()
            .filter_map(|(index, row)| {
                let fields = [
                    WeightedField::new(&row.title, 1.0),
                    WeightedField::new(&row.artist, 0.8),
                    WeightedField::new(&row.identity, 0.6),
                ];
                let found = score_weighted(&fields, &query);
                (found.quality >= MIN_QUALITY && found.score > 0).then_some((found.score, index))
            })
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        self.shown = scored.into_iter().map(|(_, index)| index).collect();
    }

    /// The selected player, if any.
    #[must_use]
    pub fn selected_row(&self) -> Option<&MediaPlayerRow> {
        self.all.get(*self.shown.get(self.selected)?)
    }

    /// What the view says in place of the list: loading, why the players
    /// cannot be listed, that none is running, or that none matches; `None`
    /// while it shows players.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&str> {
        match &self.status {
            Status::Loading => Some("Looking for media players…"),
            Status::Failed(reason) => Some(reason),
            Status::Ready if self.all.is_empty() => Some("No media player is running"),
            Status::Ready if self.shown.is_empty() => Some("No players match"),
            Status::Ready => None,
        }
    }
}

/// A row's title: the track, or the player's name when nothing is loaded.
#[must_use]
pub fn title(row: &MediaPlayerRow) -> &str {
    if row.title.is_empty() {
        &row.identity
    } else {
        &row.title
    }
}

/// The accessory at a row's right: `Playing`, `Paused`, or nothing for a
/// stopped player.
#[must_use]
pub fn accessory(row: &MediaPlayerRow) -> Option<&'static str> {
    if row.playing {
        Some("Playing")
    } else if row.paused {
        Some("Paused")
    } else {
        None
    }
}

/// A row's second line: the artist and the [`accessory`], whichever it has.
#[must_use]
pub fn subtitle(row: &MediaPlayerRow) -> Option<String> {
    match (row.artist.is_empty(), accessory(row)) {
        (false, Some(state)) => Some(format!("{}  ·  {state}", row.artist)),
        (false, None) => Some(row.artist.clone()),
        (true, Some(state)) => Some(state.to_owned()),
        (true, None) => None,
    }
}

/// A row's actions, in the panel's order: Play or Pause, then Next Track and
/// Previous Track when the player offers them.
#[must_use]
pub fn actions(row: &MediaPlayerRow) -> Vec<(&'static str, MediaAction)> {
    let mut out = Vec::with_capacity(3);
    out.push((
        if row.playing { "Pause" } else { "Play" },
        MediaAction::PlayPause,
    ));
    if row.can_go_next {
        out.push(("Next Track", MediaAction::Next));
    }
    if row.can_go_previous {
        out.push(("Previous Track", MediaAction::Previous));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(id: &str, identity: &str, title: &str, artist: &str) -> MediaPlayerRow {
        MediaPlayerRow {
            id: id.into(),
            identity: identity.into(),
            title: title.into(),
            artist: artist.into(),
            ..MediaPlayerRow::default()
        }
    }

    #[test]
    fn a_player_is_found_by_track_artist_or_name_and_stays_selected() {
        let mut page = NowPlayingPage::default();
        page.apply(Ok(vec![
            player("a", "Firefox", "", ""),
            player("b", "Spotify", "Blue Monday", "New Order"),
        ]));
        assert_eq!(page.shown, [0, 1]);
        page.query = "order".into();
        page.refilter();
        assert_eq!(page.selected_row().map(|row| row.id.as_str()), Some("b"));
        page.query.clear();
        page.refilter();
        page.selected = 1;
        page.apply(Ok(vec![
            player("b", "Spotify", "Age of Consent", "New Order"),
            player("a", "Firefox", "", ""),
        ]));
        assert_eq!(
            page.selected_row().map(|row| row.id.as_str()),
            Some("b"),
            "a reload keeps the player selected"
        );
        assert_eq!(title(&page.all[1]), "Firefox");
    }

    #[test]
    fn the_actions_follow_what_the_player_can_do() {
        let mut row = player("a", "Spotify", "T", "A");
        row.playing = true;
        row.can_go_next = true;
        let titles: Vec<&str> = actions(&row).into_iter().map(|(t, _)| t).collect();
        assert_eq!(titles, ["Pause", "Next Track"]);
        assert_eq!(accessory(&row), Some("Playing"));
        row.playing = false;
        row.paused = true;
        row.can_go_previous = true;
        let titles: Vec<&str> = actions(&row).into_iter().map(|(t, _)| t).collect();
        assert_eq!(titles, ["Play", "Next Track", "Previous Track"]);
        assert_eq!(accessory(&row), Some("Paused"));
        row.paused = false;
        assert_eq!(accessory(&row), None);
    }

    #[test]
    fn a_failure_says_why_and_shows_nothing() {
        let mut page = NowPlayingPage::default();
        page.apply(Err("no bus".into()));
        assert!(page.shown.is_empty());
        assert_eq!(page.status, Status::Failed("no bus".into()));
    }
}

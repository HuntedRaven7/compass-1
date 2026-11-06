//! The emoji and symbol picker: its state, and what it decides.
//!
//! The table is `compass_core::glyph`'s, parsed from the committed C++ one at
//! build time. What a person has done with it — visits, pins, a skin tone per
//! glyph and keywords of their own — is `compass_core::glyph_service`'s, kept
//! in the C++'s own file so both engines remember the same things. This is
//! `EmojiGridModel` (`emoji-grid-model.cpp`) over that: the pinned and
//! recently used sections above the table, the search that ranks by a
//! person's own keyword and by how often they picked a glyph, and the action
//! panel with its skin-tone section.

use std::path::PathBuf;

use compass_core::emoji_grid::{self, SkinTone};
use compass_core::glyph::{self, Glyph};
use compass_core::glyph_service::{self, GlyphService, SerializedEmojiMetadata};
use compass_search::Query;

/// The most rows drawn at once. The table holds thousands of entries and a
/// launcher list is read from the top; a query narrows it long before this.
pub const MAX_ROWS: usize = 200;

/// The pinned section's heading.
pub const PINNED_HEADING: &str = "Pinned";

/// The recently used section's heading.
pub const RECENT_HEADING: &str = "Recently used";

/// Action ids, as the panel dispatches them.
pub mod actions {
    /// Copy the glyph, in its tone. Registers a visit.
    pub const COPY: &str = "emoji.copy";
    /// Paste the glyph into the focused window.
    pub const PASTE: &str = "emoji.paste";
    /// Copy its name.
    pub const COPY_NAME: &str = "emoji.copy-name";
    /// Copy its first codepoint, `U+XXXX`.
    pub const COPY_CODEPOINT: &str = "emoji.copy-codepoint";
    /// Copy its category's label.
    pub const COPY_CATEGORY: &str = "emoji.copy-category";
    /// Edit the person's own keywords for it.
    pub const EDIT_KEYWORD: &str = "emoji.edit-keyword";
    /// Forget its visits.
    pub const RESET_RANKING: &str = "emoji.reset-ranking";
    /// Pin it.
    pub const PIN: &str = "emoji.pin";
    /// Unpin it.
    pub const UNPIN: &str = "emoji.unpin";
    /// Go back to the picker's tone.
    pub const TONE_RESET: &str = "emoji.tone-reset";
    /// Prefix of "use this tone", followed by the tone's id.
    pub const TONE: &str = "emoji.tone:";
}

/// The picker's state.
#[derive(Debug, Clone, Default)]
pub struct EmojiPage {
    /// The filter text.
    pub query: String,
    /// Positions in `compass_core::glyph::glyphs()`, in display order.
    pub shown: Vec<usize>,
    /// Position in `shown`.
    pub selected: usize,
    /// How many of `shown`'s first rows are the pinned section (empty query
    /// only).
    pub pinned: usize,
    /// How many rows after those are the recently used section.
    pub recent: usize,
    /// What the person has done with each glyph.
    pub service: GlyphService,
    /// Where `service` is kept; `None` keeps it in memory only, as tests do.
    pub path: Option<PathBuf>,
    /// The picker's `skinTone` preference, for glyphs with no tone of their
    /// own.
    pub picker_tone: Option<SkinTone>,
    /// Whether a paste can be asked for (an engine is attached).
    pub supports_paste: bool,
    /// The `defaultAction` preference: `paste` or `copy`, what Enter does.
    pub default_action: String,
    /// Why the last change could not be kept, until the next one.
    pub notice: Option<String>,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

impl EmojiPage {
    /// A picker over nothing remembered, showing the table from the top.
    #[must_use]
    pub fn new() -> Self {
        Self::open(None, None)
    }

    /// A picker over what `path` remembers, with the picker's tone
    /// preference (a `skinTone` id).
    #[must_use]
    pub fn open(path: Option<PathBuf>, picker_tone: Option<&str>) -> Self {
        let service = path
            .as_deref()
            .map(GlyphService::load_file)
            .unwrap_or_default();
        let mut page = Self {
            service,
            path,
            picker_tone: picker_tone.and_then(emoji_grid::skin_tone_by_id),
            ..Self::default()
        };
        page.refilter();
        page
    }

    /// Recomputes `shown` for the current query, back at the top.
    ///
    /// The empty query is `DisplayMode::Root`: the pinned glyphs, most
    /// recently pinned first; then the others the person has picked, most
    /// often first; then the table in its own order. A glyph in the first two
    /// appears in the table as well, as it does in the C++ grid. A query is
    /// `DisplayMode::Search`: every glyph that matches, best first, where a
    /// person's own keyword counts twice the name and each visit lifts it.
    pub fn refilter(&mut self) {
        self.selected = 0;
        let glyphs = glyph::glyphs();
        let query = self.query.trim();
        if query.is_empty() {
            let position = |character: &str| glyphs.iter().position(|g| g.character == character);
            let visited = self
                .service
                .visited(|character| glyph::lookup(character).is_some());
            let pinned: Vec<usize> = visited
                .iter()
                .filter(|entry| entry.pinned_at.is_some())
                .filter_map(|entry| position(&entry.emoji))
                .collect();
            let recent: Vec<usize> = visited
                .iter()
                .filter(|entry| entry.pinned_at.is_none())
                .filter_map(|entry| position(&entry.emoji))
                .collect();
            self.pinned = pinned.len();
            self.recent = recent.len();
            self.shown = pinned
                .into_iter()
                .chain(recent)
                .chain(0..glyphs.len().min(MAX_ROWS))
                .collect();
            return;
        }
        self.pinned = 0;
        self.recent = 0;
        let query = Query::new(query);
        let now = i64::try_from(now_secs()).unwrap_or(i64::MAX);
        let mut scored: Vec<(usize, u32)> = glyphs
            .iter()
            .enumerate()
            .filter_map(|(index, glyph)| {
                glyph_service::score(glyph, self.service.find(glyph.character), &query, now)
                    .map(|score| (index, score))
            })
            .collect();
        // Stable on ties, so equal matches keep the table's order.
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        self.shown = scored
            .into_iter()
            .take(MAX_ROWS)
            .map(|(index, _)| index)
            .collect();
    }

    /// The selected entry.
    #[must_use]
    pub fn selected_glyph(&self) -> Option<&'static Glyph> {
        glyph::glyphs().get(*self.shown.get(self.selected)?)
    }

    /// What the view says in place of the list, when nothing matches.
    #[must_use]
    pub fn empty_notice(&self) -> Option<&'static str> {
        self.shown
            .is_empty()
            .then_some("No matching emojis or symbols")
    }

    /// The heading drawn above the row at `position`, if one starts there:
    /// the pinned and recent sections', then each category's as the table
    /// reaches it. A search is one unheaded list.
    #[must_use]
    pub fn heading_at(&self, position: usize) -> Option<&'static str> {
        if !self.query.trim().is_empty() {
            return None;
        }
        if position == 0 && self.pinned > 0 {
            return Some(PINNED_HEADING);
        }
        if position == self.pinned && self.recent > 0 {
            return Some(RECENT_HEADING);
        }
        let table_start = self.pinned + self.recent;
        if position < table_start {
            return None;
        }
        let glyphs = glyph::glyphs();
        let this = glyphs.get(*self.shown.get(position)?)?;
        let starts_category = position == table_start
            || self
                .shown
                .get(position - 1)
                .and_then(|&index| glyphs.get(index))
                .is_none_or(|previous| previous.category != this.category);
        starts_category.then(|| this.category.label())
    }

    fn entry(&self, character: &str) -> Option<&SerializedEmojiMetadata> {
        self.service.find(character)
    }

    /// The tone remembered for `glyph`, if any.
    #[must_use]
    pub fn glyph_tone(&self, glyph: &Glyph) -> Option<SkinTone> {
        self.entry(glyph.character)
            .and_then(|entry| entry.skin_tone.as_deref())
            .and_then(emoji_grid::skin_tone_by_id)
    }

    /// The glyph as it is drawn and copied: in its own tone, else the
    /// picker's, for a glyph that takes one.
    #[must_use]
    pub fn display(&self, glyph: &Glyph) -> String {
        emoji_grid::copied_glyph(
            glyph.character,
            glyph.skinnable,
            emoji_grid::effective_tone(self.glyph_tone(glyph), self.picker_tone),
        )
    }

    /// Whether `glyph` is pinned.
    #[must_use]
    pub fn is_pinned(&self, glyph: &Glyph) -> bool {
        self.entry(glyph.character)
            .is_some_and(|entry| entry.pinned_at.is_some())
    }

    /// The person's own keywords for `glyph`, empty for none.
    #[must_use]
    pub fn keywords(&self, glyph: &Glyph) -> String {
        self.entry(glyph.character)
            .and_then(|entry| entry.keyword.clone())
            .unwrap_or_default()
    }

    /// Keeps the service, saying why when it could not be.
    fn save(&mut self) {
        let Some(path) = &self.path else {
            return;
        };
        self.notice = self
            .service
            .save_file(path)
            .err()
            .map(|err| format!("Could not save emoji metadata: {err}"));
    }

    /// Counts a pick of `glyph` (`VisitEmojiActionWrapper`).
    pub fn register_visit(&mut self, glyph: &Glyph) {
        self.service.register_visit(glyph.character, now_secs());
        self.save();
    }

    /// Pins or unpins `glyph`, then starts again from the top, as the C++
    /// does on `pinned` and `unpinned`.
    pub fn toggle_pin(&mut self, glyph: &Glyph) {
        if self.is_pinned(glyph) {
            self.service.unpin(glyph.character);
        } else {
            self.service.pin(glyph.character, now_secs());
        }
        self.save();
        self.refilter();
    }

    /// Forgets `glyph`'s visits, keeping the selection where it is.
    pub fn reset_ranking(&mut self, glyph: &Glyph) {
        self.service.reset_ranking(glyph.character);
        self.save();
        self.refresh_keeping_selection();
    }

    /// Remembers `tone` for `glyph`, or forgets its tone for `None`.
    pub fn set_tone(&mut self, glyph: &Glyph, tone: Option<SkinTone>) {
        match tone {
            Some(tone) => {
                self.service
                    .set_skin_tone(glyph.character, emoji_grid::skin_tone_info(tone).id);
            }
            None => {
                self.service.reset_skin_tone(glyph.character);
            }
        }
        self.save();
    }

    /// Sets the person's keywords for the glyph `character`.
    pub fn set_keywords(&mut self, character: &str, keywords: &str) {
        self.service.set_keywords(character, keywords.trim());
        self.save();
        self.refresh_keeping_selection();
    }

    fn refresh_keeping_selection(&mut self) {
        let selected = self.selected;
        self.refilter();
        self.selected = selected.min(self.shown.len().saturating_sub(1));
    }

    /// The selected glyph's panel: `buildEmojiActionPanel`, paste first when
    /// it is the default action.
    #[must_use]
    pub fn panel_sections(&self) -> Vec<crate::action_panel::PanelSection> {
        use crate::action_panel::{Action, PanelSection};
        let Some(glyph) = self.selected_glyph() else {
            return Vec::new();
        };
        let main = emoji_grid::main_actions(
            self.supports_paste,
            &self.default_action,
            self.is_pinned(glyph),
        )
        .into_iter()
        .enumerate()
        .filter_map(|(position, id)| {
            let (title, action) = match id {
                "paste" => ("Paste to active window", actions::PASTE),
                "copy" => ("Copy", actions::COPY),
                "copy-name" => ("Copy name", actions::COPY_NAME),
                "copy-codepoint" => ("Copy unicode codepoint", actions::COPY_CODEPOINT),
                "copy-category" => ("Copy category", actions::COPY_CATEGORY),
                "edit-keyword" => ("Edit keyword", actions::EDIT_KEYWORD),
                "reset-ranking" => ("Reset ranking", actions::RESET_RANKING),
                "pin" => ("Pin emoji", actions::PIN),
                "unpin" => ("Unpin emoji", actions::UNPIN),
                _ => return None,
            };
            let action = Action::new(title).with_id(action);
            Some(match id {
                _ if position == 0 => action.with_shortcut("enter"),
                "edit-keyword" => action.with_shortcut("ctrl+e"),
                _ => action,
            })
        })
        .collect();
        let mut sections = vec![PanelSection {
            name: String::new(),
            actions: main,
        }];
        let tones = emoji_grid::tone_options(
            glyph.character,
            glyph.skinnable,
            self.glyph_tone(glyph),
            self.picker_tone,
        );
        if !tones.is_empty() {
            sections.push(PanelSection {
                name: emoji_grid::TONE_SECTION_TITLE.to_owned(),
                actions: tones
                    .into_iter()
                    .map(|option| {
                        let id = option.tone.map_or_else(
                            || actions::TONE_RESET.to_owned(),
                            |tone| {
                                format!("{}{}", actions::TONE, emoji_grid::skin_tone_info(tone).id)
                            },
                        );
                        Action::new(format!("{} {}", option.preview, option.label)).with_id(id)
                    })
                    .collect(),
            });
        }
        sections
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(page: &EmojiPage, character: &str) -> usize {
        page.shown
            .iter()
            .position(|&i| glyph::glyphs()[i].character == character)
            .expect("shown")
    }

    #[test]
    fn a_name_or_a_keyword_finds_the_emoji_and_the_list_is_bounded() {
        let mut page = EmojiPage::new();
        assert_eq!(page.shown.len(), MAX_ROWS.min(glyph::glyphs().len()));

        page.query = "thumbs up".into();
        page.refilter();
        assert_eq!(page.selected_glyph().map(|g| g.character), Some("👍"));

        page.query = "pizza".into();
        page.refilter();
        assert!(
            page.shown
                .iter()
                .any(|&i| glyph::glyphs()[i].character == "🍕")
        );

        page.query = "zzzqqqxx".into();
        page.refilter();
        assert!(page.shown.is_empty());
    }

    #[test]
    fn pins_and_visits_are_kept_in_the_file_and_head_the_empty_query() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("emojis").join("emojis.json");
        let mut page = EmojiPage::open(Some(path.clone()), None);
        let pizza = glyph::lookup("🍕").expect("pizza");
        let rocket = glyph::lookup("🚀").expect("rocket");
        page.register_visit(rocket);
        page.toggle_pin(pizza);

        let reopened = EmojiPage::open(Some(path), None);
        assert_eq!((reopened.pinned, reopened.recent), (1, 1));
        assert_eq!(reopened.selected_glyph().map(|g| g.character), Some("🍕"));
        assert_eq!(reopened.heading_at(0), Some(PINNED_HEADING));
        assert_eq!(reopened.heading_at(1), Some(RECENT_HEADING));
        assert_eq!(
            glyph::glyphs()[reopened.shown[1]].character,
            "🚀",
            "the visited glyph is the recent section"
        );
        let first_category = glyph::glyphs()[0].category.label();
        assert_eq!(reopened.heading_at(2), Some(first_category));
        assert_eq!(reopened.heading_at(3), None);

        let mut page = reopened;
        page.toggle_pin(pizza);
        assert_eq!(page.pinned, 0, "unpinning takes it out of the section");
    }

    #[test]
    fn a_keyword_of_ones_own_finds_the_glyph_and_survives_a_reopen() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("emojis.json");
        let mut page = EmojiPage::open(Some(path.clone()), None);
        page.query = "qqlunch".into();
        page.refilter();
        assert!(page.shown.is_empty());

        page.set_keywords("🍕", "qqlunch");
        let mut page = EmojiPage::open(Some(path), None);
        page.query = "qqlunch".into();
        page.refilter();
        assert_eq!(page.selected_glyph().map(|g| g.character), Some("🍕"));
        assert_eq!(
            page.keywords(glyph::lookup("🍕").expect("pizza")),
            "qqlunch"
        );
    }

    #[test]
    fn a_tone_is_remembered_per_glyph_and_wins_over_the_pickers() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("emojis.json");
        let wave = glyph::lookup("👋").expect("wave");
        assert!(wave.skinnable);

        let mut page = EmojiPage::open(Some(path.clone()), Some("light"));
        assert_eq!(page.display(wave), "👋\u{1F3FB}");
        page.set_tone(wave, Some(SkinTone::Dark));

        let mut page = EmojiPage::open(Some(path.clone()), Some("light"));
        assert_eq!(page.display(wave), "👋\u{1F3FF}");
        page.set_tone(wave, None);
        let page = EmojiPage::open(Some(path), Some("light"));
        assert_eq!(page.display(wave), "👋\u{1F3FB}");
    }

    #[test]
    fn the_panel_offers_pin_or_unpin_and_the_tones_for_a_skinnable_glyph() {
        let mut page = EmojiPage::new();
        page.query = "waving hand".into();
        page.refilter();
        let position = at(&page, "👋");
        page.selected = position;
        let ids = |page: &EmojiPage| -> Vec<Vec<String>> {
            page.panel_sections()
                .iter()
                .map(|section| {
                    section
                        .actions
                        .iter()
                        .filter_map(|action| action.id.clone())
                        .collect()
                })
                .collect()
        };
        let sections = ids(&page);
        assert_eq!(
            sections[0],
            [
                actions::COPY,
                actions::COPY_NAME,
                actions::COPY_CODEPOINT,
                actions::COPY_CATEGORY,
                actions::EDIT_KEYWORD,
                actions::RESET_RANKING,
                actions::PIN,
            ]
        );
        // The six rows of the table less the default, which is also the
        // current tone: five, and no reset.
        assert_eq!(sections[1].len(), 5);
        assert!(!sections[1].contains(&actions::TONE_RESET.to_owned()));

        let wave = glyph::lookup("👋").expect("wave");
        page.service.pin(wave.character, 1);
        page.service.set_skin_tone(wave.character, "dark");
        let sections = ids(&page);
        assert_eq!(sections[0].last().map(String::as_str), Some(actions::UNPIN));
        assert_eq!(sections[1][0], actions::TONE_RESET);
        assert!(!sections[1].contains(&format!("{}dark", actions::TONE)));

        page.query = "pizza".into();
        page.refilter();
        assert_eq!(
            ids(&page).len(),
            1,
            "a glyph with no tone has no tone section"
        );
    }
}

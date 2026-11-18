//! Which apps appear in the dock and in what order:
//! 1. pinned apps;
//! 2. unpinned open apps, in opening order;
//! 3. recently closed unpinned apps, as in macOS.

pub const MAX_RECENT: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DockApp {
    pub id: String,
    pub running: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Roster {
    pinned: Vec<String>,
    extra: Vec<String>,
    running: Vec<String>,
    recent: Vec<String>,
}

impl Roster {
    #[must_use]
    pub fn new(pinned: Vec<String>, recent: Vec<String>) -> Self {
        let mut roster = Self {
            pinned,
            ..Self::default()
        };
        for id in recent.into_iter().rev() {
            roster.remember(id);
        }
        roster
    }

    /// Updates open apps and returns `true` if the dock's displayed items change.
    pub fn sync<'a>(&mut self, running: impl IntoIterator<Item = &'a str>) -> bool {
        let before = (self.apps(), self.recent.clone());

        let mut now: Vec<String> = Vec::new();
        for id in running {
            if !now.iter().any(|seen| seen == id) {
                now.push(id.to_owned());
            }
        }
        let closed: Vec<String> = self
            .extra
            .iter()
            .filter(|id| !now.contains(id))
            .cloned()
            .collect();
        self.extra.retain(|id| now.contains(id));
        for id in &now {
            if !self.pinned.contains(id) && !self.extra.contains(id) {
                self.extra.push(id.clone());
            }
        }
        // Reopened apps are no longer recent; closed apps become recent.
        self.recent.retain(|id| !now.contains(id));
        for id in closed {
            self.remember(id);
        }
        self.running = now;

        (self.apps(), self.recent.clone()) != before
    }

    #[must_use]
    pub fn is_running(&self, id: &str) -> bool {
        self.running.iter().any(|running| running == id)
    }

    #[must_use]
    pub fn is_pinned(&self, id: &str) -> bool {
        self.pinned.iter().any(|pinned| pinned == id)
    }

    /// Pins an app at the end of the pinned list. If it was open and unpinned,
    /// it keeps its screen position as it moves from the open list to the pinned list.
    pub fn pin(&mut self, id: &str) {
        if self.is_pinned(id) {
            return;
        }
        self.extra.retain(|extra| extra != id);
        self.recent.retain(|recent| recent != id);
        self.pinned.push(id.to_owned());
    }

    /// Pins `id` (or moves it, if already pinned) right before `before`, or at
    /// the end of the pinned list when `before` is `None` or not pinned.
    pub fn pin_before(&mut self, id: &str, before: Option<&str>) {
        if before == Some(id) {
            return;
        }
        self.pinned.retain(|pinned| pinned != id);
        self.extra.retain(|extra| extra != id);
        self.recent.retain(|recent| recent != id);
        let at = before
            .and_then(|before| self.pinned.iter().position(|pinned| pinned == before))
            .unwrap_or(self.pinned.len());
        self.pinned.insert(at, id.to_owned());
    }

    /// Unpins an app. If it is open, it remains in the dock as the first unpinned
    /// app until it closes.
    pub fn unpin(&mut self, id: &str) {
        if !self.is_pinned(id) {
            return;
        }
        self.pinned.retain(|pinned| pinned != id);
        if self.is_running(id) {
            self.extra.insert(0, id.to_owned());
        }
    }

    #[must_use]
    pub fn pinned(&self) -> &[String] {
        &self.pinned
    }

    /// Recent apps in display order, newest first.
    #[must_use]
    pub fn recent(&self) -> &[String] {
        &self.recent
    }

    /// Pinned and open apps in display order, with pinned apps first.
    #[must_use]
    pub fn apps(&self) -> Vec<DockApp> {
        self.pinned
            .iter()
            .chain(&self.extra)
            .map(|id| DockApp {
                id: id.clone(),
                running: self.is_running(id),
            })
            .collect()
    }

    fn remember(&mut self, id: String) {
        if self.is_pinned(&id) {
            return;
        }
        self.recent.retain(|recent| *recent != id);
        self.recent.insert(0, id);
        self.recent.truncate(MAX_RECENT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roster() -> Roster {
        Roster::new(vec!["explorer".into(), "browser".into()], Vec::new())
    }

    fn ids(roster: &Roster) -> Vec<String> {
        roster.apps().into_iter().map(|app| app.id).collect()
    }

    #[test]
    fn roster_should_show_pinned_apps_when_nothing_is_running() {
        assert_eq!(ids(&roster()), ["explorer", "browser"]);
    }

    #[test]
    fn roster_should_append_unpinned_running_apps_after_pinned() {
        let mut roster = roster();
        roster.sync(["editor", "browser"]);
        assert_eq!(ids(&roster), ["explorer", "browser", "editor"]);
    }

    #[test]
    fn roster_should_mark_only_running_apps() {
        let mut roster = roster();
        roster.sync(["browser"]);
        let running: Vec<bool> = roster.apps().iter().map(|app| app.running).collect();
        assert_eq!(running, [false, true]);
    }

    #[test]
    fn roster_should_keep_first_seen_order_for_unpinned_apps() {
        let mut roster = roster();
        roster.sync(["editor"]);
        roster.sync(["terminal", "editor"]);
        assert_eq!(ids(&roster), ["explorer", "browser", "editor", "terminal"]);
    }

    #[test]
    fn roster_should_drop_unpinned_app_when_it_closes() {
        let mut roster = roster();
        roster.sync(["editor"]);
        roster.sync([]);
        assert_eq!(ids(&roster), ["explorer", "browser"]);
    }

    #[test]
    fn sync_should_report_change_when_an_indicator_changes() {
        assert!(roster().sync(["browser"]));
    }

    #[test]
    fn sync_should_report_no_change_when_only_window_count_changes() {
        let mut roster = roster();
        roster.sync(["browser"]);
        assert!(!roster.sync(["browser", "browser"]));
    }

    #[test]
    fn closing_an_unpinned_app_should_make_it_recent() {
        let mut roster = roster();
        roster.sync(["editor"]);
        roster.sync([]);
        assert_eq!(roster.recent(), ["editor"]);
    }

    #[test]
    fn closing_a_pinned_app_should_not_make_it_recent() {
        let mut roster = roster();
        roster.sync(["browser"]);
        roster.sync([]);
        assert!(roster.recent().is_empty());
    }

    #[test]
    fn recent_should_keep_only_the_last_three_most_recent_first() {
        let mut roster = roster();
        for app in ["a", "b", "c", "d"] {
            roster.sync([app]);
            roster.sync([]);
        }
        assert_eq!(roster.recent(), ["d", "c", "b"]);
    }

    #[test]
    fn reopening_a_recent_app_should_remove_it_from_recent() {
        let mut roster = roster();
        roster.sync(["editor"]);
        roster.sync([]);
        roster.sync(["editor"]);
        assert!(roster.recent().is_empty());
    }

    #[test]
    fn sync_should_report_change_when_recent_changes() {
        let mut roster = roster();
        roster.sync(["editor"]);
        assert!(roster.sync([]));
    }

    #[test]
    fn new_should_drop_recent_apps_that_are_pinned() {
        let roster = Roster::new(vec!["browser".into()], vec!["browser".into(), "x".into()]);
        assert_eq!(roster.recent(), ["x"]);
    }

    #[test]
    fn pin_should_keep_a_running_app_in_the_same_place_on_screen() {
        let mut roster = roster();
        roster.sync(["editor"]);
        roster.pin("editor");
        assert_eq!(ids(&roster), ["explorer", "browser", "editor"]);
    }

    #[test]
    fn pin_should_remove_the_app_from_recent() {
        let mut roster = roster();
        roster.sync(["editor"]);
        roster.sync([]);
        roster.pin("editor");
        assert!(roster.recent().is_empty());
    }

    #[test]
    fn unpin_should_keep_a_running_app_as_first_unpinned() {
        let mut roster = roster();
        roster.sync(["browser", "editor"]);
        roster.unpin("browser");
        assert_eq!(ids(&roster), ["explorer", "browser", "editor"]);
    }

    #[test]
    fn unpin_should_remove_a_closed_app_from_the_dock() {
        let mut roster = roster();
        roster.unpin("browser");
        assert_eq!(ids(&roster), ["explorer"]);
    }

    #[test]
    fn pin_before_should_move_a_pinned_app_left() {
        let mut roster = Roster::new(vec!["a".into(), "b".into(), "c".into()], Vec::new());
        roster.pin_before("c", Some("b"));
        assert_eq!(roster.pinned(), ["a", "c", "b"]);
    }

    #[test]
    fn pin_before_should_move_a_pinned_app_to_the_end() {
        let mut roster = Roster::new(vec!["a".into(), "b".into(), "c".into()], Vec::new());
        roster.pin_before("a", None);
        assert_eq!(roster.pinned(), ["b", "c", "a"]);
    }

    #[test]
    fn pin_before_should_pin_a_running_app_at_the_given_place() {
        let mut roster = roster();
        roster.sync(["editor"]);
        roster.pin_before("editor", Some("browser"));
        assert_eq!(ids(&roster), ["explorer", "editor", "browser"]);
    }

    #[test]
    fn pin_before_itself_should_change_nothing() {
        let mut roster = roster();
        roster.pin_before("browser", Some("browser"));
        assert_eq!(roster.pinned(), ["explorer", "browser"]);
    }

    #[test]
    fn unpinned_running_app_should_become_recent_when_it_closes() {
        let mut roster = roster();
        roster.sync(["browser"]);
        roster.unpin("browser");
        roster.sync([]);
        assert_eq!(roster.recent(), ["browser"]);
    }
}

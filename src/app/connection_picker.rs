//! Connection picker — choose the Local or SSH server this client attaches to.
//!
//! Client-side only, like the profiles it edits. Switching destinations cannot
//! be done in place: the client is bound to one server for its lifetime, and
//! adding a live "reconnect elsewhere" message would deepen exactly the
//! server/TUI coupling the project is moving away from. So the picker records
//! the choice and detaches; the next `herdr` lands on the new destination, with
//! every pane on the machine you left still running.

use crate::remote::profiles::{validate_target, RemoteProfiles};

use super::state::{AppState, Mode};

/// One row of the picker. `Local` is always first, then saved SSH profiles,
/// then the "add" row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConnectionRow {
    Local {
        active: bool,
    },
    Remote {
        name: String,
        target: String,
        active: bool,
    },
    Add,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ConnectionPickerState {
    pub profiles: RemoteProfiles,
    pub selected: usize,
    /// `Some` while the add form is open; holds what has been typed so far.
    pub adding: Option<String>,
    /// Shown under the list: a rejected target, or the pending-detach notice.
    pub message: Option<String>,
}

impl ConnectionPickerState {
    pub(crate) fn rows(&self) -> Vec<ConnectionRow> {
        let active = self.profiles.active.as_deref();
        let mut rows = vec![ConnectionRow::Local {
            active: active.is_none(),
        }];
        rows.extend(
            self.profiles
                .profiles
                .iter()
                .map(|profile| ConnectionRow::Remote {
                    name: profile.name.clone(),
                    target: profile.target.clone(),
                    active: active == Some(profile.name.as_str()),
                }),
        );
        rows.push(ConnectionRow::Add);
        rows
    }
}

impl AppState {
    pub(crate) fn open_connection_picker(&mut self) {
        let profiles = RemoteProfiles::load();
        let selected = profiles
            .active
            .as_deref()
            .and_then(|active| {
                profiles
                    .profiles
                    .iter()
                    .position(|profile| profile.name == active)
                    .map(|idx| idx + 1) // row 0 is Local
            })
            .unwrap_or(0);
        self.connection_picker = ConnectionPickerState {
            profiles,
            selected,
            adding: None,
            message: None,
        };
        self.mode = Mode::ConnectionPicker;
    }

    pub(crate) fn close_connection_picker(&mut self) {
        if self.mode == Mode::ConnectionPicker {
            self.mode = Mode::Terminal;
        }
    }

    pub(crate) fn move_connection_selection(&mut self, delta: isize) {
        let count = self.connection_picker.rows().len();
        if count == 0 {
            return;
        }
        let current = self.connection_picker.selected.min(count - 1) as isize;
        self.connection_picker.selected = (current + delta).rem_euclid(count as isize) as usize;
    }

    /// Activates the selected row, or opens the add form.
    ///
    /// Returns true when the choice needs a detach to take effect, which is
    /// every case except re-picking the destination already in use.
    pub(crate) fn activate_connection_selection(&mut self) -> bool {
        let rows = self.connection_picker.rows();
        let Some(row) = rows.get(self.connection_picker.selected).cloned() else {
            return false;
        };
        match row {
            ConnectionRow::Add => {
                self.connection_picker.adding = Some(String::new());
                self.connection_picker.message = None;
                false
            }
            ConnectionRow::Local { active } => {
                if active {
                    self.close_connection_picker();
                    return false;
                }
                let _ = self.connection_picker.profiles.set_active(None);
                self.persist_connection_profiles();
                true
            }
            ConnectionRow::Remote { name, active, .. } => {
                if active {
                    self.close_connection_picker();
                    return false;
                }
                let _ = self.connection_picker.profiles.set_active(Some(&name));
                self.persist_connection_profiles();
                true
            }
        }
    }

    /// Saves the typed target as a new profile and makes it active.
    /// Returns true when it needs a detach to take effect.
    pub(crate) fn submit_connection_add(&mut self) -> bool {
        let Some(input) = self.connection_picker.adding.clone() else {
            return false;
        };
        let target = input.trim().to_string();
        if let Err(err) = validate_target(&target) {
            self.connection_picker.message = Some(err);
            return false;
        }
        // The target doubles as the name: an alias or user@host is already the
        // clearest label, and renaming stays available from the CLI.
        if let Err(err) = self.connection_picker.profiles.upsert(&target, &target) {
            self.connection_picker.message = Some(err);
            return false;
        }
        self.connection_picker.adding = None;
        self.persist_connection_profiles();
        let rows = self.connection_picker.rows();
        self.connection_picker.selected = rows
            .iter()
            .position(|row| matches!(row, ConnectionRow::Remote { name, .. } if name == &target))
            .unwrap_or(0);
        true
    }

    /// Forgets the selected profile. Local and the add row cannot be removed.
    pub(crate) fn remove_selected_connection(&mut self) {
        let rows = self.connection_picker.rows();
        let Some(ConnectionRow::Remote { name, .. }) = rows.get(self.connection_picker.selected)
        else {
            return;
        };
        let name = name.clone();
        if self.connection_picker.profiles.remove(&name).is_ok() {
            self.persist_connection_profiles();
            let count = self.connection_picker.rows().len();
            self.connection_picker.selected = self.connection_picker.selected.min(count - 1);
        }
    }

    fn persist_connection_profiles(&mut self) {
        if let Err(err) = self
            .connection_picker
            .profiles
            .save_to(&crate::remote::profiles::profiles_path())
        {
            self.connection_picker.message = Some(format!("could not save: {err}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picker_state() -> AppState {
        let mut state = AppState::test_new();
        state.mode = Mode::ConnectionPicker;
        state
    }

    fn with_profiles(pairs: &[(&str, &str)], active: Option<&str>) -> AppState {
        let mut state = picker_state();
        let mut profiles = RemoteProfiles::default();
        for (name, target) in pairs {
            profiles.upsert(name, target).unwrap();
        }
        profiles.set_active(active).unwrap();
        state.connection_picker = ConnectionPickerState {
            profiles,
            selected: 0,
            adding: None,
            message: None,
        };
        state
    }

    #[test]
    fn local_is_always_first_and_add_is_always_last() {
        let state = with_profiles(&[("work", "workbox")], None);

        let rows = state.connection_picker.rows();

        assert!(matches!(
            rows.first(),
            Some(ConnectionRow::Local { active: true })
        ));
        assert!(matches!(rows.last(), Some(ConnectionRow::Add)));
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn selection_wraps_across_the_whole_list() {
        let mut state = with_profiles(&[("work", "workbox")], None);

        state.move_connection_selection(-1);
        assert_eq!(state.connection_picker.selected, 2);

        state.move_connection_selection(1);
        assert_eq!(state.connection_picker.selected, 0);
    }

    #[test]
    fn picking_the_active_destination_just_closes() {
        let mut state = with_profiles(&[("work", "workbox")], None);
        state.connection_picker.selected = 0; // Local, already active

        let needs_detach = state.activate_connection_selection();

        assert!(!needs_detach);
        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn picking_another_destination_asks_for_a_detach() {
        let mut state = with_profiles(&[("work", "workbox")], None);
        state.connection_picker.selected = 1; // the SSH profile

        let needs_detach = state.activate_connection_selection();

        assert!(needs_detach);
        assert_eq!(
            state.connection_picker.profiles.active_target(),
            Some("workbox")
        );
    }

    #[test]
    fn the_add_row_opens_the_form_without_detaching() {
        let mut state = with_profiles(&[], None);
        state.connection_picker.selected = 1; // Add

        let needs_detach = state.activate_connection_selection();

        assert!(!needs_detach);
        assert_eq!(state.connection_picker.adding.as_deref(), Some(""));
    }

    #[test]
    fn a_valid_target_becomes_an_active_profile() {
        let mut state = with_profiles(&[], None);
        state.connection_picker.adding = Some("  workbox  ".to_string());

        let needs_detach = state.submit_connection_add();

        assert!(needs_detach);
        assert_eq!(
            state.connection_picker.profiles.active_target(),
            Some("workbox")
        );
        assert!(state.connection_picker.adding.is_none());
    }

    #[test]
    fn a_hostile_target_is_rejected_and_explained() {
        let mut state = with_profiles(&[], None);
        state.connection_picker.adding = Some("-oProxyCommand=x".to_string());

        let needs_detach = state.submit_connection_add();

        assert!(!needs_detach);
        assert!(state.connection_picker.profiles.profiles.is_empty());
        assert!(state.connection_picker.message.is_some());
        assert!(
            state.connection_picker.adding.is_some(),
            "form stays open to fix it"
        );
    }

    #[test]
    fn removing_a_profile_keeps_the_selection_in_range() {
        let mut state = with_profiles(&[("work", "workbox")], Some("work"));
        state.connection_picker.selected = 1;

        state.remove_selected_connection();

        assert!(state.connection_picker.profiles.profiles.is_empty());
        assert!(state.connection_picker.selected < state.connection_picker.rows().len());
    }

    #[test]
    fn local_and_add_rows_cannot_be_removed() {
        let mut state = with_profiles(&[("work", "workbox")], None);

        state.connection_picker.selected = 0;
        state.remove_selected_connection();
        state.connection_picker.selected = 2;
        state.remove_selected_connection();

        assert_eq!(state.connection_picker.profiles.profiles.len(), 1);
    }
}

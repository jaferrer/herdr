//! Keyboard for the connection picker.
//!
//! Two shapes in one modal: a list of destinations, and a one-field form to add
//! one. Choosing a different destination detaches, because a client is bound to
//! its server for its lifetime; the panes on the machine you leave keep running.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::state::AppState;

impl AppState {
    pub(crate) fn handle_connection_picker_key(&mut self, key: KeyEvent) {
        if self.connection_picker.adding.is_some() {
            self.handle_connection_add_key(key);
            return;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.close_connection_picker(),
            KeyCode::Up | KeyCode::Char('k') => self.move_connection_selection(-1),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => self.move_connection_selection(1),
            KeyCode::BackTab => self.move_connection_selection(-1),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.remove_selected_connection()
            }
            KeyCode::Enter if self.activate_connection_selection() => {
                self.request_connection_detach()
            }
            _ => {}
        }
    }

    fn handle_connection_add_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.connection_picker.adding = None;
                self.connection_picker.message = None;
            }
            KeyCode::Enter if self.submit_connection_add() => self.request_connection_detach(),
            KeyCode::Backspace => {
                if let Some(input) = self.connection_picker.adding.as_mut() {
                    input.pop();
                }
                self.connection_picker.message = None;
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(input) = self.connection_picker.adding.as_mut() {
                    input.push(ch);
                }
                self.connection_picker.message = None;
            }
            _ => {}
        }
    }

    /// Leaves the client so the next launch lands on the chosen destination.
    /// Reuses the normal detach path, so it never stops a server.
    fn request_connection_detach(&mut self) {
        self.close_connection_picker();
        if self.detach_exits {
            self.should_quit = true;
        } else {
            self.detach_requested = true;
        }
    }

    pub(crate) fn insert_connection_add_text(&mut self, text: &str) -> bool {
        let Some(input) = self.connection_picker.adding.as_mut() else {
            return false;
        };
        input.extend(text.chars().filter(|ch| !ch.is_control()));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::Mode;
    use crate::remote::profiles::RemoteProfiles;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    fn typed(text: &str, state: &mut AppState) {
        for ch in text.chars() {
            state.handle_connection_picker_key(key(KeyCode::Char(ch)));
        }
    }

    fn picker(profiles: &[(&str, &str)], active: Option<&str>) -> AppState {
        let mut state = AppState::test_new();
        let mut store = RemoteProfiles::default();
        for (name, target) in profiles {
            store.upsert(name, target).unwrap();
        }
        store.set_active(active).unwrap();
        state.connection_picker = crate::app::connection_picker::ConnectionPickerState {
            profiles: store,
            selected: 0,
            adding: None,
            message: None,
        };
        state.mode = Mode::ConnectionPicker;
        state.detach_exits = true; // makes the detach observable in tests
        state
    }

    #[test]
    fn escape_closes_the_picker() {
        let mut state = picker(&[], None);

        state.handle_connection_picker_key(key(KeyCode::Esc));

        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn arrows_and_tab_move_through_the_list() {
        let mut state = picker(&[("work", "workbox")], None);

        state.handle_connection_picker_key(key(KeyCode::Down));
        assert_eq!(state.connection_picker.selected, 1);

        state.handle_connection_picker_key(key(KeyCode::Tab));
        assert_eq!(state.connection_picker.selected, 2);

        state.handle_connection_picker_key(key(KeyCode::Up));
        assert_eq!(state.connection_picker.selected, 1);
    }

    #[test]
    fn choosing_another_destination_detaches() {
        let mut state = picker(&[("work", "workbox")], None);
        state.connection_picker.selected = 1;

        state.handle_connection_picker_key(key(KeyCode::Enter));

        assert!(state.should_quit, "detach requested");
        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn choosing_the_active_destination_closes_without_detaching() {
        let mut state = picker(&[("work", "workbox")], None);
        state.connection_picker.selected = 0; // local, already active

        state.handle_connection_picker_key(key(KeyCode::Enter));

        assert!(!state.should_quit);
        assert_eq!(state.mode, Mode::Terminal);
    }

    #[test]
    fn typing_a_target_and_pressing_enter_saves_and_detaches() {
        let mut state = picker(&[], None);
        state.connection_picker.selected = 1; // add row
        state.handle_connection_picker_key(key(KeyCode::Enter));

        typed("workbox", &mut state);
        state.handle_connection_picker_key(key(KeyCode::Enter));

        assert_eq!(
            state.connection_picker.profiles.active_target(),
            Some("workbox")
        );
        assert!(state.should_quit);
    }

    #[test]
    fn backspace_edits_the_typed_target() {
        let mut state = picker(&[], None);
        state.connection_picker.adding = Some(String::new());

        typed("workboxx", &mut state);
        state.handle_connection_picker_key(key(KeyCode::Backspace));

        assert_eq!(state.connection_picker.adding.as_deref(), Some("workbox"));
    }

    #[test]
    fn a_rejected_target_keeps_the_form_open_and_explains() {
        let mut state = picker(&[], None);
        state.connection_picker.adding = Some(String::new());

        typed("-oProxyCommand=x", &mut state);
        state.handle_connection_picker_key(key(KeyCode::Enter));

        assert!(!state.should_quit);
        assert!(state.connection_picker.adding.is_some());
        assert!(state.connection_picker.message.is_some());
    }

    #[test]
    fn escape_in_the_form_returns_to_the_list() {
        let mut state = picker(&[], None);
        state.connection_picker.adding = Some("half-typed".to_string());

        state.handle_connection_picker_key(key(KeyCode::Esc));

        assert!(state.connection_picker.adding.is_none());
        assert_eq!(state.mode, Mode::ConnectionPicker, "still in the picker");
    }

    #[test]
    fn ctrl_d_forgets_the_selected_destination() {
        let mut state = picker(&[("work", "workbox")], None);
        state.connection_picker.selected = 1;

        state
            .handle_connection_picker_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));

        assert!(state.connection_picker.profiles.profiles.is_empty());
    }

    #[test]
    fn pasted_text_lands_in_the_form_only() {
        let mut state = picker(&[], None);

        assert!(!state.insert_connection_add_text("workbox"));

        state.connection_picker.adding = Some(String::new());
        assert!(state.insert_connection_add_text("work\nbox"));
        assert_eq!(state.connection_picker.adding.as_deref(), Some("workbox"));
    }
}

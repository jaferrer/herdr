//! Client-local Local/SSH connection profiles.
//!
//! This is **client state, not server state**: it records which server this
//! client attaches to, so no server ever sees it. `None` as the active profile
//! means the local server, which is Herdr's normal launch.
//!
//! Passwords are never stored. Remote attach uses normal OpenSSH
//! authentication, so credentials stay with `ssh` and `ssh-agent`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One saved SSH destination. `name` is what the user sees, `target` is what
/// `herdr --remote` receives (`host`, `user@host`, or an `~/.ssh/config` alias).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemoteProfile {
    pub(crate) name: String,
    pub(crate) target: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RemoteProfiles {
    #[serde(default)]
    pub(crate) profiles: Vec<RemoteProfile>,
    /// Name of the active profile. `None` is the local server.
    #[serde(default)]
    pub(crate) active: Option<String>,
}

pub(crate) fn profiles_path() -> PathBuf {
    crate::config::config_dir().join("connections.json")
}

/// Rejects targets `ssh` would misread as options or as a whole command, so a
/// stored profile can never smuggle extra arguments into the attach.
pub(crate) fn validate_target(target: &str) -> Result<&str, String> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return Err("SSH target cannot be empty".to_string());
    }
    if trimmed.starts_with('-') {
        return Err("SSH target must not start with '-'".to_string());
    }
    if trimmed.split_whitespace().count() != 1 {
        return Err("SSH target must be a single host, alias, or user@host".to_string());
    }
    if trimmed.chars().any(char::is_control) {
        return Err("SSH target contains control characters".to_string());
    }
    Ok(trimmed)
}

fn validate_name(name: &str) -> Result<&str, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("profile name cannot be empty".to_string());
    }
    if trimmed.chars().any(char::is_control) {
        return Err("profile name contains control characters".to_string());
    }
    Ok(trimmed)
}

impl RemoteProfiles {
    pub(crate) fn load() -> Self {
        Self::load_from(&profiles_path())
    }

    /// A broken or unreadable file degrades to "local only" instead of failing
    /// the launch: the whole point of the store is picking a destination, and
    /// the local one always works.
    pub(crate) fn load_from(path: &Path) -> Self {
        let Ok(content) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let mut loaded: Self = match serde_json::from_str(&content) {
            Ok(loaded) => loaded,
            Err(err) => {
                tracing::warn!(%err, path = %path.display(), "ignoring unreadable connection profiles");
                return Self::default();
            }
        };
        loaded.sanitize();
        loaded
    }

    /// Drops entries that could not be attached to anyway, and clears an active
    /// name with no profile behind it, so callers never have to re-check.
    fn sanitize(&mut self) {
        self.profiles.retain(|profile| {
            validate_name(&profile.name).is_ok() && validate_target(&profile.target).is_ok()
        });
        self.profiles.dedup_by(|a, b| a.name == b.name);
        if let Some(active) = &self.active {
            if !self.profiles.iter().any(|profile| &profile.name == active) {
                self.active = None;
            }
        }
    }

    pub(crate) fn save_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        let tmp_path = path.with_extension("json.tmp");
        std::fs::write(&tmp_path, &json)?;
        if let Err(err) = std::fs::rename(&tmp_path, path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(err);
        }
        Ok(())
    }

    /// Adds or updates a profile by name and makes it active.
    pub(crate) fn upsert(&mut self, name: &str, target: &str) -> Result<(), String> {
        let name = validate_name(name)?.to_string();
        let target = validate_target(target)?.to_string();
        match self
            .profiles
            .iter_mut()
            .find(|profile| profile.name == name)
        {
            Some(existing) => existing.target = target,
            None => self.profiles.push(RemoteProfile {
                name: name.clone(),
                target,
            }),
        }
        self.active = Some(name);
        Ok(())
    }

    pub(crate) fn rename(&mut self, current: &str, new_name: &str) -> Result<(), String> {
        let new_name = validate_name(new_name)?.to_string();
        if self.profiles.iter().any(|profile| profile.name == new_name) {
            return Err(format!("a profile named '{new_name}' already exists"));
        }
        let Some(profile) = self
            .profiles
            .iter_mut()
            .find(|profile| profile.name == current)
        else {
            return Err(format!("unknown profile '{current}'"));
        };
        profile.name = new_name.clone();
        if self.active.as_deref() == Some(current) {
            self.active = Some(new_name);
        }
        Ok(())
    }

    /// Removing the active profile falls back to local rather than leaving the
    /// client pointed at something that no longer exists.
    pub(crate) fn remove(&mut self, name: &str) -> Result<(), String> {
        let before = self.profiles.len();
        self.profiles.retain(|profile| profile.name != name);
        if self.profiles.len() == before {
            return Err(format!("unknown profile '{name}'"));
        }
        if self.active.as_deref() == Some(name) {
            self.active = None;
        }
        Ok(())
    }

    /// `None` selects the local server.
    pub(crate) fn set_active(&mut self, name: Option<&str>) -> Result<(), String> {
        match name {
            None => {
                self.active = None;
                Ok(())
            }
            Some(name) if self.profiles.iter().any(|profile| profile.name == name) => {
                self.active = Some(name.to_string());
                Ok(())
            }
            Some(name) => Err(format!("unknown profile '{name}'")),
        }
    }

    /// The SSH target to attach to, or `None` for the local server.
    pub(crate) fn active_target(&self) -> Option<&str> {
        let active = self.active.as_deref()?;
        self.profiles
            .iter()
            .find(|profile| profile.name == active)
            .map(|profile| profile.target.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("herdr-profiles-test-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("connections.json")
    }

    #[test]
    fn no_profiles_means_the_local_server() {
        let profiles = RemoteProfiles::default();

        assert_eq!(profiles.active_target(), None);
    }

    #[test]
    fn upsert_adds_updates_and_activates() {
        let mut profiles = RemoteProfiles::default();

        profiles.upsert("work", "workbox").unwrap();
        assert_eq!(profiles.active_target(), Some("workbox"));

        profiles.upsert("work", "user@workbox").unwrap();
        assert_eq!(profiles.profiles.len(), 1);
        assert_eq!(profiles.active_target(), Some("user@workbox"));
    }

    #[test]
    fn a_hostile_target_never_becomes_a_profile() {
        let mut profiles = RemoteProfiles::default();

        assert!(profiles
            .upsert("bad", "-oProxyCommand=touch /tmp/pwn")
            .is_err());
        assert!(profiles.upsert("bad", "host; rm -rf /").is_err());
        assert!(profiles.upsert("bad", "  ").is_err());
        assert!(profiles.profiles.is_empty());
    }

    #[test]
    fn removing_the_active_profile_falls_back_to_local() {
        let mut profiles = RemoteProfiles::default();
        profiles.upsert("work", "workbox").unwrap();

        profiles.remove("work").unwrap();

        assert_eq!(profiles.active_target(), None);
        assert!(profiles.remove("work").is_err());
    }

    #[test]
    fn renaming_follows_the_active_selection() {
        let mut profiles = RemoteProfiles::default();
        profiles.upsert("work", "workbox").unwrap();
        profiles.upsert("other", "otherbox").unwrap();
        profiles.set_active(Some("work")).unwrap();

        profiles.rename("work", "office").unwrap();

        assert_eq!(profiles.active.as_deref(), Some("office"));
        assert_eq!(profiles.active_target(), Some("workbox"));
        assert!(profiles.rename("office", "other").is_err());
    }

    #[test]
    fn set_active_rejects_unknown_and_accepts_local() {
        let mut profiles = RemoteProfiles::default();
        profiles.upsert("work", "workbox").unwrap();

        assert!(profiles.set_active(Some("nope")).is_err());
        profiles.set_active(None).unwrap();
        assert_eq!(profiles.active_target(), None);
    }

    #[test]
    fn profiles_round_trip_through_disk() {
        let path = temp_path("roundtrip");
        let mut profiles = RemoteProfiles::default();
        profiles.upsert("work", "workbox").unwrap();

        profiles.save_to(&path).unwrap();
        let loaded = RemoteProfiles::load_from(&path);

        assert_eq!(loaded, profiles);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_corrupt_or_missing_file_degrades_to_local() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "{ not json").unwrap();

        assert_eq!(RemoteProfiles::load_from(&path), RemoteProfiles::default());

        let _ = std::fs::remove_file(&path);
        assert_eq!(RemoteProfiles::load_from(&path), RemoteProfiles::default());
    }

    #[test]
    fn stored_garbage_is_dropped_on_load() {
        let path = temp_path("garbage");
        std::fs::write(
            &path,
            r#"{"profiles":[{"name":"ok","target":"workbox"},{"name":"bad","target":"-oProxyCommand=x"}],"active":"bad"}"#,
        )
        .unwrap();

        let loaded = RemoteProfiles::load_from(&path);

        assert_eq!(loaded.profiles.len(), 1);
        assert_eq!(loaded.active_target(), None);
        let _ = std::fs::remove_file(&path);
    }
}

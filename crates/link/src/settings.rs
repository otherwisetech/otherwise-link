//! `hub.json` in the shared folder: the switches that apply to every app. Otherwise Apps edits it
//! (Settings → Hub); apps and `otherwise-hub` only read it.
//!
//! ```json
//! { "schema": 1, "entities": true, "mcpWrites": false, "mcpDisabled": ["nodo"],
//!   "background": { "quiet": false, "jobsOff": ["nodo"], "notificationsOff": ["vigia"] } }
//! ```
//!
//! Fields this version doesn't know are kept on save (a newer launcher's settings survive an
//! older one).
use crate::paths::Root;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HubSettings {
    pub schema: u32,
    /// The shared entity registry is on. Off (the default): apps don't open, create or write
    /// `entities.db`.
    pub entities: bool,
    /// `otherwise-hub` offers write tools (its own and the apps'). Off by default.
    pub mcp_writes: bool,
    /// App ids whose tools the hub doesn't proxy.
    pub mcp_disabled: Vec<String>,
    /// Otherwise Helper (background jobs and notifications).
    pub background: BackgroundSettings,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BackgroundSettings {
    /// No notifications at all (jobs still run).
    pub quiet: bool,
    /// App ids whose background jobs don't run.
    pub jobs_off: Vec<String>,
    /// App ids whose notifications aren't shown.
    pub notifications_off: Vec<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl BackgroundSettings {
    pub fn jobs_on(&self, app: &str) -> bool {
        !self.jobs_off.iter().any(|a| a == app)
    }

    pub fn notifications_on(&self, app: &str) -> bool {
        !self.quiet && !self.notifications_off.iter().any(|a| a == app)
    }

    /// Adds or removes `app` from one of the off lists.
    pub fn set(list: &mut Vec<String>, app: &str, on: bool) {
        list.retain(|a| a != app);
        if !on {
            list.push(app.to_string());
            list.sort();
        }
    }
}

impl HubSettings {
    /// Missing or unreadable file: the defaults (everything off).
    pub fn load(root: &Root) -> HubSettings {
        std::fs::read_to_string(root.settings_file()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, root: &Root) -> std::io::Result<()> {
        let mut s = self.clone();
        s.schema = 1;
        crate::fsutil::write_atomic(&root.settings_file(), serde_json::to_string_pretty(&s).unwrap_or_default().as_bytes())
    }

    /// Load, change, save.
    pub fn update(root: &Root, f: impl FnOnce(&mut HubSettings)) -> std::io::Result<HubSettings> {
        let mut s = HubSettings::load(root);
        f(&mut s);
        s.save(root)?;
        Ok(s)
    }

    pub fn mcp_enabled(&self, app: &str) -> bool {
        !self.mcp_disabled.iter().any(|a| a == app)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_switches_and_unknown_fields_survive() {
        let t = tempfile::tempdir().unwrap();
        let root = Root::at(t.path());
        std::fs::write(root.settings_file(), r#"{"schema":1,"entities":true,"future":{"x":1},"background":{"quiet":false,"later":true}}"#).unwrap();
        let s = HubSettings::update(&root, |s| {
            BackgroundSettings::set(&mut s.background.jobs_off, "nodo", false);
            BackgroundSettings::set(&mut s.background.notifications_off, "vigia", false);
        })
        .unwrap();
        assert!(!s.background.jobs_on("nodo") && s.background.jobs_on("expense"));
        assert!(!s.background.notifications_on("vigia") && s.background.notifications_on("expense"));
        let raw = std::fs::read_to_string(root.settings_file()).unwrap();
        assert!(raw.contains("\"future\"") && raw.contains("\"later\""), "{raw}");
        let mut s = HubSettings::load(&root);
        s.background.quiet = true;
        assert!(!s.background.notifications_on("expense"));
        BackgroundSettings::set(&mut s.background.jobs_off, "nodo", true);
        assert!(s.background.jobs_on("nodo"));
        assert_eq!(HubSettings::load(&Root::at(t.path().join("none"))), HubSettings::default());
    }
}

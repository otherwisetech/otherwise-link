//! `hub.json` in the shared folder: the switches that apply to every app. Otherwise Apps edits it
//! (Settings → Hub); apps and `otherwise-hub` only read it.
//!
//! ```json
//! { "schema": 1, "entities": true, "mcpWrites": false, "mcpDisabled": ["nodo"] }
//! ```
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

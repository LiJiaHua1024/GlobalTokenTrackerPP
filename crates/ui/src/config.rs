//! Persisted UI preferences: theme overrides + per-page widget order/visibility.
//! Stored next to ledger.db as `ui.json` — plain JSON so users can hand-edit
//! skins/layouts even without the in-app editor.

use crate::theme::ThemeConfig;
use globaltokentracker_core::store::default_db_path;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub theme: ThemeConfig,
    /// Overview statistics range key: today|week|month|all ("" = week).
    pub range: String,
    /// Checked tool names for the app filter; `None`/absent = all tools.
    /// `Some(empty)` = user unchecked everything (an honest empty view).
    pub apps: Option<Vec<String>>,
    /// page name → layout
    pub pages: BTreeMap<String, PageLayout>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PageLayout {
    /// Widget ids in display order; ids missing here append at the end.
    pub order: Vec<String>,
    /// Widget ids the user hid.
    pub hidden: Vec<String>,
}

pub fn config_path() -> PathBuf {
    default_db_path()
        .parent()
        .map(|p| p.join("ui.json"))
        .unwrap_or_else(|| PathBuf::from("ui.json"))
}

impl UiConfig {
    pub fn load() -> Self {
        std::fs::read_to_string(config_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        if let Ok(s) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(config_path(), s);
        }
    }

    pub fn layout(&self, page: &str) -> PageLayout {
        self.pages.get(page).cloned().unwrap_or_default()
    }

    /// Widget ids in display order: configured order first (known ids only),
    /// then any registry ids not yet in the layout.
    pub fn order_for(&self, page: &str, registry: &[&'static str]) -> Vec<String> {
        let l = self.layout(page);
        let mut out: Vec<String> = Vec::new();
        for id in &l.order {
            if registry.contains(&id.as_str()) && !out.contains(id) {
                out.push(id.clone());
            }
        }
        for id in registry {
            if !out.iter().any(|x| x == id) {
                out.push(id.to_string());
            }
        }
        out
    }

    pub fn hidden(&self, page: &str) -> Vec<String> {
        self.layout(page).hidden
    }

    pub fn move_widget(&mut self, page: &str, id: &str, registry: &[&'static str], delta: i32) {
        let order = self.order_for(page, registry);
        let mut l = self.layout(page);
        let pos = order.iter().position(|x| x == id);
        if let Some(i) = pos {
            let j = (i as i32 + delta).clamp(0, order.len() as i32 - 1) as usize;
            if j != i {
                let mut order = order;
                order.swap(i, j);
                l.order = order;
            }
        }
        // ensure new order persisted even if unchanged
        if l.order.is_empty() {
            l.order = self.order_for(page, registry);
        }
        self.pages.insert(page.to_string(), l);
        self.save();
    }

    pub fn set_hidden(&mut self, page: &str, id: &str, hidden: bool) {
        let mut l = self.layout(page);
        l.hidden.retain(|x| x != id);
        if hidden {
            l.hidden.push(id.to_string());
        }
        self.pages.insert(page.to_string(), l);
        self.save();
    }
}

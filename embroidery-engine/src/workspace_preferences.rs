//! Editor chrome preferences, deliberately separate from embroidery projects.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(super) struct WorkspacePreferences {
    pub theme: ui_theme::Theme,
    pub layers_open: bool,
    pub inspector_open: bool,
    pub timeline_open: bool,
    pub grid: bool,
    pub rulers: bool,
    pub hints: bool,
    pub layers_width: f32,
    pub inspector_width: f32,
    pub timeline_height: f32,
}

impl Default for WorkspacePreferences {
    fn default() -> Self {
        Self {
            theme: ui_theme::Theme::Dark,
            layers_open: true,
            inspector_open: true,
            timeline_open: true,
            grid: true,
            rulers: false,
            hints: false,
            layers_width: 228.,
            inspector_width: 278.,
            timeline_height: 105.,
        }
    }
}

impl WorkspacePreferences {
    fn bounded(mut self) -> Self {
        for (value, fallback, min, max) in [
            (&mut self.layers_width, 228., 180., 380.),
            (&mut self.inspector_width, 278., 240., 420.),
            (&mut self.timeline_height, 105., 88., 240.),
        ] {
            *value = if value.is_finite() {
                value.clamp(min, max)
            } else {
                fallback
            };
        }
        self
    }

    pub fn load() -> Self {
        i18n::preferences_path()
            .and_then(|path| std::fs::read(path).ok())
            .filter(|bytes| bytes.len() <= 256 * 1024)
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| value.get("workspace").cloned())
            .and_then(|value| serde_json::from_value::<Self>(value).ok())
            .unwrap_or_default()
            .bounded()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = i18n::preferences_path().ok_or("Cannot locate the preferences folder")?;
        self.save_at(&path)
    }

    fn save_at(&self, path: &std::path::Path) -> Result<(), String> {
        let mut preferences = match std::fs::read(path) {
            Ok(bytes) if bytes.len() <= 256 * 1024 => {
                serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&bytes)
                    .map_err(|error| format!("Cannot read workspace preferences: {error}"))?
            }
            Ok(_) => return Err("Workspace preferences exceed 256 KiB".into()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Default::default(),
            Err(error) => return Err(format!("Cannot read workspace preferences: {error}")),
        };
        preferences.insert(
            "workspace".into(),
            serde_json::to_value(self.bounded_clone()).map_err(|e| e.to_string())?,
        );
        let bytes = serde_json::to_vec_pretty(&preferences).map_err(|e| e.to_string())?;
        embroidery_lab::persistence::atomic_write(path, &bytes)
    }

    fn bounded_clone(&self) -> Self {
        self.clone().bounded()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_save_preserves_language_and_does_not_overwrite_invalid_json() {
        let path =
            std::env::temp_dir().join(format!("embroidery-workspace-{}.json", std::process::id()));
        std::fs::write(&path, br#"{"language":"en","future":42}"#).unwrap();
        let preferences = WorkspacePreferences::default();
        preferences.save_at(&path).unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["language"], "en");
        assert_eq!(saved["future"], 42);
        assert_eq!(
            serde_json::from_value::<WorkspacePreferences>(saved["workspace"].clone()).unwrap(),
            preferences
        );
        std::fs::write(&path, b"invalid").unwrap();
        assert!(preferences.save_at(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid");
        let _ = std::fs::remove_file(path);
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The app's settings, in `~/Library/Application Support/Persimmon/
//! settings.json` (DD-48). A missing file means defaults; a damaged one is
//! kept aside as `settings.json.bad`, never silently lost.

use std::path::{Path, PathBuf};

use persimmon_core::output::OutputPolicy;

use crate::queue::QueueMode;

/// Where converted files go (spec D3).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum OutputMode {
    /// Beside the original, falling back when its folder can't be written.
    Beside,
    /// Always in this folder.
    Folder { folder: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub output: OutputMode,
    /// Where output goes when the original's folder can't be written.
    pub fallback: PathBuf,
    pub mode: QueueMode,
    pub drop_window: bool,
}

impl Default for Settings {
    fn default() -> Settings {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        Settings {
            output: OutputMode::Beside,
            fallback: home.join("Downloads").join("Persimmon"),
            mode: QueueMode::Convert,
            drop_window: false,
        }
    }
}

/// `data_dir` is `~/Library/Application Support`.
pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join("Persimmon").join("settings.json")
}

/// The settings at `path`: defaults when there are none, and when the file
/// is damaged, which is then kept beside it as `settings.json.bad`.
pub fn load(path: &Path) -> Settings {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Settings::default();
    };
    match serde_json::from_str(&text) {
        Ok(settings) => settings,
        Err(_) => {
            let _ = std::fs::rename(path, path.with_extension("json.bad"));
            Settings::default()
        }
    }
}

/// Write the settings through a temporary file renamed into place.
pub fn save(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("json.part");
    let text = serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?;
    std::fs::write(&temp, text)?;
    std::fs::rename(&temp, path)
}

/// The engine's output policy for these settings. The app never
/// overwrites (DD-42).
pub fn output_policy(settings: &Settings) -> OutputPolicy {
    match &settings.output {
        OutputMode::Beside => OutputPolicy {
            dir: None,
            overwrite: false,
            fallback: Some(settings.fallback.clone()),
        },
        OutputMode::Folder { folder } => OutputPolicy {
            dir: Some(folder.clone()),
            overwrite: false,
            fallback: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Dir {
            let d = std::env::temp_dir()
                .join(format!("prsm-app-settings-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Dir(d)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn home() -> PathBuf {
        PathBuf::from(std::env::var_os("HOME").unwrap())
    }

    #[test]
    fn defaults_when_missing() {
        let d = Dir::new("missing");
        let s = load(&d.0.join("settings.json"));
        assert_eq!(s.output, OutputMode::Beside);
        assert_eq!(s.fallback, home().join("Downloads/Persimmon"));
        assert_eq!(s.mode, QueueMode::Convert);
        assert!(!s.drop_window);
    }

    #[test]
    fn the_path_is_application_support_persimmon() {
        assert_eq!(
            settings_path(Path::new("/Users/x/Library/Application Support")),
            PathBuf::from("/Users/x/Library/Application Support/Persimmon/settings.json")
        );
    }

    #[test]
    fn round_trips() {
        let d = Dir::new("round");
        let path = d.0.join("Persimmon/settings.json");
        let s = Settings {
            output: OutputMode::Folder {
                folder: "/x/out".into(),
            },
            fallback: "/x/fb".into(),
            mode: QueueMode::Combine,
            drop_window: true,
        };
        save(&path, &s).unwrap();
        assert_eq!(load(&path), s);
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(
            json["output"],
            serde_json::json!({"mode": "folder", "folder": "/x/out"})
        );
        assert_eq!(json["mode"], "combine");
        assert_eq!(json["dropWindow"], true);
    }

    #[test]
    fn damaged_settings_are_kept_aside() {
        let d = Dir::new("damaged");
        let path = d.0.join("settings.json");
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load(&path), Settings::default());
        assert_eq!(
            std::fs::read_to_string(d.0.join("settings.json.bad")).unwrap(),
            "{not json"
        );
        assert!(!path.exists());
    }

    #[test]
    fn unknown_fields_and_missing_fields_are_fine() {
        let d = Dir::new("partial");
        let path = d.0.join("settings.json");
        std::fs::write(&path, r#"{"dropWindow":true,"future":1}"#).unwrap();
        let s = load(&path);
        assert!(s.drop_window);
        assert_eq!(s.output, OutputMode::Beside);
    }

    #[test]
    fn policy_for_each_mode() {
        let mut s = Settings::default();
        let p = output_policy(&s);
        assert_eq!(
            (p.dir, p.fallback, p.overwrite),
            (None, Some(s.fallback.clone()), false)
        );
        s.output = OutputMode::Folder {
            folder: "/x/out".into(),
        };
        let p = output_policy(&s);
        assert_eq!(
            (p.dir, p.fallback, p.overwrite),
            (Some("/x/out".into()), None, false)
        );
    }

    #[test]
    fn save_is_atomic() {
        let d = Dir::new("atomic");
        let path = d.0.join("settings.json");
        save(&path, &Settings::default()).unwrap();
        let names: Vec<String> = std::fs::read_dir(&d.0)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["settings.json"]);
    }
}

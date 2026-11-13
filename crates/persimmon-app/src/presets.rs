// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Presets (spec D4): a name, a mode, a target and today's engine options.
//! Three are built in; the user's own are one JSON file each under
//! `~/Library/Application Support/Persimmon/presets`. Each shows the
//! `prsm` command it amounts to (DD-18).

use std::path::{Path, PathBuf};

use persimmon_core::format::{FormatId, MediaKind};
use persimmon_core::job::JobOptions;
use persimmon_core::registry::Registry;

use crate::queue::QueueMode;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub builtin: bool,
    pub mode: QueueMode,
    pub target: Option<FormatId>,
    #[serde(default)]
    pub options: JobOptions,
}

pub fn presets_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("Persimmon").join("presets")
}

pub fn builtins() -> Vec<Preset> {
    use persimmon_core::job::{Preset as Video, Quality};
    let preset = |id: &str, name: &str, target, options| Preset {
        id: id.into(),
        name: name.into(),
        builtin: true,
        mode: QueueMode::Convert,
        target: Some(target),
        options,
    };
    vec![
        preset(
            "web-image",
            "Web image",
            FormatId::Webp,
            JobOptions {
                quality: Some(Quality::Medium),
                max_size: Some(2048),
                strip_metadata: true,
                ..JobOptions::default()
            },
        ),
        preset(
            "email-video",
            "Email video",
            FormatId::Mp4,
            JobOptions {
                preset: Some(Video::Compatible),
                max_size: Some(1280),
                quality: Some(Quality::Low),
                strip_metadata: true,
                ..JobOptions::default()
            },
        ),
        preset(
            "archive-quality",
            "Archive quality",
            FormatId::Mp4,
            JobOptions {
                preset: Some(Video::Compatible),
                quality: Some(Quality::Max),
                ..JobOptions::default()
            },
        ),
    ]
}

fn verb(mode: QueueMode) -> &'static str {
    match mode {
        QueueMode::Convert => "convert",
        QueueMode::Compress => "compress",
        QueueMode::Extract => "extract",
        QueueMode::Combine => "merge",
    }
}

/// The `prsm` command a preset amounts to, for the editor to show.
pub fn command(p: &Preset) -> String {
    use persimmon_core::job::{Preset as Video, Quality};
    let mut line = format!("prsm {} <files>", verb(p.mode));
    if let Some(t) = p.target {
        line.push_str(&format!(" --to {}", t.as_str()));
    }
    // `prsm merge` and `prsm extract` take none of the options (DD-18).
    if !takes_options(p.mode) {
        return line;
    }
    if let Some(v) = p.options.preset {
        line.push_str(match v {
            Video::Compatible => " --preset compatible",
            Video::Small => " --preset small",
        });
    }
    if let Some(q) = p.options.quality {
        line.push_str(match q {
            Quality::Low => " --quality low",
            Quality::Medium => " --quality medium",
            Quality::High => " --quality high",
            Quality::Max => " --quality max",
        });
    }
    if let Some(m) = p.options.max_size {
        line.push_str(&format!(" --max-size {m}"));
    }
    if p.options.strip_metadata {
        line.push_str(" --strip-metadata");
    }
    line
}

pub fn operation(mode: QueueMode) -> persimmon_core::registry::Operation {
    use persimmon_core::registry::Operation;
    match mode {
        QueueMode::Convert => Operation::Convert,
        QueueMode::Compress => Operation::Compress,
        QueueMode::Extract => Operation::Extract,
        QueueMode::Combine => Operation::Merge,
    }
}

/// The kinds of file a preset can take, in a fixed order.
pub fn kinds(p: &Preset, reg: &Registry) -> Vec<MediaKind> {
    // A video preset (compatible, small) makes a Compress preset video-only.
    if p.mode == QueueMode::Compress && p.options.preset.is_some() {
        return vec![MediaKind::Video];
    }
    let op = operation(p.mode);
    [
        MediaKind::Image,
        MediaKind::Video,
        MediaKind::Audio,
        MediaKind::Document,
        MediaKind::Archive,
    ]
    .into_iter()
    .filter(|&k| {
        reg.formats()
            .iter()
            .filter(|f| f.kind == k)
            .any(|f| reg.find(op, f.id, p.target).is_some())
    })
    .collect()
}

/// Whether a mode's command takes quality, size, preset or metadata
/// options: Combine and Extract don't.
fn takes_options(mode: QueueMode) -> bool {
    matches!(mode, QueueMode::Convert | QueueMode::Compress)
}

/// Refuse options the engine would refuse for this target (DD-35).
pub fn check(p: &Preset) -> Result<(), String> {
    use persimmon_core::registry::Registry;
    let reg = Registry::builtin();
    let o = &p.options;
    if !takes_options(p.mode)
        && (o.quality.is_some() || o.preset.is_some() || o.max_size.is_some() || o.strip_metadata)
    {
        return Err("Combining and extracting take no quality, size or metadata options".into());
    }
    let target_kind = p.target.map(|t| reg.format(t).kind);
    if p.options.preset.is_some()
        && p.mode != QueueMode::Compress
        && !matches!(
            p.target,
            Some(FormatId::Mp4 | FormatId::Mov | FormatId::Webm)
        )
    {
        return Err("A video preset needs an MP4, MOV or WebM target".into());
    }
    if p.options.max_size.is_some()
        && matches!(
            target_kind,
            Some(MediaKind::Audio | MediaKind::Document | MediaKind::Archive)
        )
    {
        return Err("A size limit is for images and video".into());
    }
    if p.options.pages.is_some() {
        return Err("Page ranges aren't part of presets in this version".into());
    }
    if p.mode == QueueMode::Convert && p.target.is_none() {
        return Err("Pick an output format".into());
    }
    Ok(())
}

/// The built-in presets and the user's own.
pub struct Presets {
    dir: PathBuf,
    list: Vec<Preset>,
    /// The file each of the user's presets was loaded from or saved to,
    /// which may not be `{id}.json`.
    files: std::collections::HashMap<String, PathBuf>,
}

/// A preset id is a file name of its own: `[a-z0-9-]+`, so it can't leave
/// the presets folder.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().to_lowercase().chars() {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() { "preset".into() } else { out }
}

impl Presets {
    /// The built-ins, then the user's own sorted by name. A file that
    /// doesn't read as a preset is kept aside as `.bad`.
    pub fn load(dir: &Path) -> Presets {
        let mut mine = Vec::new();
        let mut files = std::collections::HashMap::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_none_or(|e| e != "json") {
                    continue;
                }
                let parsed = std::fs::read(&path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Preset>(&b).ok());
                match parsed {
                    Some(mut p) => {
                        p.builtin = false;
                        if !valid_id(&p.id) {
                            p.id = slug(&path.file_stem().unwrap_or_default().to_string_lossy());
                        }
                        files.insert(p.id.clone(), path.clone());
                        mine.push(p);
                    }
                    None => {
                        let _ = std::fs::rename(&path, path.with_extension("json.bad"));
                    }
                }
            }
        }
        let mut presets = Presets {
            dir: dir.to_path_buf(),
            list: builtins(),
            files,
        };
        presets.list.extend(mine);
        presets.sort();
        presets
    }

    fn sort(&mut self) {
        let (mut builtin, mut mine): (Vec<Preset>, Vec<Preset>) =
            self.list.drain(..).partition(|p| p.builtin);
        builtin.sort_by_key(|p| builtins().iter().position(|b| b.id == p.id));
        mine.sort_by_key(|p| p.name.to_lowercase());
        self.list = builtin.into_iter().chain(mine).collect();
    }

    pub fn all(&self) -> &[Preset] {
        &self.list
    }

    pub fn get(&self, id: &str) -> Option<&Preset> {
        self.list.iter().find(|p| p.id == id)
    }

    fn free_id(&self, base: &str) -> String {
        let taken = |id: &str| self.list.iter().any(|p| p.id == id);
        if !taken(base) {
            return base.to_string();
        }
        (2..)
            .map(|n| format!("{base}-{n}"))
            .find(|id| !taken(id))
            .unwrap_or_default()
    }

    /// Save one of the user's presets. A built-in is saved as a copy; a new
    /// preset (no id) gets one from its name. Names must be unique and not
    /// empty, and the options must be ones the engine takes.
    pub fn save(&mut self, preset: Preset) -> Result<Preset, String> {
        let mut p = preset;
        p.name = p.name.trim().to_string();
        if p.name.is_empty() {
            return Err("Give the preset a name".into());
        }
        check(&p)?;
        let is_builtin = p.builtin || builtins().iter().any(|b| b.id == p.id);
        if is_builtin {
            p.id = self.free_id(&format!("{}-copy", p.id));
            p.builtin = false;
            // A copy needs a name of its own: "Web image copy", "… copy 2".
            if self
                .list
                .iter()
                .any(|o| o.name.eq_ignore_ascii_case(&p.name))
            {
                let base = format!("{} copy", p.name);
                let taken = |n: &str| self.list.iter().any(|o| o.name.eq_ignore_ascii_case(n));
                p.name = if taken(&base) {
                    (2..)
                        .map(|n| format!("{base} {n}"))
                        .find(|n| !taken(n))
                        .unwrap_or(base)
                } else {
                    base
                };
            }
        } else if p.id.is_empty() {
            p.id = self.free_id(&slug(&p.name));
        } else if !valid_id(&p.id) {
            return Err("This preset can't be saved under that id".into());
        }
        let clash = self
            .list
            .iter()
            .any(|o| o.id != p.id && o.name.eq_ignore_ascii_case(&p.name));
        if clash {
            return Err(format!("There's already a preset called {}", p.name));
        }
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let path = self.dir.join(format!("{}.json", p.id));
        let temp = path.with_extension("json.part");
        let text = serde_json::to_string_pretty(&p).map_err(|e| e.to_string())?;
        std::fs::write(&temp, text)
            .and_then(|_| std::fs::rename(&temp, &path))
            .map_err(|e| e.to_string())?;
        // Loaded from a file of another name: that one goes, or the old
        // version would come back next launch.
        if let Some(old) = self.files.insert(p.id.clone(), path.clone())
            && old != path
        {
            let _ = std::fs::remove_file(old);
        }
        self.list.retain(|o| o.id != p.id);
        self.list.push(p.clone());
        self.sort();
        Ok(p)
    }

    pub fn delete(&mut self, id: &str) -> Result<(), String> {
        match self.get(id) {
            None => Ok(()),
            Some(p) if p.builtin => Err("Built-in presets can't be deleted".into()),
            Some(_) => {
                let file = self
                    .files
                    .remove(id)
                    .unwrap_or_else(|| self.dir.join(format!("{id}.json")));
                let _ = std::fs::remove_file(file);
                self.list.retain(|p| p.id != id);
                Ok(())
            }
        }
    }

    /// The preset ⌘`n` picks, 1 to 9.
    pub fn shortcut(&self, n: usize) -> Option<&Preset> {
        (1..=9).contains(&n).then(|| self.list.get(n - 1)).flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use persimmon_core::job::{Preset as VideoPreset, Quality};

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Dir {
            let d = std::env::temp_dir()
                .join(format!("prsm-app-presets-{name}-{}", std::process::id()));
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

    fn mine(name: &str) -> Preset {
        Preset {
            id: String::new(),
            name: name.into(),
            builtin: false,
            mode: QueueMode::Convert,
            target: Some(FormatId::Png),
            options: JobOptions::default(),
        }
    }

    #[test]
    fn the_folder_is_application_support_persimmon_presets() {
        assert_eq!(
            presets_dir(Path::new("/Users/x/Library/Application Support")),
            PathBuf::from("/Users/x/Library/Application Support/Persimmon/presets")
        );
    }

    #[test]
    fn builtins_are_the_three_from_the_spec() {
        let b = builtins();
        let ids: Vec<&str> = b.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["web-image", "email-video", "archive-quality"]);
        assert!(b.iter().all(|p| p.builtin && p.mode == QueueMode::Convert));
        assert_eq!(b[0].name, "Web image");
        assert_eq!(b[0].target, Some(FormatId::Webp));
        assert_eq!(b[0].options.quality, Some(Quality::Medium));
        assert_eq!(b[0].options.max_size, Some(2048));
        assert!(b[0].options.strip_metadata);
        assert_eq!(b[1].name, "Email video");
        assert_eq!(b[1].target, Some(FormatId::Mp4));
        assert_eq!(b[1].options.preset, Some(VideoPreset::Compatible));
        assert_eq!(b[1].options.max_size, Some(1280));
        assert_eq!(b[1].options.quality, Some(Quality::Low));
        assert!(b[1].options.strip_metadata);
        assert_eq!(b[2].name, "Archive quality");
        assert_eq!(b[2].target, Some(FormatId::Mp4));
        assert_eq!(b[2].options.preset, Some(VideoPreset::Compatible));
        assert_eq!(b[2].options.quality, Some(Quality::Max));
        assert!(!b[2].options.strip_metadata);
    }

    #[test]
    fn commands_teach_the_cli() {
        let b = builtins();
        assert_eq!(
            command(&b[0]),
            "prsm convert <files> --to webp --quality medium --max-size 2048 --strip-metadata"
        );
        assert_eq!(
            command(&b[2]),
            "prsm convert <files> --to mp4 --preset compatible --quality max"
        );
        let combine = Preset {
            mode: QueueMode::Combine,
            target: Some(FormatId::TarGz),
            ..mine("x")
        };
        assert_eq!(command(&combine), "prsm merge <files> --to tar.gz");
        let compress = Preset {
            mode: QueueMode::Compress,
            target: None,
            ..mine("x")
        };
        assert_eq!(command(&compress), "prsm compress <files>");
    }

    #[test]
    fn save_refuses_an_id_that_isnt_a_slug() {
        let d = Dir::new("slug");
        let mut presets = Presets::load(&d.0.join("presets"));
        for id in ["../x", "a/b", "Up"] {
            let mut p = mine(&format!("P {id}"));
            p.id = id.into();
            assert!(presets.save(p).is_err(), "{id}");
        }
        assert!(!d.0.join("x.json").exists());
        assert!(!d.0.join("presets/a").exists());
    }

    #[test]
    fn delete_removes_the_file_the_preset_came_from() {
        let d = Dir::new("file");
        let mut p = mine("Other");
        p.id = "other".into();
        std::fs::write(d.0.join("custom.json"), serde_json::to_string(&p).unwrap()).unwrap();
        let mut presets = Presets::load(&d.0);
        presets.delete("other").unwrap();
        assert!(!d.0.join("custom.json").exists());
        assert!(Presets::load(&d.0).get("other").is_none());
    }

    #[test]
    fn a_new_preset_cant_take_a_builtins_name() {
        let d = Dir::new("taken");
        let mut presets = Presets::load(&d.0);
        assert_eq!(
            presets.save(mine("Web image")).unwrap_err(),
            "There's already a preset called Web image"
        );
    }

    #[test]
    fn combine_and_extract_presets_carry_no_options() {
        for mode in [QueueMode::Combine, QueueMode::Extract] {
            let mut p = mine("Bundle");
            p.mode = mode;
            p.target = (mode == QueueMode::Combine).then_some(FormatId::Zip);
            p.options.quality = Some(Quality::High);
            assert!(check(&p).is_err(), "{mode:?} with a quality");
            assert!(!command(&p).contains("--quality"), "{mode:?}");
            p.options.quality = None;
            p.options.strip_metadata = true;
            assert!(check(&p).is_err(), "{mode:?} stripping metadata");
            assert!(!command(&p).contains("--strip-metadata"), "{mode:?}");
        }
    }

    #[test]
    fn editing_a_builtin_saves_a_copy() {
        let d = Dir::new("copy");
        let mut presets = Presets::load(&d.0);
        let mut web = presets.get("web-image").unwrap().clone();
        web.options.max_size = Some(1024);
        let saved = presets.save(web).unwrap();
        assert_eq!(saved.id, "web-image-copy");
        assert_eq!(saved.name, "Web image copy");
        assert!(!saved.builtin);
        assert_eq!(
            presets.get("web-image").unwrap().options.max_size,
            Some(2048)
        );
        assert!(d.0.join("web-image-copy.json").is_file());
        // Mine come after the built-ins, so ⌘4 is the copy.
        assert_eq!(presets.shortcut(4).unwrap().id, "web-image-copy");
        let again = presets
            .save(presets.get("web-image").unwrap().clone())
            .unwrap();
        assert_eq!(again.id, "web-image-copy-2");
        assert_eq!(again.name, "Web image copy 2");
        assert_eq!(Presets::load(&d.0).all().len(), 5);
    }

    #[test]
    fn names_are_unique_and_not_empty() {
        let d = Dir::new("names");
        let mut presets = Presets::load(&d.0);
        assert!(presets.save(mine("  ")).is_err());
        let a = presets.save(mine("Thumbs")).unwrap();
        assert_eq!(a.id, "thumbs");
        assert!(presets.save(mine("Thumbs")).is_err());
        let mut renamed = a.clone();
        renamed.options.strip_metadata = true;
        assert!(
            presets.save(renamed).is_ok(),
            "saving a preset under its own name is fine"
        );
    }

    #[test]
    fn builtins_cant_be_deleted() {
        let d = Dir::new("delete");
        let mut presets = Presets::load(&d.0);
        assert_eq!(
            presets.delete("web-image").unwrap_err(),
            "Built-in presets can't be deleted"
        );
        let a = presets.save(mine("Thumbs")).unwrap();
        presets.delete(&a.id).unwrap();
        assert!(presets.get(&a.id).is_none());
        assert!(!d.0.join("thumbs.json").exists());
    }

    #[test]
    fn a_damaged_preset_file_is_kept_aside() {
        let d = Dir::new("damaged");
        std::fs::write(d.0.join("broken.json"), "{nope").unwrap();
        let presets = Presets::load(&d.0);
        assert_eq!(presets.all().len(), 3);
        assert!(d.0.join("broken.json.bad").is_file());
    }

    #[test]
    fn shortcuts_follow_the_list() {
        let d = Dir::new("shortcuts");
        let presets = Presets::load(&d.0);
        assert_eq!(presets.shortcut(1).unwrap().id, "web-image");
        assert_eq!(presets.shortcut(3).unwrap().id, "archive-quality");
        assert!(presets.shortcut(4).is_none());
        assert!(presets.shortcut(0).is_none() && presets.shortcut(10).is_none());
    }

    #[test]
    fn check_rejects_options_the_engine_would_refuse() {
        let mut small_png = mine("x");
        small_png.options.preset = Some(VideoPreset::Small);
        assert!(check(&small_png).is_err());
        let mut sized_mp3 = mine("x");
        sized_mp3.target = Some(FormatId::Mp3);
        sized_mp3.options.max_size = Some(100);
        assert!(check(&sized_mp3).is_err());
        assert!(builtins().iter().all(|p| check(p).is_ok()));
        let d = Dir::new("check");
        assert!(Presets::load(&d.0).save(small_png).is_err());
    }

    #[test]
    fn kinds_of_each_builtin() {
        let reg = Registry::builtin();
        let b = builtins();
        assert_eq!(kinds(&b[0], &reg), vec![MediaKind::Image]);
        assert_eq!(kinds(&b[1], &reg), vec![MediaKind::Video]);
    }
}

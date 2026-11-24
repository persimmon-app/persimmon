// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Watch rules (spec D2): what each watches, what it makes, and where
//! results and finished originals go. Kept in
//! `~/Library/Application Support/Persimmon/watch/rules.json`.

use std::path::{Path, PathBuf};

use persimmon_core::error::ErrorCode;
use persimmon_core::format::FormatId;
use persimmon_core::registry::Registry;
use persimmon_core::watch::{Patterns, WatchSpec};

use crate::presets::Presets;
use crate::queue::{QueueMode, tilde};
use crate::wording::{Shown, app_error};

/// One watch rule: the canvas's fields.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub id: String,
    pub folder: PathBuf,
    pub target: FormatId,
    /// A preset's id; its options apply.
    pub preset: Option<String>,
    /// Match files, as typed: `*.jpg, *.png`.
    pub patterns: String,
    pub results: PathBuf,
    /// Where originals move after success; `None` leaves them.
    pub done: Option<PathBuf>,
    pub recursive: bool,
    /// Wait until the file stops growing.
    pub settle: bool,
    pub notify_each: bool,
    pub enabled: bool,
}

/// A new rule for `folder`, with the spec's defaults.
pub fn draft(reg: &Registry, folder: PathBuf, target: FormatId) -> Rule {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Rule {
        id: format!("{nanos:x}"),
        results: folder.join("converted"),
        patterns: Patterns::for_target(reg, target).to_string(),
        folder,
        target,
        preset: None,
        done: None,
        recursive: false,
        settle: true,
        notify_each: false,
        enabled: true,
    }
}

/// `data` is `~/Library/Application Support/Persimmon`.
pub fn rules_path(data: &Path) -> PathBuf {
    data.join("watch").join("rules.json")
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
struct Stored {
    rules: Vec<Rule>,
}

/// Every rule, as saved.
pub struct Rules {
    path: PathBuf,
    rules: Vec<Rule>,
}

impl Rules {
    /// The rules at `path`: none when there's no file, and none when it's
    /// damaged, which is then kept aside as `rules.json.bad`.
    pub fn open(path: &Path) -> Rules {
        let rules = match std::fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<Stored>(&bytes) {
                Ok(stored) => stored.rules,
                Err(_) => {
                    let _ = std::fs::rename(path, path.with_extension("json.bad"));
                    Vec::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(_) => {
                let _ = std::fs::rename(path, path.with_extension("json.bad"));
                Vec::new()
            }
        };
        Rules {
            path: path.to_path_buf(),
            rules,
        }
    }

    pub fn all(&self) -> &[Rule] {
        &self.rules
    }

    pub fn get(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.id == id)
    }

    /// Add `rule`, or replace the one with its id, and save.
    pub fn put(&mut self, rule: Rule) -> std::io::Result<()> {
        match self.rules.iter_mut().find(|r| r.id == rule.id) {
            Some(r) => *r = rule,
            None => self.rules.push(rule),
        }
        self.save()
    }

    pub fn remove(&mut self, id: &str) -> std::io::Result<()> {
        self.rules.retain(|r| r.id != id);
        self.save()
    }

    /// A rule that uses preset `id`, if any.
    pub fn using_preset(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|r| r.preset.as_deref() == Some(id))
    }

    fn save(&self) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_vec_pretty(&Stored {
            rules: self.rules.clone(),
        })
        .map_err(std::io::Error::other)?;
        let temp = self.path.with_extension("json.part");
        std::fs::write(&temp, text)?;
        std::fs::rename(&temp, &self.path)
    }
}

/// The rule as the engine's spec, and its preset's name. Reads nothing:
/// options come from the preset, or the defaults when it's gone, and
/// patterns that don't parse fall back to the target's defaults.
pub fn spec(rule: &Rule, presets: &Presets, reg: &Registry) -> (WatchSpec, Option<String>) {
    let preset = rule.preset.as_deref().and_then(|id| presets.get(id));
    let mut spec = WatchSpec::new(reg, rule.folder.clone(), rule.target);
    spec.options = preset.map(|p| p.options.clone()).unwrap_or_default();
    if let Ok(p) = Patterns::parse(&rule.patterns) {
        spec.patterns = p;
    }
    spec.results = rule.results.clone();
    spec.done = rule.done.clone();
    spec.recursive = rule.recursive;
    spec.settle = rule.settle;
    spec.existing = true;
    (spec, preset.map(|p| p.name.clone()))
}

const CANT_READ: &str = "Persimmon can't read this folder";
const ALLOW: &str = "Allow Persimmon in System Settings → Privacy & Security → Files and Folders";

/// `spec`, after every refusal a rule can meet when saved. Reads the
/// folder, so macOS asks for access now, while someone is there.
pub fn check(
    rule: &Rule,
    others: &[Rule],
    presets: &Presets,
    reg: &Registry,
) -> Result<(WatchSpec, Option<String>), Shown> {
    let refuse =
        |code, file: &Path, reason: &str, fix: &str| Err(app_error(code, file, reason, Some(fix)));
    if Patterns::parse(&rule.patterns).is_err() {
        return Err(Shown {
            code: ErrorCode::Usage,
            file: None,
            reason: "Match files needs at least one pattern".into(),
            fix: Some("Type a pattern such as *.jpg.".into()),
        });
    }
    if let Some(id) = &rule.preset {
        match presets.get(id) {
            None => {
                return refuse(
                    ErrorCode::Usage,
                    &rule.folder,
                    "That preset no longer exists",
                    "Choose another preset.",
                );
            }
            Some(p)
                if p.mode != QueueMode::Convert || p.target.is_some_and(|t| t != rule.target) =>
            {
                let makes = p
                    .target
                    .map(|t| t.as_str().to_uppercase())
                    .unwrap_or_else(|| "something else".into());
                return refuse(
                    ErrorCode::Usage,
                    &rule.folder,
                    &format!(
                        "{} makes {makes}, not {}",
                        p.name,
                        rule.target.as_str().to_uppercase()
                    ),
                    "Choose another preset.",
                );
            }
            Some(_) => {}
        }
    }
    if !rule.folder.exists() {
        return refuse(
            ErrorCode::InputNotFound,
            &rule.folder,
            "This folder doesn't exist",
            "Choose another folder.",
        );
    }
    match std::fs::metadata(&rule.folder) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => {
            return refuse(
                ErrorCode::Usage,
                &rule.folder,
                "This isn't a folder",
                "Choose another folder.",
            );
        }
        Err(_) => {
            return refuse(ErrorCode::UnsupportedInput, &rule.folder, CANT_READ, ALLOW);
        }
    }
    if std::fs::read_dir(&rule.folder).is_err() {
        return refuse(ErrorCode::UnsupportedInput, &rule.folder, CANT_READ, ALLOW);
    }
    let folder = resolved(&rule.folder);
    let others: Vec<PathBuf> = others
        .iter()
        .filter(|o| o.id != rule.id)
        .map(|o| resolved(&o.folder))
        .collect();
    if others
        .iter()
        .any(|o| folder.starts_with(o) || o.starts_with(&folder))
    {
        return refuse(
            ErrorCode::Usage,
            &rule.folder,
            "Another rule already watches this folder",
            "Choose another folder.",
        );
    }
    let results = resolved(&rule.results);
    if folder.starts_with(&results) {
        return refuse(
            ErrorCode::Usage,
            &rule.results,
            "Results would be converted again",
            "Choose another folder for results.",
        );
    }
    if others.iter().any(|o| results.starts_with(o)) {
        return refuse(
            ErrorCode::Usage,
            &rule.results,
            "Another rule watches this folder, so results would be converted again",
            "Choose another folder for results.",
        );
    }
    if let Some(done) = &rule.done {
        let resolved_done = resolved(done);
        if folder.starts_with(&resolved_done) {
            return refuse(
                ErrorCode::Usage,
                done,
                "Finished originals would be converted again",
                "Choose another folder for finished originals.",
            );
        }
        if others.iter().any(|o| resolved_done.starts_with(o)) {
            return refuse(
                ErrorCode::Usage,
                done,
                "Another rule watches this folder, so finished originals would be converted again",
                "Choose another folder for finished originals.",
            );
        }
    }
    Ok(spec(rule, presets, reg))
}

/// `path` with the part that exists resolved (symlinks, `/var` as
/// `/private/var`), and the rest as written.
fn resolved(path: &Path) -> PathBuf {
    let mut existing = path.to_path_buf();
    let mut rest = Vec::new();
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
    let mut out = std::fs::canonicalize(&existing).unwrap_or(existing);
    for name in rest.into_iter().rev() {
        out.push(name);
    }
    out
}

/// A path for the command line: `~` for home, and quoted after it when it
/// holds anything a shell would split or expand, so `~` still expands.
fn arg(path: &Path) -> String {
    let shown = tilde(path);
    let plain = |s: &str| {
        s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-~+,:@".contains(c))
    };
    if plain(&shown) {
        return shown;
    }
    let quote = |s: &str| format!("'{}'", s.replace('\'', r"'\''"));
    match shown.strip_prefix("~/") {
        Some(rest) => format!("~/{}", quote(rest)),
        None => quote(&shown),
    }
}

/// The `prsm watch` line the rule amounts to (DD-18), its preset's
/// options written out.
pub fn command(rule: &Rule, presets: &Presets) -> String {
    let options = rule
        .preset
        .as_deref()
        .and_then(|id| presets.get(id))
        .map(|p| p.options.clone())
        .unwrap_or_default();
    let patterns = Patterns::parse(&rule.patterns)
        .map(|p| p.to_string())
        .unwrap_or_else(|_| rule.patterns.clone());
    let mut line = format!(
        "prsm watch {} --to {}{} --match \"{}\" --out {}",
        arg(&rule.folder),
        rule.target.as_str(),
        crate::presets::option_args(&options),
        patterns,
        arg(&rule.results),
    );
    if let Some(done) = &rule.done {
        line.push_str(&format!(" --then-move {}", arg(done)));
    }
    if rule.recursive {
        line.push_str(" --recursive");
    }
    if !rule.settle {
        line.push_str(" --no-wait");
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use persimmon_core::job::{JobOptions, Quality};

    use crate::presets::Preset;

    struct Dir(PathBuf);
    impl Dir {
        fn new(name: &str) -> Dir {
            let d =
                std::env::temp_dir().join(format!("prsm-app-rule-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Dir(d)
        }
        fn folder(&self, rel: &str) -> PathBuf {
            let p = self.0.join(rel);
            std::fs::create_dir_all(&p).unwrap();
            p
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn reg() -> Registry {
        Registry::builtin()
    }

    fn presets(d: &Dir) -> Presets {
        Presets::load(&d.0.join("presets"))
    }

    fn mine(presets: &mut Presets, target: FormatId) -> Preset {
        presets
            .save(Preset {
                id: String::new(),
                name: "Small web".into(),
                builtin: false,
                mode: QueueMode::Convert,
                target: Some(target),
                options: JobOptions {
                    quality: Some(Quality::Medium),
                    max_size: Some(2048),
                    ..JobOptions::default()
                },
            })
            .unwrap()
    }

    fn reason(r: Result<(persimmon_core::watch::WatchSpec, Option<String>), Shown>) -> String {
        r.map(|_| ()).unwrap_err().reason
    }

    #[test]
    fn a_draft_has_the_spec_defaults() {
        let r = draft(&reg(), PathBuf::from("/x/Inbox"), FormatId::Webp);
        assert_eq!(r.results, PathBuf::from("/x/Inbox/converted"));
        assert_eq!(
            r.patterns,
            persimmon_core::watch::Patterns::for_target(&reg(), FormatId::Webp).to_string()
        );
        assert!(r.settle && r.enabled);
        assert!(!r.recursive && !r.notify_each);
        assert_eq!(r.done, None);
        assert_eq!(r.preset, None);
        assert!(!r.id.is_empty());
    }

    #[test]
    fn the_command_is_the_cli_line() {
        let d = Dir::new("command");
        let mut ps = presets(&d);
        let p = mine(&mut ps, FormatId::Webp);
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let mut r = draft(&reg(), home.join("Inbox"), FormatId::Webp);
        r.preset = Some(p.id.clone());
        r.patterns = "*.jpg,*.png".into();
        r.done = Some(home.join("Inbox/done"));
        r.recursive = true;
        assert_eq!(
            command(&r, &ps),
            "prsm watch ~/Inbox --to webp --quality medium --max-size 2048 --match \"*.jpg,*.png\" --out ~/Inbox/converted --then-move ~/Inbox/done --recursive"
        );
        r.settle = false;
        assert!(command(&r, &ps).ends_with(" --no-wait"));
        let spaced = draft(&reg(), home.join("My Inbox"), FormatId::Png);
        assert!(
            command(&spaced, &ps).starts_with("prsm watch ~/'My Inbox' --to png"),
            "{}",
            command(&spaced, &ps)
        );
    }

    #[test]
    fn a_checked_rule_gives_its_spec() {
        let d = Dir::new("ok");
        let ps = presets(&d);
        let r = draft(&reg(), d.folder("in"), FormatId::Webp);
        let (spec, name) = check(&r, &[], &ps, &reg()).unwrap();
        assert_eq!(spec.folder, r.folder);
        assert_eq!(spec.results, r.results);
        assert!(spec.settle);
        assert_eq!(name, None);
        // Checked against itself, a saved rule isn't refused.
        check(&r, std::slice::from_ref(&r), &ps, &reg()).unwrap();
    }

    #[test]
    fn results_in_the_folder_itself_are_refused() {
        let d = Dir::new("results-self");
        let mut r = draft(&reg(), d.folder("in"), FormatId::Webp);
        r.results = r.folder.clone();
        let e = check(&r, &[], &presets(&d), &reg()).unwrap_err();
        assert_eq!(e.reason, "Results would be converted again");
        assert_eq!(e.fix.as_deref(), Some("Choose another folder for results."));
    }

    #[test]
    fn done_in_the_folder_itself_is_refused() {
        let d = Dir::new("done-self");
        let mut r = draft(&reg(), d.folder("in"), FormatId::Webp);
        r.done = Some(r.folder.clone());
        let e = check(&r, &[], &presets(&d), &reg()).unwrap_err();
        assert_eq!(e.reason, "Finished originals would be converted again");
        assert_eq!(
            e.fix.as_deref(),
            Some("Choose another folder for finished originals.")
        );
    }

    #[test]
    fn a_folder_inside_another_rules_is_refused() {
        let d = Dir::new("nested");
        let ps = presets(&d);
        let outer = draft(&reg(), d.folder("in"), FormatId::Webp);
        let inner = draft(&reg(), d.folder("in/sub"), FormatId::Png);
        for (rule, other) in [(&inner, &outer), (&outer, &inner)] {
            let e = check(rule, std::slice::from_ref(other), &ps, &reg()).unwrap_err();
            assert_eq!(e.reason, "Another rule already watches this folder");
        }
        let same = draft(&reg(), outer.folder.clone(), FormatId::Png);
        assert_eq!(
            reason(check(&same, std::slice::from_ref(&outer), &ps, &reg())),
            "Another rule already watches this folder"
        );
    }

    #[test]
    fn results_in_another_rules_folder_are_refused() {
        let d = Dir::new("chain");
        let ps = presets(&d);
        let mut a = draft(&reg(), d.folder("a"), FormatId::Webp);
        let mut b = draft(&reg(), d.folder("b"), FormatId::Png);
        a.results = b.folder.clone();
        b.results = a.folder.join("from-b");
        check(&a, &[], &ps, &reg()).unwrap();
        assert_eq!(
            reason(check(&b, std::slice::from_ref(&a), &ps, &reg())),
            "Another rule watches this folder, so results would be converted again"
        );
        b.results = d.0.join("elsewhere");
        b.done = Some(a.folder.clone());
        assert_eq!(
            reason(check(&b, std::slice::from_ref(&a), &ps, &reg())),
            "Another rule watches this folder, so finished originals would be converted again"
        );
    }

    #[test]
    fn empty_patterns_are_refused() {
        let d = Dir::new("patterns");
        let mut r = draft(&reg(), d.folder("in"), FormatId::Webp);
        r.patterns = " , ".into();
        assert_eq!(
            reason(check(&r, &[], &presets(&d), &reg())),
            "Match files needs at least one pattern"
        );
    }

    #[test]
    fn an_unreadable_folder_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let d = Dir::new("unreadable");
        let folder = d.folder("in");
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads anything: nothing to test then.
        if std::fs::read_dir(&folder).is_ok() {
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }
        let r = draft(&reg(), folder.clone(), FormatId::Webp);
        let e = check(&r, &[], &presets(&d), &reg()).unwrap_err();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(e.reason, "Persimmon can't read this folder");
        assert_eq!(
            e.fix.as_deref(),
            Some("Allow Persimmon in System Settings → Privacy & Security → Files and Folders")
        );
    }

    #[test]
    fn a_missing_folder_is_refused() {
        let d = Dir::new("missing");
        let r = draft(&reg(), d.0.join("nowhere"), FormatId::Webp);
        let e = check(&r, &[], &presets(&d), &reg()).unwrap_err();
        assert_eq!(e.reason, "This folder doesn't exist");
        assert_eq!(e.fix.as_deref(), Some("Choose another folder."));
    }

    #[test]
    fn a_preset_for_another_target_is_refused() {
        let d = Dir::new("preset");
        let mut ps = presets(&d);
        let p = mine(&mut ps, FormatId::Png);
        let mut r = draft(&reg(), d.folder("in"), FormatId::Webp);
        r.preset = Some(p.id);
        assert_eq!(
            reason(check(&r, &[], &ps, &reg())),
            "Small web makes PNG, not WEBP"
        );
    }

    #[test]
    fn spec_needs_no_folder() {
        let d = Dir::new("spec");
        let mut ps = presets(&d);
        let p = mine(&mut ps, FormatId::Webp);
        let mut r = draft(&reg(), d.0.join("nowhere"), FormatId::Webp);
        r.preset = Some(p.id.clone());
        let (s, name) = spec(&r, &ps, &reg());
        assert_eq!(s.folder, r.folder);
        assert_eq!(s.options.max_size, Some(2048));
        assert_eq!(name.as_deref(), Some("Small web"));
        ps.delete(&p.id).unwrap();
        let (s, name) = spec(&r, &ps, &reg());
        assert_eq!(s.options, JobOptions::default());
        assert_eq!(name, None);
    }

    #[test]
    fn rules_survive_a_reopen_and_a_damaged_file_is_kept_aside() {
        let d = Dir::new("store");
        let path = rules_path(&d.0);
        assert_eq!(path, d.0.join("watch").join("rules.json"));
        let mut rules = Rules::open(&path);
        let a = draft(&reg(), d.folder("a"), FormatId::Webp);
        rules.put(a.clone()).unwrap();
        let mut changed = a.clone();
        changed.enabled = false;
        rules.put(changed.clone()).unwrap();
        let again = Rules::open(&path);
        assert_eq!(again.all(), std::slice::from_ref(&changed));
        assert_eq!(again.get(&a.id), Some(&changed));
        let mut again = again;
        again.remove(&a.id).unwrap();
        assert!(Rules::open(&path).all().is_empty());
        std::fs::write(&path, b"{ nope").unwrap();
        assert!(Rules::open(&path).all().is_empty());
        assert_eq!(
            std::fs::read(path.with_extension("json.bad")).unwrap(),
            b"{ nope"
        );
    }

    #[test]
    fn using_preset_finds_the_rule() {
        let d = Dir::new("using");
        let mut rules = Rules::open(&rules_path(&d.0));
        let mut a = draft(&reg(), d.folder("a"), FormatId::Webp);
        a.preset = Some("small-web".into());
        rules.put(a.clone()).unwrap();
        assert_eq!(rules.using_preset("small-web").map(|r| &r.id), Some(&a.id));
        assert!(rules.using_preset("other").is_none());
    }
}

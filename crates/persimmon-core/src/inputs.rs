// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Folder inputs: a folder given to convert or compress stands for the
//! files inside it, and with `recursive`, the files below it too.
//!
//! Inside a folder, entries go in name order (byte-wise): a folder's own
//! files first, then its subfolders' files, subfolder by subfolder. Names
//! starting with `.` (Finder's `.DS_Store` and `._*`, Persimmon's own
//! `.prsm-*` temp files), `Icon\r`, and anything that isn't a regular file
//! or a folder are left out without a word. A symlink to a folder is never
//! followed, so a link back up the tree can't loop; a symlink to a file is
//! a file. A file that can't become the target is *skipped* and counted,
//! never failed. A file that's empty, unreadable or corrupt is kept, so
//! planning reports it as it would any other input. A package (an `.app`,
//! a Photos library, an `.rtfd`: a folder Finder shows as one file) is
//! never entered: inside a walk it's skipped like a file that can't become
//! the target.

use std::path::{Path, PathBuf};

use crate::detect::{detect, unreadable};
use crate::error::{Error, ErrorCode, FixKind};
use crate::format::FormatId;
use crate::registry::{Operation, Registry};

/// What a folder stands for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderFiles {
    /// The files to plan, in order.
    pub files: Vec<PathBuf>,
    /// Files that can't become the target (or, for compress, can't be
    /// compressed): left alone and counted.
    pub skipped: Vec<PathBuf>,
    /// Subfolders that couldn't be read, one error each.
    pub errors: Vec<Error>,
}

/// The files `folder` stands for in `op` to `target` (`None` for compress,
/// and for a convert whose target is chosen per file). An error when the
/// folder can't be read, or when it yields no file to plan.
pub fn expand_folder(
    reg: &Registry,
    op: Operation,
    target: Option<FormatId>,
    folder: &Path,
    recursive: bool,
) -> Result<FolderFiles, Error> {
    let mut out = FolderFiles::default();
    let mut seen_file = false;
    let mut seen_folder = false;
    // Folders still to list, last first: popping keeps name order.
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match list(&dir) {
            Ok(entries) => entries,
            Err(e) if dir == folder => return Err(unreadable(folder, &e)),
            Err(e) => {
                out.errors.push(unreadable(&dir, &e));
                continue;
            }
        };
        let mut subfolders = Vec::new();
        for path in entries {
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() && is_package(&path) {
                // Finder shows a package as one file: never write inside it.
                seen_file = true;
                out.skipped.push(path);
                continue;
            }
            if meta.is_dir() {
                seen_folder = true;
                if recursive {
                    subfolders.push(path);
                }
                continue;
            }
            let is_file = meta.is_file()
                || (meta.file_type().is_symlink()
                    && std::fs::metadata(&path).is_ok_and(|m| m.is_file()));
            if !is_file {
                continue;
            }
            seen_file = true;
            if usable(reg, op, target, &path) {
                out.files.push(path);
            } else {
                out.skipped.push(path);
            }
        }
        stack.extend(subfolders.into_iter().rev());
    }
    if out.files.is_empty() && out.errors.is_empty() {
        return Err(nothing(
            reg,
            op,
            target,
            folder,
            seen_file,
            seen_folder,
            recursive,
        ));
    }
    Ok(out)
}

/// Folder extensions macOS treats as a single document or app (a
/// package): a folder walk never goes inside one, and a package is never
/// expanded, since converting inside it would write into an app bundle or
/// a library such as Photos'.
const PACKAGES: &[&str] = &[
    "app",
    "appex",
    "bundle",
    "component",
    "dsym",
    "fcpbundle",
    "framework",
    "imovielibrary",
    "kext",
    "key",
    "logicx",
    "mpkg",
    "musiclibrary",
    "numbers",
    "pages",
    "photolibrary",
    "photoslibrary",
    "pkg",
    "playground",
    "plugin",
    "prefpane",
    "qlgenerator",
    "rtfd",
    "saver",
    "scptd",
    "sparsebundle",
    "theater",
    "tvlibrary",
    "wdgt",
    "xcodeproj",
    "xcworkspace",
    "xpc",
];

/// Whether `path` is a folder macOS shows as one file (by its extension).
pub fn is_package(path: &Path) -> bool {
    path.is_dir()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| PACKAGES.contains(&e.to_ascii_lowercase().as_str()))
}

/// A folder's entries worth looking at, sorted by name.
fn list(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut names: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name())
        .filter(|n| {
            let n = n.as_encoded_bytes();
            !n.starts_with(b".") && n != b"Icon\r"
        })
        .collect();
    names.sort();
    Ok(names.into_iter().map(|n| dir.join(n)).collect())
}

/// Whether a file goes to planning: anything with a route to the target,
/// and anything detection fails on for a reason other than its format.
fn usable(reg: &Registry, op: Operation, target: Option<FormatId>, path: &Path) -> bool {
    match detect(path, reg.formats()) {
        Ok(d) => match (op, target) {
            (Operation::Convert, None) => !reg.targets(Operation::Convert, d.format).is_empty(),
            _ => reg.find(op, d.format, target).is_some(),
        },
        Err(e) => e.code != ErrorCode::UnsupportedInput,
    }
}

/// The error for a folder that yields no file to plan.
fn nothing(
    reg: &Registry,
    op: Operation,
    target: Option<FormatId>,
    folder: &Path,
    seen_file: bool,
    seen_folder: bool,
    recursive: bool,
) -> Error {
    let e = |reason: String| Error::new(ErrorCode::UnsupportedInput, reason).with_file(folder);
    if !seen_file && !recursive && seen_folder {
        return e("it has no files, only folders".into()).with_fix_as(
            FixKind::IncludeSubfolders,
            "add --recursive to include its subfolders",
        );
    }
    if !seen_file {
        return e("the folder is empty".into());
    }
    let what = match (op, target) {
        (Operation::Compress, _) => "be compressed".to_string(),
        (_, Some(to)) => format!("become {}", reg.format(to).name),
        (_, None) => "be converted".to_string(),
    };
    e(format!("none of its files can {what}")).with_fix_as(
        FixKind::SeeFormats,
        "run prsm formats to see what each format can become",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{ErrorCode, FixKind};
    use crate::format::FormatId;
    use crate::registry::{Operation, Registry};
    use persimmon_corpus::{GIF_1X1, PNG_1X1, ZIP_EMPTY};
    use std::path::{Path, PathBuf};

    const WEBP_HEAD: &[u8] = b"RIFF\x1a\0\0\0WEBPVP8 \x0e\0\0\0";

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("prsm-inputs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, bytes).unwrap();
        p
    }

    fn webp(dir: &Path, recursive: bool) -> Result<FolderFiles, Error> {
        expand_folder(
            &Registry::builtin(),
            Operation::Convert,
            Some(FormatId::Webp),
            dir,
            recursive,
        )
    }

    #[test]
    fn folder_stands_for_the_files_directly_inside() {
        let dir = scratch("direct");
        put(&dir, "b.gif", GIF_1X1);
        put(&dir, "a.png", &PNG_1X1);
        put(&dir, "sub/c.png", &PNG_1X1);
        let ff = webp(&dir, false).unwrap();
        assert_eq!(ff.files, vec![dir.join("a.png"), dir.join("b.gif")]);
        assert!(ff.skipped.is_empty() && ff.errors.is_empty());
    }

    #[test]
    fn recursive_walks_subfolders_files_first() {
        let dir = scratch("recursive");
        put(&dir, "a.png", &PNG_1X1);
        put(&dir, "sub/c.png", &PNG_1X1);
        put(&dir, "b.gif", GIF_1X1);
        put(&dir, "sub/deeper/d.png", &PNG_1X1);
        let ff = webp(&dir, true).unwrap();
        assert_eq!(
            ff.files,
            vec![
                dir.join("a.png"),
                dir.join("b.gif"),
                dir.join("sub/c.png"),
                dir.join("sub/deeper/d.png")
            ]
        );
    }

    #[test]
    fn hidden_litter_and_temp_files_are_left_out_silently() {
        let dir = scratch("litter");
        put(&dir, "a.png", &PNG_1X1);
        put(&dir, ".DS_Store", b"litter");
        put(&dir, "._a.png", b"litter");
        put(&dir, ".prsm-x.part", &PNG_1X1);
        put(&dir, ".hidden.png", &PNG_1X1);
        put(&dir, "Icon\r", b"");
        let ff = webp(&dir, true).unwrap();
        assert_eq!(ff.files, vec![dir.join("a.png")]);
        assert!(ff.skipped.is_empty());
    }

    #[test]
    fn files_that_cant_become_the_target_are_skipped() {
        let dir = scratch("skip");
        put(&dir, "a.png", &PNG_1X1);
        put(&dir, "notes.txt", b"plain text, nothing to convert");
        put(&dir, "x.webp", WEBP_HEAD);
        let ff = webp(&dir, false).unwrap();
        assert_eq!(ff.files, vec![dir.join("a.png")]);
        assert_eq!(ff.skipped, vec![dir.join("notes.txt"), dir.join("x.webp")]);
    }

    #[test]
    fn compress_skips_what_it_cant_compress() {
        let dir = scratch("compress");
        put(&dir, "a.png", &PNG_1X1);
        put(&dir, "notes.txt", b"plain text");
        put(&dir, "archive.zip", &ZIP_EMPTY);
        let ff =
            expand_folder(&Registry::builtin(), Operation::Compress, None, &dir, false).unwrap();
        assert_eq!(ff.files, vec![dir.join("a.png")]);
        assert_eq!(ff.skipped.len(), 2);
    }

    #[test]
    fn convert_without_a_target_skips_only_what_can_become_nothing() {
        let dir = scratch("any");
        put(&dir, "a.png", &PNG_1X1);
        put(&dir, "notes.txt", b"plain text");
        let ff =
            expand_folder(&Registry::builtin(), Operation::Convert, None, &dir, false).unwrap();
        assert_eq!(ff.files, vec![dir.join("a.png")]);
        assert_eq!(ff.skipped, vec![dir.join("notes.txt")]);
    }

    #[test]
    fn broken_files_are_kept_for_planning_to_report() {
        let dir = scratch("broken");
        put(&dir, "a.png", &PNG_1X1);
        put(&dir, "empty.png", b"");
        let ff = webp(&dir, false).unwrap();
        assert!(ff.files.contains(&dir.join("empty.png")), "{ff:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_folder_is_not_followed() {
        let dir = scratch("symlink");
        let a = put(&dir, "a.png", &PNG_1X1);
        std::os::unix::fs::symlink(&dir, dir.join("loop")).unwrap();
        std::os::unix::fs::symlink(&a, dir.join("link.png")).unwrap();
        let ff = webp(&dir, true).unwrap();
        assert_eq!(ff.files, vec![dir.join("a.png"), dir.join("link.png")]);
    }

    #[test]
    fn an_empty_folder_is_an_error() {
        let dir = scratch("empty");
        put(&dir, ".DS_Store", b"litter");
        let e = webp(&dir, true).unwrap_err();
        assert_eq!(
            (e.code, e.reason.as_str(), e.fix.is_none()),
            (ErrorCode::UnsupportedInput, "the folder is empty", true)
        );
        assert_eq!(e.file.as_deref(), Some(dir.as_path()));
    }

    #[test]
    fn only_subfolders_without_recursive_names_recursive() {
        let dir = scratch("only-sub");
        put(&dir, "sub/a.png", &PNG_1X1);
        let e = webp(&dir, false).unwrap_err();
        assert_eq!(e.reason, "it has no files, only folders");
        assert_eq!(
            e.fix.as_ref().map(|f| (&f.kind, f.text.as_str())),
            Some((
                &FixKind::IncludeSubfolders,
                "add --recursive to include its subfolders"
            ))
        );
    }

    #[test]
    fn nothing_usable_is_an_error_naming_the_target() {
        let dir = scratch("nothing");
        put(&dir, "notes.txt", b"plain text");
        let reg = Registry::builtin();
        let err = |op, to| expand_folder(&reg, op, to, &dir, false).unwrap_err();
        for (e, reason) in [
            (
                err(Operation::Convert, Some(FormatId::Webp)),
                "none of its files can become WebP",
            ),
            (
                err(Operation::Compress, None),
                "none of its files can be compressed",
            ),
            (
                err(Operation::Convert, None),
                "none of its files can be converted",
            ),
        ] {
            assert_eq!(e.reason, reason);
            assert_eq!(e.code, ErrorCode::UnsupportedInput);
            assert_eq!(
                e.fix.as_ref().map(|f| (&f.kind, f.text.as_str())),
                Some((
                    &FixKind::SeeFormats,
                    "run prsm formats to see what each format can become"
                ))
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_subfolder_is_reported_and_the_rest_expand() {
        use std::os::unix::fs::PermissionsExt;
        let dir = scratch("unreadable");
        put(&dir, "a.png", &PNG_1X1);
        put(&dir, "locked/b.png", &PNG_1X1);
        let locked = dir.join("locked");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read_dir(&locked).is_ok() {
            // Running as root: permissions don't apply.
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }
        let ff = webp(&dir, true);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let ff = ff.unwrap();
        assert_eq!(ff.files, vec![dir.join("a.png")]);
        assert_eq!(ff.errors.len(), 1);
        assert_eq!(ff.errors[0].file.as_deref(), Some(locked.as_path()));
    }

    #[test]
    fn a_package_is_never_entered() {
        let dir = scratch("package");
        put(&dir, "a.png", &PNG_1X1);
        put(&dir, "Foo.app/Contents/Resources/b.png", &PNG_1X1);
        put(&dir, "x.photoslibrary/originals/c.png", &PNG_1X1);
        put(&dir, "Notes.rtfd/d.png", &PNG_1X1);
        let ff = webp(&dir, true).unwrap();
        assert_eq!(ff.files, vec![dir.join("a.png")]);
        assert_eq!(
            ff.skipped,
            vec![
                dir.join("Foo.app"),
                dir.join("Notes.rtfd"),
                dir.join("x.photoslibrary")
            ]
        );
        assert!(is_package(&dir.join("Foo.app")) && !is_package(&dir));
    }

    #[test]
    fn expands_a_large_deep_tree() {
        let dir = scratch("large");
        let mut level = dir.clone();
        for depth in 0..20 {
            for i in 0..100 {
                put(&level, &format!("p{depth}-{i}.png"), &PNG_1X1);
            }
            level = level.join(format!("d{depth}"));
        }
        let ff = webp(&dir, true).unwrap();
        assert_eq!(ff.files.len(), 2_000);
    }
}

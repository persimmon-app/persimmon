// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! The extraction sink: every safety rule for writing an archive's
//! entries, in one place (DD-37).
//!
//! - **Inside the folder or nothing.** A name that's absolute, has a drive
//!   letter or a `..`, or that the platform can't hold, fails the whole
//!   extraction.
//! - **Never through a link.** An entry whose path passes through a symlink
//!   extracted earlier fails the same way, so `link -> /etc` followed by
//!   `link/passwd` writes nothing.
//! - **Hard links are copies** of a file already written.
//! - **Clashes:** an exact repeat replaces the earlier file, as tar does.
//!   Files whose names differ only by case or Unicode normalisation are
//!   numbered (`a (1).txt`), since APFS and NTFS would merge them; folders
//!   that differ that way merge into the first spelling, as they would on
//!   those volumes anyway.
//! - **`__MACOSX` sidecars are skipped:** they carry resource forks and
//!   extended attributes, which prsm doesn't keep.
//! - **Attributes:** modification times and the executable bit (0755 or
//!   0644); never owners, set-uid, set-gid or sticky bits.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use unicode_normalization::UnicodeNormalization;

use crate::error::{Error, ErrorCode};
use crate::exec::CancelToken;
use crate::platform;

/// How much is written between cancel checks.
const CHUNK: usize = 1 << 20;

/// `name` as a relative path that stays inside the output folder: `/` and
/// `\` both separate, empty and `.` parts are dropped. `None` for any `..`
/// (even one that would stay inside), a leading separator, a drive prefix,
/// a NUL, a name the platform can't hold, or nothing left.
pub(crate) fn safe_path(name: &str) -> Option<PathBuf> {
    if name.contains('\0') || name.starts_with(['/', '\\']) {
        return None;
    }
    let b = name.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return None;
    }
    let mut out = PathBuf::new();
    for part in name.split(['/', '\\']) {
        match part {
            "" | "." => {}
            ".." => return None,
            p if platform::file_name_ok(p) => out.push(p),
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// Whether `name` names the output folder itself: only `.` parts and
/// separators, like the `./` entry `tar -C dir .` writes first. A leading
/// separator stays unsafe.
fn is_root(name: &str) -> bool {
    !name.starts_with(['/', '\\']) && name.split(['/', '\\']).all(|p| p.is_empty() || p == ".")
}

/// Whether `name` is in a `__MACOSX` folder: the resource-fork and
/// extended-attribute sidecars (`._*`) that Finder and `ditto` add to
/// ZIPs. Archive Utility hides them; prsm keeps no extended attributes,
/// so they're skipped rather than left as clutter.
fn sidecar(name: &str) -> bool {
    name.split(['/', '\\'])
        .find(|p| !p.is_empty() && *p != ".")
        .is_some_and(|first| first == "__MACOSX")
}

/// Names compare the way APFS and NTFS do: ignoring case and Unicode
/// normalisation.
fn fold(name: &str) -> String {
    name.nfc().collect::<String>().to_lowercase()
}

/// `name` with ` (n)` before its extension: `a (1).txt`, `.env (1)`.
fn numbered(name: &str, n: u32) -> String {
    match name.rfind('.') {
        Some(dot) if dot > 0 => format!("{} ({n}){}", &name[..dot], &name[dot..]),
        _ => format!("{name} ({n})"),
    }
}

fn time(secs: i64) -> SystemTime {
    if secs >= 0 {
        UNIX_EPOCH + Duration::from_secs(secs as u64)
    } else {
        UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Dir,
    Leaf,
}

/// Writes one archive's entries under `root`.
pub(crate) struct Sink {
    root: PathBuf,
    archive: PathBuf,
    /// Per folder (relative, as written): folded name → the name written.
    names: HashMap<(PathBuf, String), String>,
    /// The archive's own path of each file written → where it went, for
    /// hard links.
    written: HashMap<PathBuf, PathBuf>,
    /// Folder modification times, applied by `finish`.
    dir_times: Vec<(PathBuf, i64)>,
    /// Names chosen by numbering, per folder (folded), so a real entry of
    /// the same name is numbered in turn instead of replacing it.
    numbered: std::collections::HashSet<(PathBuf, String)>,
    /// The archive's own path of each symlink recreated, so a hard link to
    /// one can say so.
    links: std::collections::HashSet<PathBuf>,
}

impl Sink {
    /// A sink writing under `root`, created as needed. Errors name
    /// `archive`.
    pub(crate) fn new(root: PathBuf, archive: PathBuf) -> Sink {
        Sink {
            root,
            archive,
            names: HashMap::new(),
            written: HashMap::new(),
            dir_times: Vec::new(),
            numbered: std::collections::HashSet::new(),
            links: std::collections::HashSet::new(),
        }
    }

    fn unsafe_path(&self, name: &str) -> Error {
        Error::new(
            ErrorCode::CorruptInput,
            format!("it has an unsafe path: {name}"),
        )
        .with_file(&self.archive)
    }

    /// A write that failed. A name the file system can't hold is the
    /// archive's problem, so it gets no `--out` fix.
    fn not_writable(&self, name: &str, e: &std::io::Error) -> Error {
        if e.kind() == std::io::ErrorKind::InvalidFilename {
            return Error::new(
                ErrorCode::CorruptInput,
                format!("it has a name this system can't hold: {name}"),
            )
            .with_file(&self.archive);
        }
        Error::new(
            ErrorCode::OutputNotWritable,
            format!("couldn't write {name} ({e})"),
        )
        .with_file(&self.archive)
        .with_fix("choose another folder with --out <dir>")
    }

    fn conflict(&self, name: &str) -> Error {
        Error::new(
            ErrorCode::CorruptInput,
            format!("it has conflicting entries: {name}"),
        )
        .with_file(&self.archive)
    }

    /// Where entry `name` goes, relative to `root`, with its parent folders
    /// created. Applies the path, link and clash rules.
    fn place(&mut self, name: &str, kind: Kind) -> Result<(PathBuf, PathBuf), Error> {
        let rel = safe_path(name).ok_or_else(|| self.unsafe_path(name))?;
        let parts: Vec<String> = rel
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let mut actual = PathBuf::new();
        for (i, part) in parts.iter().enumerate() {
            let leaf = i + 1 == parts.len();
            let key = (actual.clone(), fold(part));
            let chosen = match self.names.get(&key).cloned() {
                // Folders merge into the first spelling; so do repeats, unless
                // the name was one a clash was numbered to.
                Some(existing)
                    if !leaf
                        || kind == Kind::Dir
                        || (existing == *part && !self.numbered.contains(&key)) =>
                {
                    existing
                }
                Some(_) => {
                    let mut n = 1;
                    while self
                        .names
                        .contains_key(&(actual.clone(), fold(&numbered(part, n))))
                    {
                        n += 1;
                    }
                    let name = numbered(part, n);
                    self.names
                        .insert((actual.clone(), fold(&name)), name.clone());
                    self.numbered.insert((actual.clone(), fold(&name)));
                    name
                }
                None => {
                    self.names.insert(key, part.clone());
                    part.clone()
                }
            };
            actual.push(&chosen);
            if !leaf || kind == Kind::Dir {
                let path = self.root.join(&actual);
                match std::fs::symlink_metadata(&path) {
                    Ok(m) if m.file_type().is_symlink() => return Err(self.unsafe_path(name)),
                    Ok(m) if !m.is_dir() => return Err(self.conflict(name)),
                    Ok(_) => {}
                    Err(_) => {
                        std::fs::create_dir_all(&path).map_err(|e| self.not_writable(name, &e))?
                    }
                }
            }
        }
        Ok((rel, actual))
    }

    /// Make room at a leaf: a symlink or file there is replaced; a folder
    /// is a conflict.
    fn clear_leaf(&self, name: &str, path: &Path) -> Result<(), Error> {
        match std::fs::symlink_metadata(path) {
            Ok(m) if m.is_dir() => Err(self.conflict(name)),
            Ok(_) => std::fs::remove_file(path).map_err(|e| self.not_writable(name, &e)),
            Err(_) => Ok(()),
        }
    }

    pub(crate) fn dir(&mut self, name: &str, mtime: Option<i64>) -> Result<(), Error> {
        // `./` (from `tar -C dir .`) is the output folder itself.
        if is_root(name) || sidecar(name) {
            return Ok(());
        }
        std::fs::create_dir_all(&self.root).map_err(|e| self.not_writable(name, &e))?;
        let (_, actual) = self.place(name, Kind::Dir)?;
        if let Some(t) = mtime {
            self.dir_times.push((actual, t));
        }
        Ok(())
    }

    /// Write a file from `data`, checking `cancel` after every 1 MiB.
    pub(crate) fn file(
        &mut self,
        name: &str,
        mode: Option<u32>,
        mtime: Option<i64>,
        data: &mut dyn Read,
        cancel: &CancelToken,
    ) -> Result<(), Error> {
        if sidecar(name) {
            return Ok(());
        }
        std::fs::create_dir_all(&self.root).map_err(|e| self.not_writable(name, &e))?;
        let (rel, actual) = self.place(name, Kind::Leaf)?;
        let path = self.root.join(&actual);
        self.clear_leaf(name, &path)?;
        let mut out = File::create(&path).map_err(|e| self.not_writable(name, &e))?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            let mut filled = 0;
            while filled < CHUNK {
                match data.read(&mut buf[filled..]) {
                    Ok(0) => break,
                    Ok(n) => filled += n,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => {
                        return Err(Error::new(
                            ErrorCode::CorruptInput,
                            format!("can't read this archive ({e})"),
                        )
                        .with_file(&self.archive));
                    }
                }
            }
            out.write_all(&buf[..filled])
                .map_err(|e| self.not_writable(name, &e))?;
            if cancel.is_cancelled() {
                return Err(Error::new(ErrorCode::Cancelled, "cancelled").with_file(&self.archive));
            }
            if filled < CHUNK {
                break;
            }
        }
        if let Some(t) = mtime {
            out.set_modified(time(t))
                .map_err(|e| self.not_writable(name, &e))?;
        }
        drop(out);
        if let Some(mode) = mode {
            platform::set_mode(&path, mode & 0o111 != 0)
                .map_err(|e| self.not_writable(name, &e))?;
        }
        self.written.insert(rel, actual);
        Ok(())
    }

    /// Recreate a symlink with its target as stored. A platform that
    /// refuses symlinks (Windows without Developer Mode) skips it.
    pub(crate) fn symlink(&mut self, name: &str, target: &str) -> Result<(), Error> {
        if sidecar(name) {
            return Ok(());
        }
        std::fs::create_dir_all(&self.root).map_err(|e| self.not_writable(name, &e))?;
        let (rel, actual) = self.place(name, Kind::Leaf)?;
        let path = self.root.join(&actual);
        self.clear_leaf(name, &path)?;
        self.links.insert(rel);
        // A file replaced by a link is no longer a file a hard link may
        // copy: copying would follow the link outside the folder.
        self.written.retain(|_, written| *written != actual);
        platform::symlink(Path::new(target), &path).map_err(|e| self.not_writable(name, &e))?;
        Ok(())
    }

    /// A hard link to `target`, an entry written earlier, as a copy.
    pub(crate) fn hardlink(&mut self, name: &str, target: &str) -> Result<(), Error> {
        if sidecar(name) {
            return Ok(());
        }
        let source = safe_path(target).ok_or_else(|| self.unsafe_path(target))?;
        let missing = || {
            Error::new(
                ErrorCode::CorruptInput,
                format!("it links to a file that isn't in it: {target}"),
            )
            .with_file(&self.archive)
        };
        if self.links.contains(&source) && !self.written.contains_key(&source) {
            return Err(Error::new(
                ErrorCode::CorruptInput,
                format!("it has a hard link to a link: {target}"),
            )
            .with_file(&self.archive));
        }
        let from = self.written.get(&source).cloned().ok_or_else(missing)?;
        // Only ever copy a regular file the sink wrote, never through a link.
        if !std::fs::symlink_metadata(self.root.join(&from)).is_ok_and(|m| m.is_file()) {
            return Err(missing());
        }
        let (rel, actual) = self.place(name, Kind::Leaf)?;
        let path = self.root.join(&actual);
        self.clear_leaf(name, &path)?;
        std::fs::copy(self.root.join(&from), &path).map_err(|e| self.not_writable(name, &e))?;
        self.written.insert(rel, actual);
        Ok(())
    }

    /// Apply folder times (writing files changed them), then lift the
    /// contents up when the only top-level item is a folder named `stem`.
    pub(crate) fn finish(self, stem: &str) -> Result<(), Error> {
        std::fs::create_dir_all(&self.root).map_err(|e| self.not_writable(stem, &e))?;
        for (dir, t) in self.dir_times.iter().rev() {
            // Best effort: a folder's time is a nicety, never a failure.
            let _ = platform::set_dir_modified(&self.root.join(dir), time(*t));
        }
        let entries: Vec<_> = std::fs::read_dir(&self.root)
            .map_err(|e| self.not_writable(stem, &e))?
            .filter_map(Result::ok)
            .collect();
        if let [only] = entries.as_slice()
            && only.file_type().is_ok_and(|t| t.is_dir())
            && fold(&only.file_name().to_string_lossy()) == fold(stem)
        {
            let mut lifted = self.root.clone().into_os_string();
            lifted.push(".lift");
            let lifted = PathBuf::from(lifted);
            let fail = |e: std::io::Error| self.not_writable(stem, &e);
            std::fs::rename(only.path(), &lifted).map_err(fail)?;
            std::fs::remove_dir(&self.root).map_err(fail)?;
            std::fs::rename(&lifted, &self.root).map_err(fail)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::exec::CancelToken;
    use std::path::{Path, PathBuf};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("prsm-sink-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn sink(dir: &Path) -> Sink {
        Sink::new(dir.join("out"), dir.join("a.zip"))
    }

    fn put(s: &mut Sink, name: &str, data: &[u8]) -> Result<(), Error> {
        s.file(name, None, None, &mut &data[..], &CancelToken::new())
    }

    fn read(dir: &Path, rel: &str) -> String {
        std::fs::read_to_string(dir.join("out").join(rel)).unwrap()
    }

    #[test]
    fn safe_path_keeps_names_inside() {
        let p = |s: &str| safe_path(s).map(|p| p.to_string_lossy().replace('\\', "/"));
        assert_eq!(p("a/b.txt").as_deref(), Some("a/b.txt"));
        assert_eq!(p("./a//b").as_deref(), Some("a/b"));
        assert_eq!(p("a\\b").as_deref(), Some("a/b"));
        for bad in [
            "../x",
            "a/../../x",
            "/etc/x",
            "C:/x",
            "C:x",
            "a\0b",
            "",
            ".",
        ] {
            assert_eq!(safe_path(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn files_dirs_and_modes() {
        let dir = scratch("modes");
        let mut s = sink(&dir);
        s.dir("d", Some(1_767_323_045)).unwrap();
        s.file(
            "d/run.sh",
            Some(0o755),
            Some(1_767_323_045),
            &mut &b"x"[..],
            &CancelToken::new(),
        )
        .unwrap();
        s.file(
            "d/plain.txt",
            Some(0o644),
            None,
            &mut &b"y"[..],
            &CancelToken::new(),
        )
        .unwrap();
        s.finish("other").unwrap();
        let run = dir.join("out/d/run.sh");
        let mtime = |p: &Path| {
            std::fs::metadata(p)
                .unwrap()
                .modified()
                .unwrap()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        };
        assert_eq!(mtime(&run), 1_767_323_045);
        assert_eq!(mtime(&dir.join("out/d")), 1_767_323_045);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&run), 0o755);
            assert_eq!(mode(&dir.join("out/d/plain.txt")), 0o644);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn exact_duplicate_replaces() {
        let dir = scratch("dup");
        let mut s = sink(&dir);
        put(&mut s, "x.txt", b"first").unwrap();
        put(&mut s, "x.txt", b"second").unwrap();
        s.finish("other").unwrap();
        assert_eq!(read(&dir, "x.txt"), "second");
        assert_eq!(std::fs::read_dir(dir.join("out")).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn case_clash_keeps_both() {
        let dir = scratch("case");
        let mut s = sink(&dir);
        put(&mut s, "A.txt", b"upper").unwrap();
        put(&mut s, "a.txt", b"lower").unwrap();
        s.finish("other").unwrap();
        assert_eq!(read(&dir, "A.txt"), "upper");
        assert_eq!(read(&dir, "a (1).txt"), "lower");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nfd_and_nfc_clash_keeps_both() {
        let dir = scratch("nfd");
        let mut s = sink(&dir);
        put(&mut s, "caf\u{e9}.txt", b"nfc").unwrap();
        put(&mut s, "cafe\u{301}.txt", b"nfd").unwrap();
        s.finish("other").unwrap();
        assert_eq!(read(&dir, "caf\u{e9}.txt"), "nfc");
        assert_eq!(read(&dir, "cafe\u{301} (1).txt"), "nfd");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn writes_never_go_through_a_symlink() {
        let dir = scratch("through");
        let target = std::env::temp_dir();
        let escaped = target.join(format!("prsm-sink-x-{}", std::process::id()));
        let mut s = sink(&dir);
        s.symlink("link", target.to_str().unwrap()).unwrap();
        let name = format!("link/{}", escaped.file_name().unwrap().to_str().unwrap());
        let e = put(&mut s, &name, b"evil").unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput);
        assert_eq!(e.reason, format!("it has an unsafe path: {name}"));
        assert_eq!(e.file.as_deref(), Some(dir.join("a.zip").as_path()));
        assert!(!escaped.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A name too long for the file system is the archive's problem, not
    /// the output folder's: no `--out` fix.
    #[test]
    fn overlong_name_is_corrupt_input() {
        let dir = scratch("overlong");
        let mut s = sink(&dir);
        let long = format!("{}.txt", "a".repeat(296));
        let e = put(&mut s, &long, b"x").unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput, "{}", e.reason);
        assert_eq!(
            e.reason,
            format!("it has a name this system can't hold: {long}")
        );
        assert_eq!(e.fix, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn hardlink_to_a_link_says_so() {
        let dir = scratch("hard-to-link");
        let mut s = sink(&dir);
        s.symlink("l", "a.txt").unwrap();
        let e = s.hardlink("b.txt", "l").unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput);
        assert_eq!(e.reason, "it has a hard link to a link: l");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A real entry whose name a clash already took gets its own number:
    /// nothing is overwritten.
    #[test]
    fn real_name_after_numbered_clash_is_numbered_again() {
        let dir = scratch("renumber");
        let mut s = sink(&dir);
        put(&mut s, "A.txt", b"upper").unwrap();
        put(&mut s, "a.txt", b"lower").unwrap();
        put(&mut s, "a (1).txt", b"real").unwrap();
        s.finish("other").unwrap();
        assert_eq!(read(&dir, "A.txt"), "upper");
        assert_eq!(read(&dir, "a (1).txt"), "lower");
        // Numbered by its own name, as every clash is.
        assert_eq!(read(&dir, "a (1) (1).txt"), "real");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hardlink_copies_an_earlier_file() {
        let dir = scratch("hard");
        let mut s = sink(&dir);
        put(&mut s, "a.txt", b"shared").unwrap();
        s.hardlink("b.txt", "a.txt").unwrap();
        s.finish("other").unwrap();
        assert_eq!(read(&dir, "b.txt"), "shared");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn hardlink_outside_is_unsafe() {
        let dir = scratch("hard-out");
        let mut s = sink(&dir);
        let e = s.hardlink("b.txt", "/etc/passwd").unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput);
        assert_eq!(e.reason, "it has an unsafe path: /etc/passwd");
        let e = s.hardlink("c.txt", "missing.txt").unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput);
        assert!(!dir.join("out/b.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unwrap_same_name_folder() {
        let dir = scratch("unwrap");
        let mut s = sink(&dir);
        s.dir("photos", None).unwrap();
        put(&mut s, "photos/a.txt", b"a").unwrap();
        s.finish("Photos").unwrap();
        assert_eq!(read(&dir, "a.txt"), "a");
        assert!(!dir.join("out/photos").exists());

        let other = scratch("no-unwrap");
        let mut s = sink(&other);
        put(&mut s, "project/a.txt", b"a").unwrap();
        s.finish("photos").unwrap();
        assert_eq!(read(&other, "project/a.txt"), "a");
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn cancel_mid_file() {
        let dir = scratch("cancel");
        let mut s = sink(&dir);
        let cancel = CancelToken::new();
        cancel.cancel();
        let data = vec![0u8; 3 << 20];
        let e = s
            .file("big.bin", None, None, &mut &data[..], &cancel)
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::Cancelled);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn macosx_sidecars_are_skipped() {
        let dir = scratch("macosx");
        let mut s = sink(&dir);
        put(&mut s, "notes/a.txt", b"a").unwrap();
        s.dir("__MACOSX", None).unwrap();
        s.dir("__MACOSX/notes", None).unwrap();
        put(&mut s, "__MACOSX/notes/._a.txt", b"sidecar").unwrap();
        s.symlink("__MACOSX/link", "a.txt").unwrap();
        s.finish("other").unwrap();
        assert!(dir.join("out/notes/a.txt").is_file());
        assert!(!dir.join("out/__MACOSX").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn symlink_swapped_in_never_feeds_a_hardlink() {
        let dir = scratch("swap");
        std::fs::create_dir_all(&dir).unwrap();
        let secret = dir.join("secret.txt");
        std::fs::write(&secret, "secret").unwrap();
        let mut s = sink(&dir);
        put(&mut s, "a.txt", b"plain").unwrap();
        s.symlink("a.txt", secret.to_str().unwrap()).unwrap();
        let e = s.hardlink("b.txt", "a.txt").unwrap_err();
        assert_eq!(e.code, ErrorCode::CorruptInput, "{e}");
        assert!(
            std::fs::read(dir.join("out/b.txt")).map_or(true, |b| b != b"secret"),
            "the outside file was copied in"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dot_entries_are_the_folder_itself() {
        let dir = scratch("dot");
        let mut s = sink(&dir);
        s.dir("./", None).unwrap();
        s.dir(".", None).unwrap();
        put(&mut s, "./t.txt", b"t").unwrap();
        s.finish("other").unwrap();
        assert_eq!(read(&dir, "t.txt"), "t");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Copying pages out of existing PDFs into a new one, for merge and split
//! (DD-36).
//!
//! The trap is that a page points at much else: its parent page tree, and
//! through links, outline entries and form fields, at other pages. Copied
//! naively, one kept page keeps the whole source alive. So before copying,
//! every reference to the source's catalog, page tree, outline root and
//! unkept pages becomes `null`. Whatever is then unreachable is pruned by
//! the caller. Links into dropped pages are left inert; bookmarks, the
//! form field tree, page labels and the structure tree are not carried.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use hayro::hayro_syntax::{DecryptionError, LoadPdfError};
use lopdf::{Document, Object, ObjectId};

use super::load_error;
use crate::error::{Error, ErrorCode};

/// Page attributes a page may inherit from its page tree.
const INHERITED: [&[u8]; 4] = [b"MediaBox", b"CropBox", b"Resources", b"Rotate"];

/// Load `path` with lopdf's default limits, which cap how far any stream
/// may decompress. A PDF that still needs a password is an error.
pub(crate) fn load_pdf(path: &Path) -> Result<Document, Error> {
    let doc = Document::load(path).map_err(|e| {
        Error::new(
            ErrorCode::CorruptInput,
            format!("can't read this PDF ({e})"),
        )
        .with_file(path)
    })?;
    if doc.is_encrypted() {
        return Err(
            load_error(LoadPdfError::Decryption(DecryptionError::PasswordProtected))
                .with_file(path),
        );
    }
    Ok(doc)
}

/// What [`append_pages`] added.
pub(crate) struct Appended {
    /// The kept pages' new ids, in document order.
    pub pages: Vec<ObjectId>,
    /// The source's document info, renumbered, if it had one.
    pub info: Option<ObjectId>,
}

/// Move `pages` (1-based) of `source` into `target`, renumbered past
/// `target`'s highest id. Each kept page carries its inherited attributes
/// itself, and its `Parent` is `null` until the caller sets it.
pub(crate) fn append_pages(
    target: &mut Document,
    mut source: Document,
    pages: &[u32],
) -> Result<Appended, Error> {
    let all = source.get_pages();
    let kept: Vec<ObjectId> = pages
        .iter()
        .map(|n| {
            all.get(n)
                .copied()
                .ok_or_else(|| Error::new(ErrorCode::CorruptInput, format!("page {n} is missing")))
        })
        .collect::<Result<_, _>>()?;

    for &page in &kept {
        for key in INHERITED {
            if let Some(value) = inherited(&source, page, key)
                && let Ok(dict) = source.get_dictionary_mut(page)
            {
                dict.set(key, value);
            }
        }
    }

    let keep: HashSet<ObjectId> = kept.iter().copied().collect();
    let mut dropped: HashSet<ObjectId> = HashSet::new();
    for &page in all.values() {
        if !keep.contains(&page) {
            dropped.insert(page);
        }
        let mut node = page;
        while let Some(parent) = source
            .get_dictionary(node)
            .ok()
            .and_then(|d| d.get(b"Parent").ok())
            .and_then(|p| p.as_reference().ok())
        {
            if !dropped.insert(parent) {
                break;
            }
            node = parent;
        }
    }
    if let Ok(root) = source.trailer.get(b"Root").and_then(Object::as_reference) {
        dropped.insert(root);
        if let Ok(outlines) = source
            .get_dictionary(root)
            .and_then(|c| c.get(b"Outlines"))
            .and_then(Object::as_reference)
        {
            dropped.insert(outlines);
        }
    }

    // Renumber everything that isn't dropped; a reference to anything else,
    // dropped or missing, becomes null.
    let mut next = target.max_id + 1;
    let mut map = HashMap::new();
    for &id in source.objects.keys() {
        if !dropped.contains(&id) {
            map.insert(id, (next, 0));
            next += 1;
        }
    }
    for (old, mut object) in std::mem::take(&mut source.objects) {
        if let Some(&new) = map.get(&old) {
            rewrite(&mut object, &map);
            target.objects.insert(new, object);
        }
    }
    target.max_id = next - 1;
    let info = source
        .trailer
        .get(b"Info")
        .and_then(Object::as_reference)
        .ok()
        .and_then(|id| map.get(&id).copied());
    Ok(Appended {
        pages: kept.iter().map(|id| map[id]).collect(),
        info,
    })
}

/// `key` on `page` or the nearest ancestor that has it.
fn inherited(doc: &Document, page: ObjectId, key: &[u8]) -> Option<Object> {
    let mut seen = HashSet::new();
    let mut node = page;
    loop {
        let dict = doc.get_dictionary(node).ok()?;
        if let Ok(value) = dict.get(key) {
            return Some(value.clone());
        }
        node = dict.get(b"Parent").ok()?.as_reference().ok()?;
        if !seen.insert(node) {
            return None;
        }
    }
}

/// Point every reference in `object` at its new id, or `null` when the
/// target isn't being copied.
fn rewrite(object: &mut Object, map: &HashMap<ObjectId, ObjectId>) {
    match object {
        Object::Reference(id) => {
            *object = map
                .get(id)
                .map_or(Object::Null, |&new| Object::Reference(new));
        }
        Object::Array(items) => items.iter_mut().for_each(|o| rewrite(o, map)),
        Object::Dictionary(dict) => dict.iter_mut().for_each(|(_, o)| rewrite(o, map)),
        Object::Stream(stream) => stream.dict.iter_mut().for_each(|(_, o)| rewrite(o, map)),
        _ => {}
    }
}

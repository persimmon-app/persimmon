// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! How `prsm` prints things: sizes, paths, `formats` and `info`.

use std::path::Path;

use persimmon_core::detect::Detected;
use persimmon_core::format::FormatId;
use persimmon_core::registry::{Operation, Registry};

/// A byte count in decimal units, as Finder shows it: `999 B`, `1.5 KB`,
/// `4.3 GB`.
pub fn human_size(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 0;
    // 999.95 would print as "1000.0": move up a unit instead.
    while value >= 999.95 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// A path as people expect to read it: without a leading `./`.
pub fn show(path: &Path) -> String {
    path.strip_prefix(".").unwrap_or(path).display().to_string()
}

/// `prsm formats`: one section per operation, one line per source format.
pub fn formats(reg: &Registry) -> String {
    let mut out = String::new();
    for op in [
        Operation::Convert,
        Operation::Compress,
        Operation::Extract,
        Operation::Merge,
        Operation::Split,
    ] {
        out.push_str(op.as_str());
        out.push('\n');
        for f in reg.formats() {
            if !reg
                .capabilities()
                .iter()
                .any(|c| c.op == op && c.from == f.id)
            {
                continue;
            }
            let targets = match op {
                Operation::Extract => "folder".to_string(),
                // Compress keeps the format.
                Operation::Compress => f.id.as_str().to_string(),
                _ => ids(&reg.targets(op, f.id)),
            };
            out.push_str(&format!("  {:<8}→ {targets}\n", f.id.as_str()));
        }
    }
    out
}

/// `prsm info`: label/value rows.
pub fn info(reg: &Registry, path: &Path, detected: Detected, bytes: u64) -> String {
    let format = reg.format(detected.format);
    let mut becomes = ids(&reg.targets(Operation::Convert, detected.format));
    if becomes.is_empty() {
        becomes = if reg
            .find(Operation::Extract, detected.format, None)
            .is_some()
        {
            "a folder (prsm extract)".to_string()
        } else {
            "—".to_string()
        };
    }
    let mut rows = vec![
        ("file", show(path)),
        (
            "format",
            format!("{} ({})", format.name, format.id.as_str()),
        ),
        ("size", format!("{} ({bytes} bytes)", human_size(bytes))),
        ("becomes", becomes),
    ];
    if detected.extension_mismatch {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_default();
        rows.push((
            "note",
            format!(
                "the extension says .{ext} but the content is {}",
                format.name
            ),
        ));
    }
    rows.iter()
        .map(|(label, value)| format!("{label:<9}{value}\n"))
        .collect()
}

fn ids(formats: &[FormatId]) -> String {
    formats
        .iter()
        .map(|f| f.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_size_values() {
        assert_eq!(human_size(0), "0 B");
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(1000), "1.0 KB");
        assert_eq!(human_size(1500), "1.5 KB");
        assert_eq!(human_size(999_960), "1.0 MB");
        assert_eq!(human_size(4_294_967_296), "4.3 GB");
    }

    #[test]
    fn show_drops_leading_dot() {
        assert_eq!(show(Path::new("./a.webp")), "a.webp");
        assert_eq!(show(Path::new("../a.webp")), "../a.webp");
    }
}

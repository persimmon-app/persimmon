// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Persimmon contributors

//! Which files a watched folder converts: `--match "*.jpg,*.png"`.

use std::fmt;

use crate::error::{Error, ErrorCode};
use crate::format::FormatId;
use crate::registry::{Operation, Registry};

/// File-name globs, compared ignoring case: `*` is any run of characters,
/// `?` is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patterns(Vec<String>);

impl Patterns {
    /// Patterns as `--match` takes them, separated by commas.
    pub fn parse(s: &str) -> Result<Patterns, Error> {
        let globs: Vec<String> = s
            .split(',')
            .map(str::trim)
            .filter(|g| !g.is_empty())
            .map(str::to_lowercase)
            .collect();
        if globs.is_empty() {
            return Err(Error::new(
                ErrorCode::Usage,
                "--match needs at least one pattern, such as \"*.jpg\"",
            ));
        }
        Ok(Patterns(globs))
    }

    /// `*.ext` for every extension of every format that can become `target`.
    pub fn for_target(reg: &Registry, target: FormatId) -> Patterns {
        let mut globs: Vec<String> = reg
            .formats()
            .iter()
            .filter(|f| reg.targets(Operation::Convert, f.id).contains(&target))
            .flat_map(|f| {
                f.extensions
                    .iter()
                    .map(|e| format!("*.{}", e.to_lowercase()))
            })
            .collect();
        globs.sort();
        globs.dedup();
        Patterns(globs)
    }

    /// Whether a file named `name` matches.
    pub fn matches(&self, name: &str) -> bool {
        let name: Vec<char> = name.to_lowercase().chars().collect();
        self.0
            .iter()
            .any(|g| glob(&g.chars().collect::<Vec<_>>(), &name))
    }
}

/// Whether `name` matches `pattern`: `*` any run, `?` one character.
/// Backtracks only to the last `*`, so it's linear in practice.
fn glob(pattern: &[char], name: &[char]) -> bool {
    let (mut p, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        match pattern.get(p) {
            Some('*') => {
                star = Some((p, n));
                p += 1;
            }
            Some(&c) if c == '?' || c == name[n] => {
                p += 1;
                n += 1;
            }
            _ => match star {
                Some((sp, sn)) => {
                    p = sp + 1;
                    n = sn + 1;
                    star = Some((sp, sn + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

impl fmt::Display for Patterns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_parse_and_match() {
        let p = Patterns::parse(" *.jpg, ,*.png ").unwrap();
        assert!(p.matches("a.jpg"));
        assert!(p.matches("b.png"));
        assert!(!p.matches("a.jpg.crdownload"));
        assert!(!p.matches("a.gif"));
        assert_eq!(p.to_string(), "*.jpg,*.png");
        assert!(
            Patterns::parse("scan-??.pdf")
                .unwrap()
                .matches("scan-07.pdf")
        );
        assert!(
            !Patterns::parse("scan-??.pdf")
                .unwrap()
                .matches("scan-7.pdf")
        );
        assert!(Patterns::parse("*").unwrap().matches("anything"));
        assert!(Patterns::parse("a*b*c").unwrap().matches("aXXbYYbZc"));
        assert_eq!(Patterns::parse(" , ").unwrap_err().code, ErrorCode::Usage);
    }

    #[test]
    fn patterns_ignore_case() {
        assert!(Patterns::parse("*.jpg").unwrap().matches("PHOTO.JPG"));
        assert!(Patterns::parse("*.JPG").unwrap().matches("photo.jpg"));
    }

    #[test]
    fn default_patterns_are_what_the_target_can_be_made_from() {
        let p = Patterns::for_target(&Registry::builtin(), FormatId::Webp);
        for name in [
            "a.jpg", "a.jpeg", "a.png", "a.heic", "a.tiff", "a.gif", "a.avif", "a.bmp",
        ] {
            assert!(p.matches(name), "{name}");
        }
        assert!(!p.matches("a.mp4"));
        assert!(!p.matches("a.pdf"));
    }
}

//! SigmaOS-apt-show-versions — Debian version parsing & comparison.
//!
//! Inspired by Debian's `apt-show-versions`: parses `[epoch:]upstream[-revision]`
//! version strings and orders them per Debian Policy Manual §5.6.3 — the same
//! ordering `apt`, `dpkg --compare-versions`, and `apt-show-versions` use to
//! decide which candidate is newer. Also parses `apt-show-versions`-style
//! `pkg=installed,candidate` report lines.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fmt;

/// Errors raised by this component.
#[derive(Debug, thiserror::Error)]
pub enum ComponentError {
    /// A version string had no usable upstream component.
    #[error("empty version string")]
    Empty,
}

/// Result alias for component operations.
pub type ComponentResult<T> = Result<T, ComponentError>;

/// A parsed Debian version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// Epoch (defaults to 0 when absent).
    pub epoch: u64,
    /// Upstream version (everything before the last `-`).
    pub upstream: String,
    /// Debian revision (after the last `-`), empty when absent.
    pub revision: String,
}

impl Version {
    /// Parse `[epoch:]upstream[-revision]`.
    pub fn parse(s: &str) -> ComponentResult<Self> {
        if s.is_empty() {
            return Err(ComponentError::Empty);
        }
        // Epoch: leading digits followed by ':'
        let (epoch, rest) = match s.split_once(':') {
            Some((e, r)) if !e.is_empty() && e.chars().all(|c| c.is_ascii_digit()) => {
                (e.parse::<u64>().unwrap_or(0), r)
            }
            _ => (0, s),
        };
        // Revision: split at the LAST '-' that is followed by a digit-starting
        // revision; Debian uses the last hyphen. Use rsplit_once('-').
        let (upstream, revision) = match rest.rsplit_once('-') {
            Some((u, r)) if !u.is_empty() => (u.to_string(), r.to_string()),
            _ => (rest.to_string(), String::new()),
        };
        Ok(Self { epoch, upstream, revision })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.epoch > 0 {
            write!(f, "{}:", self.epoch)?;
        }
        write!(f, "{}", self.upstream)?;
        if !self.revision.is_empty() {
            write!(f, "-{}", self.revision)?;
        }
        Ok(())
    }
}

/// Compare two version strings per Debian policy.
pub fn compare(a: &str, b: &str) -> std::cmp::Ordering {
    let va = Version::parse(a).unwrap_or(Version { epoch: 0, upstream: String::new(), revision: String::new() });
    let vb = Version::parse(b).unwrap_or(Version { epoch: 0, upstream: String::new(), revision: String::new() });
    va.epoch.cmp(&vb.epoch)
        .then_with(|| verrevcmp(&va.upstream, &vb.upstream))
        .then_with(|| {
            // Absent revision sorts BEFORE a present one only if upstream equal
            // and one is empty; Debian treats missing revision as "0"-less, so
            // compare non-empty vs empty: empty < non-empty.
            match (va.revision.is_empty(), vb.revision.is_empty()) {
                (true, true) => std::cmp::Ordering::Equal,
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                (false, false) => verrevcmp(&va.revision, &vb.revision),
            }
        })
}

/// Debian `verrevcmp`: order digit runs numerically, non-digit runs by the
/// Debian lexical rules where `~` sorts before everything (including end-of-
/// string) and letters sort before non-letters.
fn verrevcmp(a: &str, b: &str) -> std::cmp::Ordering {
    let a = a.as_bytes();
    let b = b.as_bytes();
    let (mut i, mut j) = (0usize, 0usize);
    loop {
        // Compare the non-digit prefix char-by-char with Debian ordering.
        while (i < a.len() && !a[i].is_ascii_digit()) || (j < b.len() && !b[j].is_ascii_digit()) {
            let ac = if i < a.len() { order(a[i]) } else { END };
            let bc = if j < b.len() { order(b[j]) } else { END };
            if ac != bc {
                return if ac < bc { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
            }
            // Same order class: for letters compare ASCII; else equal-ish, advance.
            if i < a.len() && j < b.len() {
                let (x, y) = (a[i], b[j]);
                if x.is_ascii_alphabetic() && y.is_ascii_alphabetic() && x != y {
                    return if x < y { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
                }
            }
            i += 1;
            j += 1;
        }
        // Skip leading zeros in the digit runs.
        while i < a.len() && a[i] == b'0' { i += 1; }
        while j < b.len() && b[j] == b'0' { j += 1; }
        // Compare digit runs by length then lexically.
        let is = i; let js = j;
        while i < a.len() && a[i].is_ascii_digit() { i += 1; }
        while j < b.len() && b[j].is_ascii_digit() { j += 1; }
        let na = &a[is..i]; let nb = &b[js..j];
        if na.len() != nb.len() {
            return if na.len() < nb.len() { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
        }
        if na != nb {
            return if na < nb { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
        }
        if i >= a.len() && j >= b.len() {
            return std::cmp::Ordering::Equal;
        }
    }
}

/// Debian order value for a byte in the non-digit phase.
/// `~` sorts before everything (even end-of-string); letters by ASCII; other
/// non-digits after letters.
fn order(c: u8) -> i32 {
    match c {
        b'~' => -1,
        b'a'..=b'z' | b'A'..=b'Z' => c as i32,
        _ => c as i32 + 256,
    }
}

/// Sentinel order for end-of-string: after `~` but before letters/others.
const END: i32 = 0;

/// One `apt-show-versions` report row: `pkg=installed,candidate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PkgVersion {
    /// Package name.
    pub name: String,
    /// Installed version (empty if not installed).
    pub installed: String,
    /// Candidate version (empty if none).
    pub candidate: String,
}

impl PkgVersion {
    /// Whether an upgrade is available (candidate > installed).
    pub fn upgradable(&self) -> bool {
        !self.candidate.is_empty()
            && (self.installed.is_empty()
                || compare(&self.candidate, &self.installed) == std::cmp::Ordering::Greater)
    }
}

/// Parse an `apt-show-versions` output line: `pkg=ver` or `pkg=inst,cand`.
pub fn parse_line(line: &str) -> Option<PkgVersion> {
    let line = line.trim();
    let (name, rhs) = line.split_once('=')?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return None;
    }
    let mut parts = rhs.split(',');
    let first = parts.next().unwrap_or("").trim();
    let second = parts.next().map(|s| s.trim().to_string());
    // `pkg=ver` means installed==candidate==ver; `pkg=i,c` means installed=i,candidate=c.
    match second {
        Some(cand) => Some(PkgVersion { name, installed: first.to_string(), candidate: cand }),
        None => Some(PkgVersion { name, installed: first.to_string(), candidate: first.to_string() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn parses_epoch_upstream_revision() {
        let v = Version::parse("1:2.3-4").unwrap();
        assert_eq!(v.epoch, 1);
        assert_eq!(v.upstream, "2.3");
        assert_eq!(v.revision, "4");
        let v2 = Version::parse("2.3").unwrap();
        assert_eq!(v2.epoch, 0);
        assert_eq!(v2.upstream, "2.3");
        assert_eq!(v2.revision, "");
    }

    #[test]
    fn epoch_dominates() {
        assert_eq!(compare("1:1.0", "2:0.1"), Ordering::Less);
        assert_eq!(compare("2:0.1", "1:9.9"), Ordering::Greater);
    }

    #[test]
    fn numeric_runs_compare_by_value() {
        assert_eq!(compare("1.10", "1.9"), Ordering::Greater);
        assert_eq!(compare("1.9", "1.10"), Ordering::Less);
        assert_eq!(compare("1.02", "1.2"), Ordering::Equal);
    }

    #[test]
    fn tilde_sorts_first() {
        assert_eq!(compare("1.0~beta", "1.0"), Ordering::Less);
        assert_eq!(compare("1.0~rc1", "1.0~beta"), Ordering::Greater);
    }

    #[test]
    fn letters_before_punctuation() {
        assert_eq!(compare("1.0a", "1.0+"), Ordering::Less);
    }

    #[test]
    fn revision_absent_sorts_first() {
        assert_eq!(compare("1.0", "1.0-1"), Ordering::Less);
        assert_eq!(compare("1.0-2", "1.0-10"), Ordering::Less);
    }

    #[test]
    fn equal_versions() {
        assert_eq!(compare("1.2.3-4", "1.2.3-4"), Ordering::Equal);
    }

    #[test]
    fn parse_report_line() {
        let p = parse_line("bash=5.2.15-2+b1").unwrap();
        assert_eq!(p.name, "bash");
        assert_eq!(p.installed, "5.2.15-2+b1");
        assert_eq!(p.candidate, "5.2.15-2+b1");
        assert!(!p.upgradable());
        let p2 = parse_line("vim=2:9.0-1,2:9.1-1").unwrap();
        assert_eq!(p2.installed, "2:9.0-1");
        assert_eq!(p2.candidate, "2:9.1-1");
        assert!(p2.upgradable());
    }
}

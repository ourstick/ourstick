//! What a receiver chose not to get (ADR 0012 and its Amendment 1): paths
//! of one or two parts, byte-exact as the stick names them. The one place
//! that decides what a choice covers, which paths a choice may hold, and
//! what a tick changes.

use std::collections::BTreeSet;

/// A choice may hold paths of at most this many parts: a top-level entry,
/// or one inside a top-level folder.
const DEEPEST: usize = 2;

/// How an entry shows in the ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    /// Got.
    On,
    /// Not got: left out, or inside a folder left out.
    Off,
    /// A folder with some of its inside left out.
    Part,
}

/// The paths left out of a stick.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LeftOut {
    paths: BTreeSet<String>,
}

impl LeftOut {
    /// The set as stored (each path checked when it was saved).
    pub fn new(paths: &[String]) -> Self {
        Self {
            paths: paths.iter().cloned().collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    /// The paths, sorted.
    pub fn paths(&self) -> Vec<String> {
        self.paths.iter().cloned().collect()
    }

    /// The left-out path that covers `path` (it, or a folder it lies in),
    /// if any. `path` is a manifest path.
    pub fn covers(&self, path: &str) -> Option<&str> {
        let mut end = 0;
        for (parts, piece) in path.split('/').enumerate() {
            if parts == DEEPEST {
                break;
            }
            end += piece.len() + usize::from(parts > 0);
            if let Some(found) = self.paths.get(&path[..end]) {
                return Some(found.as_str());
            }
        }
        None
    }

    /// How `path` (one or two parts) shows in the ticks.
    pub fn tick_of(&self, path: &str) -> Tick {
        if self.covers(path).is_some() {
            return Tick::Off;
        }
        let inside = format!("{path}/");
        if self.paths.iter().any(|left| left.starts_with(&inside)) {
            Tick::Part
        } else {
            Tick::On
        }
    }

    /// Ticks `path` on or off. `siblings` are the paths beside it in its
    /// folder (all of them, itself too): ticking one thing on inside a
    /// folder that is off turns the folder on with the rest of it off.
    pub fn tick(&mut self, path: &str, on: bool, siblings: &[String]) {
        if !on {
            // Off covers its inside: nothing under it needs its own row.
            let inside = format!("{path}/");
            self.paths.retain(|left| !left.starts_with(&inside));
            self.paths.insert(path.to_owned());
            return;
        }
        self.paths.remove(path);
        let inside = format!("{path}/");
        self.paths.retain(|left| !left.starts_with(&inside));
        if let Some((folder, _)) = path.rsplit_once('/') {
            if self.paths.remove(folder) {
                // Only what is in that folder: a list of anything else is
                // not taken for its inside.
                let inside = format!("{folder}/");
                for sibling in siblings {
                    if sibling != path
                        && sibling.starts_with(&inside)
                        && !sibling[inside.len()..].contains('/')
                    {
                        self.paths.insert(sibling.clone());
                    }
                }
            }
        }
    }

    /// The paths of `paths` a choice may hold, the others dropped: a row
    /// saved before a check refused it can match no manifest (they refuse
    /// such names too), and must not make every later save fail.
    pub fn valid_only(paths: &[String]) -> Vec<String> {
        paths
            .iter()
            .filter(|path| check(path).is_ok())
            .cloned()
            .collect()
    }

    /// Checks and simplifies a set to save: every path a plain one of at
    /// most two parts; a path under another one dropped (it is covered).
    pub fn normalised(paths: &[String]) -> Result<Self, PathProblem> {
        for path in paths {
            check(path)?;
        }
        let all: BTreeSet<&str> = paths.iter().map(String::as_str).collect();
        let kept = paths
            .iter()
            .filter(|path| match path.split_once('/') {
                Some((folder, _)) => !all.contains(folder),
                None => true,
            })
            .cloned()
            .collect();
        Ok(Self { paths: kept })
    }
}

/// Why a path cannot be in a choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PathProblem {
    #[error("a choice holds paths of at most two parts")]
    TooDeep,
    #[error("a part of a path in a choice is empty, '.' or '..'")]
    NotAName,
    #[error("a part of a path in a choice has a backslash or a control character")]
    BadCharacter,
}

/// A path a choice may hold: one or two plain parts.
fn check(path: &str) -> Result<(), PathProblem> {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > DEEPEST {
        return Err(PathProblem::TooDeep);
    }
    for part in parts {
        if part.is_empty() || part == "." || part == ".." {
            return Err(PathProblem::NotAName);
        }
        if part.chars().any(|c| c == '\\' || c.is_control()) {
            return Err(PathProblem::BadCharacter);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "left_out_tests.rs"]
mod tests;

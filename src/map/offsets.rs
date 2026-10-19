//! Labels a viewer has moved, and the file that keeps them.
//!
//! An offset is how far a label sits from its anchor, in the label's line
//! heights (em), so it holds at any zoom, text size or export resolution:
//! a moved label keeps its em distance from its anchor as the view zooms,
//! it does not travel with the data.
//!
//! An [`OffsetBook`] is one JSON file at a path the viewer chooses (beside
//! the run), with the offsets of each layout keyed by label **name**, so a
//! re-clustering that renumbers groups keeps every nudge on its label:
//! `{"version":1,"layouts":{"<layout>":{"<label>":[dx,dy]}}}`. New here,
//! for the shared viewers.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// How far each moved label sits from its anchor, in em, by key.
#[derive(Clone, Debug, PartialEq)]
pub struct Offsets<K: Eq + Hash> {
    by: HashMap<K, (f32, f32)>,
}

impl<K: Eq + Hash> Default for Offsets<K> {
    fn default() -> Self {
        Self { by: HashMap::new() }
    }
}

/// An offset this close to none is none.
const NONE_EM: f32 = 1e-4;

impl<K: Eq + Hash> Offsets<K> {
    /// Label `k`'s offset, if it was moved.
    #[must_use]
    pub fn get(&self, k: &K) -> Option<(f32, f32)> {
        self.by.get(k).copied()
    }

    /// Move label `k` by `d_em` from where it is. Moved back to its
    /// anchor, it is no longer moved. A move that is not a finite number
    /// is ignored.
    pub fn nudge(&mut self, k: K, (dx, dy): (f32, f32)) {
        if !(dx.is_finite() && dy.is_finite()) {
            return;
        }
        let at = self.by.entry(k).or_insert((0.0, 0.0));
        *at = (at.0 + dx, at.1 + dy);
        if at.0.abs() < NONE_EM && at.1.abs() < NONE_EM {
            self.by
                .retain(|_, d| d.0.abs() >= NONE_EM || d.1.abs() >= NONE_EM);
        }
    }

    /// Put label `k` back at its anchor.
    pub fn reset(&mut self, k: &K) {
        self.by.remove(k);
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by.is_empty()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by.len()
    }

    /// Every moved label and its offset, in no set order.
    pub fn iter(&self) -> impl Iterator<Item = (&K, (f32, f32))> {
        self.by.iter().map(|(k, &d)| (k, d))
    }
}

/// Offsets that are not finite numbers are left out: JSON has no place for
/// them, and one in a book would spoil the whole file.
impl<K: Eq + Hash> FromIterator<(K, (f32, f32))> for Offsets<K> {
    fn from_iter<I: IntoIterator<Item = (K, (f32, f32))>>(it: I) -> Self {
        Self {
            by: it
                .into_iter()
                .filter(|(_, (dx, dy))| dx.is_finite() && dy.is_finite())
                .collect(),
        }
    }
}

/// The version of the book's file this reads and writes.
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct BookFile {
    version: u32,
    #[serde(default)]
    layouts: BTreeMap<String, BTreeMap<String, [f32; 2]>>,
}

/// Every layout's moved labels, by label name, kept in one file.
#[derive(Clone, Debug)]
pub struct OffsetBook {
    path: PathBuf,
    layouts: BTreeMap<String, BTreeMap<String, [f32; 2]>>,
}

impl OffsetBook {
    /// The book at `path`; empty when there is none.
    ///
    /// # Errors
    /// When the file is there but cannot be read, is not a book, or is of
    /// another version (`InvalidData`), so a caller does not save over it.
    pub fn load(path: &Path) -> io::Result<Self> {
        let layouts = match std::fs::read_to_string(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => return Err(e),
            Ok(s) => {
                let f: BookFile = serde_json::from_str(&s)?;
                if f.version != VERSION {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "{}: label offsets of version {}, not {VERSION}",
                            path.display(),
                            f.version
                        ),
                    ));
                }
                f.layouts
            }
        };
        Ok(Self {
            path: path.to_path_buf(),
            layouts,
        })
    }

    /// Where the book is kept.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// No layout has a moved label.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.layouts.is_empty()
    }

    /// Layout `id`'s offsets, empty when none were kept.
    #[must_use]
    pub fn layout(&self, id: &str) -> Offsets<String> {
        self.layouts
            .get(id)
            .map(|m| {
                m.iter()
                    .map(|(k, &[dx, dy])| (k.clone(), (dx, dy)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Keep `o` as layout `id`'s offsets; none drops the layout.
    pub fn set_layout(&mut self, id: &str, o: &Offsets<String>) {
        if o.is_empty() {
            self.layouts.remove(id);
        } else {
            let m = o
                .iter()
                .map(|(k, (dx, dy))| (k.clone(), [dx, dy]))
                .collect();
            self.layouts.insert(id.to_string(), m);
        }
    }

    /// Write the book: to a temporary file beside it, then moved into
    /// place, so a reader never sees half of it. Its folder is made when
    /// missing.
    ///
    /// # Errors
    /// When the folder or the file cannot be written.
    pub fn save(&self) -> io::Result<()> {
        let dir = match self.path.parent() {
            Some(d) if !d.as_os_str().is_empty() => d,
            _ => Path::new("."),
        };
        std::fs::create_dir_all(dir)?;
        let file = BookFile {
            version: VERSION,
            layouts: self.layouts.clone(),
        };
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        serde_json::to_writer_pretty(&mut tmp, &file)?;
        tmp.write_all(b"\n")?;
        tmp.persist(&self.path).map_err(|e| e.error)?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/offsets.rs"]
mod tests;

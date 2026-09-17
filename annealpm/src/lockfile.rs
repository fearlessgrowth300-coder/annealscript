//! anneal.lock: the exact resolved version + content hash for each
//! installed package, in the same `key = value` blocks-separated-by-blank-
//! lines format as package.manifest.

use crate::manifest::{Kind, Version};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct LockEntry {
    pub version: Version,
    pub kind: Kind,
    pub hash: String,
}

pub struct Lockfile {
    pub entries: BTreeMap<String, LockEntry>,
}

impl Lockfile {
    pub fn load(path: &Path) -> Lockfile {
        let mut entries = BTreeMap::new();
        if let Ok(text) = fs::read_to_string(path) {
            for block in text.split("\n\n") {
                let (mut name, mut version, mut kind, mut hash) = (None, None, None, None);
                for line in block.lines() {
                    if let Some((k, v)) = line.split_once('=') {
                        let v = v.trim();
                        match k.trim() {
                            "name" => name = Some(v.to_string()),
                            "version" => version = Version::parse(v).ok(),
                            "kind" => kind = Kind::parse(v).ok(),
                            "hash" => hash = Some(v.to_string()),
                            _ => {}
                        }
                    }
                }
                if let (Some(name), Some(version), Some(kind), Some(hash)) = (name, version, kind, hash) {
                    entries.insert(name, LockEntry { version, kind, hash });
                }
            }
        }
        Lockfile { entries }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut out = String::new();
        for (name, e) in &self.entries {
            out.push_str(&format!("name = {name}\nversion = {}\nkind = {}\nhash = {}\n\n", e.version, e.kind.as_str(), e.hash));
        }
        fs::write(path, out).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_multiple_entries() {
        let path = std::env::temp_dir().join(format!("annealpm_lock_test_{}.lock", std::process::id()));
        let mut lock = Lockfile { entries: BTreeMap::new() };
        lock.entries.insert("greeter".into(), LockEntry { version: Version::parse("1.2.0").unwrap(), kind: Kind::Lib, hash: "abc123".into() });
        lock.entries.insert("similarity".into(), LockEntry { version: Version::parse("0.1.0").unwrap(), kind: Kind::Model, hash: "def456".into() });
        lock.save(&path).unwrap();

        let loaded = Lockfile::load(&path);
        assert_eq!(loaded.entries.len(), 2);
        assert_eq!(loaded.entries["greeter"], lock.entries["greeter"]);
        assert_eq!(loaded.entries["similarity"], lock.entries["similarity"]);

        fs::remove_file(&path).ok();
    }
}

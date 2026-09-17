//! Local filesystem package registry.
//!
//! There's no hosted intentpm server -- building a fake network client
//! against infrastructure that doesn't exist would be pure scaffolding.
//! What's real here: version resolution, content-hashed installs, and
//! handling `lib` (IntentScript source) and `model` (quantized .onnx +
//! metadata) packages through the identical publish/resolve/copy/hash
//! path. Swap `Registry` for an HTTP-backed client once there's a server
//! to talk to -- `resolve`/`publish`'s signatures don't need to change.

use crate::manifest::{self, Kind, Manifest, Version};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Registry {
    pub root: PathBuf,
}

impl Registry {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Registry { root: root.into() }
    }

    fn package_dir(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub fn package_path(&self, name: &str, version: &Version) -> PathBuf {
        self.package_dir(name).join(version.to_string())
    }

    /// Publish a local package directory into the registry under name@version.
    pub fn publish(&self, src: &Path, name: &str, version: &Version, kind: Kind, extra: HashMap<String, String>) -> Result<(), String> {
        let dest = self.package_path(name, version);
        fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        copy_dir(src, &dest)?;
        let manifest = Manifest { name: name.to_string(), version: *version, kind, fields: extra };
        fs::write(dest.join("package.manifest"), manifest::write(&manifest)).map_err(|e| e.to_string())
    }

    /// Every published version of `name`, ascending.
    pub fn versions(&self, name: &str) -> Result<Vec<Version>, String> {
        let dir = self.package_dir(name);
        let mut versions = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| format!("no such package {name:?} in registry: {e}"))? {
            let entry = entry.map_err(|e| e.to_string())?;
            if let Some(n) = entry.file_name().to_str() {
                if let Ok(v) = Version::parse(n) {
                    versions.push(v);
                }
            }
        }
        versions.sort();
        Ok(versions)
    }

    /// Resolve a requirement: "*" (or empty) means latest, anything else
    /// must match an exact published version. No caret/tilde ranges yet --
    /// nothing has needed them.
    pub fn resolve(&self, name: &str, requirement: &str) -> Result<Version, String> {
        let versions = self.versions(name)?;
        if requirement.is_empty() || requirement == "*" {
            versions.last().copied().ok_or_else(|| format!("no versions of {name:?} published"))
        } else {
            let wanted = Version::parse(requirement)?;
            versions.into_iter().find(|v| *v == wanted).ok_or_else(|| format!("{name:?}@{requirement} not found in registry"))
        }
    }

    pub fn read_manifest(&self, name: &str, version: &Version) -> Result<Manifest, String> {
        let path = self.package_path(name, version).join("package.manifest");
        let text = fs::read_to_string(&path).map_err(|e| format!("reading {path:?}: {e}"))?;
        manifest::parse(&text)
    }
}

pub fn copy_dir(src: &Path, dest: &Path) -> Result<(), String> {
    for entry in fs::read_dir(src).map_err(|e| format!("reading {src:?}: {e}"))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let dest_path = dest.join(entry.file_name());
        if entry.path().is_dir() {
            fs::create_dir_all(&dest_path).map_err(|e| e.to_string())?;
            copy_dir(&entry.path(), &dest_path)?;
        } else {
            fs::copy(entry.path(), &dest_path).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Order-independent content hash of every file in a package directory.
/// This is an integrity check (does the install match what the lockfile
/// recorded), not a cryptographic signature -- there's no key/signing
/// infrastructure to back a real one yet.
pub fn hash_dir(dir: &Path) -> Result<String, String> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(|e| format!("reading {dir:?}: {e}"))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    entries.sort();
    let mut hasher = Sha256::new();
    for path in entries {
        hasher.update(path.file_name().unwrap().to_string_lossy().as_bytes());
        hasher.update(fs::read(&path).map_err(|e| e.to_string())?);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("intentpm_test_{label}_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn publish_then_resolve_latest_and_exact() {
        let root = temp_dir("registry_resolve");
        let registry = Registry::new(&root);

        let src = temp_dir("src_pkg");
        fs::write(src.join("lib.is"), "print(1)").unwrap();

        registry.publish(&src, "greeter", &Version::parse("1.0.0").unwrap(), Kind::Lib, HashMap::new()).unwrap();
        registry.publish(&src, "greeter", &Version::parse("1.2.0").unwrap(), Kind::Lib, HashMap::new()).unwrap();

        assert_eq!(registry.resolve("greeter", "*").unwrap(), Version::parse("1.2.0").unwrap());
        assert_eq!(registry.resolve("greeter", "1.0.0").unwrap(), Version::parse("1.0.0").unwrap());
        assert!(registry.resolve("greeter", "9.9.9").is_err());

        fs::remove_dir_all(&root).ok();
        fs::remove_dir_all(&src).ok();
    }

    #[test]
    fn hash_dir_is_stable_and_detects_content_changes() {
        let dir = temp_dir("hash_dir");
        fs::write(dir.join("a.txt"), "content").unwrap();
        let h1 = hash_dir(&dir).unwrap();
        let h2 = hash_dir(&dir).unwrap();
        assert_eq!(h1, h2, "hashing the same directory twice must be stable");

        fs::write(dir.join("a.txt"), "different content").unwrap();
        let h3 = hash_dir(&dir).unwrap();
        assert_ne!(h1, h3, "changed content must change the hash");

        fs::remove_dir_all(&dir).ok();
    }
}

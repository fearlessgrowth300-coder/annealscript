//! Dead-simple `key = value` manifest format. No toml/serde dependency --
//! five fields don't need one.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Lib,
    Model,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Lib => "lib",
            Kind::Model => "model",
        }
    }

    pub fn parse(s: &str) -> Result<Kind, String> {
        match s {
            "lib" => Ok(Kind::Lib),
            "model" => Ok(Kind::Model),
            other => Err(format!("unknown kind {other:?}, expected 'lib' or 'model'")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub fn parse(s: &str) -> Result<Version, String> {
        let parts: Vec<&str> = s.trim().split('.').collect();
        if parts.len() != 3 {
            return Err(format!("version {s:?} must be major.minor.patch"));
        }
        let nums: Result<Vec<u32>, _> = parts.iter().map(|p| p.parse::<u32>()).collect();
        let nums = nums.map_err(|_| format!("version {s:?} must be numeric major.minor.patch"))?;
        Ok(Version { major: nums[0], minor: nums[1], patch: nums[2] })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    pub name: String,
    pub version: Version,
    pub kind: Kind,
    /// Extra metadata: `entry` for lib packages, `task` for model packages, etc.
    pub fields: HashMap<String, String>,
}

pub fn parse(text: &str) -> Result<Manifest, String> {
    let mut fields = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (k, v) = line.split_once('=').ok_or_else(|| format!("bad manifest line: {line:?}"))?;
        fields.insert(k.trim().to_string(), v.trim().to_string());
    }
    let name = fields.remove("name").ok_or("manifest missing 'name'")?;
    let version = Version::parse(&fields.remove("version").ok_or("manifest missing 'version'")?)?;
    let kind = Kind::parse(&fields.remove("kind").ok_or("manifest missing 'kind'")?)?;
    Ok(Manifest { name, version, kind, fields })
}

pub fn write(m: &Manifest) -> String {
    let mut out = format!("name = {}\nversion = {}\nkind = {}\n", m.name, m.version, m.kind.as_str());
    for (k, v) in &m.fields {
        out.push_str(&format!("{k} = {v}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_parse_and_write() {
        let mut fields = HashMap::new();
        fields.insert("task".to_string(), "similarity-scorer".to_string());
        let m = Manifest { name: "similarity".into(), version: Version::parse("1.2.0").unwrap(), kind: Kind::Model, fields };
        let text = write(&m);
        let parsed = parse(&text).unwrap();
        assert_eq!(parsed, m);
    }

    #[test]
    fn versions_compare_by_semver_order() {
        let v1 = Version::parse("1.2.0").unwrap();
        let v2 = Version::parse("1.10.0").unwrap();
        assert!(v1 < v2, "1.10.0 must sort after 1.2.0, not lexicographically before it");
    }
}

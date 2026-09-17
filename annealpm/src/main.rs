use annealpm::lockfile::{LockEntry, Lockfile};
use annealpm::manifest::{Kind, Version};
use annealpm::registry::{self, Registry};
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let registry = Registry::new(env::var("ANNEALPM_REGISTRY").unwrap_or_else(|_| "registry".to_string()));
    let lock_path = PathBuf::from("anneal.lock");

    match args.get(1).map(String::as_str) {
        Some("publish") => {
            let src = PathBuf::from(args.get(2).ok_or("usage: annealpm publish <src_dir> <name> <version> <lib|model> [k=v ...]")?);
            let name = args.get(3).ok_or("missing name")?;
            let version = Version::parse(args.get(4).ok_or("missing version")?)?;
            let kind = Kind::parse(args.get(5).ok_or("missing kind (lib|model)")?)?;
            let mut extra = HashMap::new();
            for kv in &args[6.min(args.len())..] {
                if let Some((k, v)) = kv.split_once('=') {
                    extra.insert(k.to_string(), v.to_string());
                }
            }
            registry.publish(&src, name, &version, kind, extra)?;
            println!("published {name}@{version} ({}) to registry", kind.as_str());
        }
        Some("add") => {
            let name = args.get(2).ok_or("usage: annealpm add <name> [version]")?;
            let requirement = args.get(3).map(String::as_str).unwrap_or("*");
            let version = registry.resolve(name, requirement)?;
            let manifest = registry.read_manifest(name, &version)?;
            let src = registry.package_path(name, &version);
            let dest = PathBuf::from("anneal_packages").join(name).join(version.to_string());
            std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            registry::copy_dir(&src, &dest)?;
            let hash = registry::hash_dir(&dest)?;

            let mut lock = Lockfile::load(&lock_path);
            lock.entries.insert(name.clone(), LockEntry { version, kind: manifest.kind, hash });
            lock.save(&lock_path)?;
            println!("added {name}@{version} [{}] -> anneal_packages/{name}/{version}", manifest.kind.as_str());
        }
        Some("list") => {
            let lock = Lockfile::load(&lock_path);
            if lock.entries.is_empty() {
                println!("no packages installed (see anneal.lock)");
            }
            for (name, e) in &lock.entries {
                println!("{name}@{} [{}] sha256:{}", e.version, e.kind.as_str(), &e.hash[..12]);
            }
        }
        _ => {
            eprintln!("usage: annealpm <publish|add|list> ...");
            std::process::exit(2);
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

//! A connector package: its manifest and its other files (fixtures, assets, `dist/`), from a directory or from files
//! compiled into the server. Loading validates it ([`crate::validate`]); an invalid package is refused with every
//! problem found, each naming where it is.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::manifest::Manifest;

/// Where a package comes from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Origin {
    /// Shipped with ObserveCraft (`integrations/connectors/`).
    Bundled,
    /// The operator's (`OC_CONNECTORS_DIR`): community, third-party or private.
    ThirdParty { dir: String },
}

#[derive(Clone, Debug)]
pub struct Package {
    pub manifest: Manifest,
    /// Text files by their path in the package (`fixtures/events.json`).
    pub files: BTreeMap<String, String>,
    pub origin: Origin,
}

/// The manifest's file name.
pub const MANIFEST: &str = "connector.yaml";
/// Files larger than this are not read.
pub const MAX_FILE_BYTES: u64 = 8 << 20;

impl Package {
    /// A package from its files; `Err` lists every problem.
    pub fn from_files(files: BTreeMap<String, String>, origin: Origin) -> Result<Package, Vec<String>> {
        let text = files.get(MANIFEST).ok_or_else(|| vec![format!("{MANIFEST} is missing")])?;
        let manifest: Manifest = serde_yaml_ng::from_str(text).map_err(|e| vec![format!("{MANIFEST}: {e}")])?;
        let problems = crate::validate::validate(&manifest, &files);
        if !problems.is_empty() {
            return Err(problems);
        }
        Ok(Package { manifest, files, origin })
    }

    /// A package from a directory (every text file under it, `.git` and hidden files left out).
    pub fn from_dir(dir: &Path, origin: Origin) -> Result<Package, Vec<String>> {
        let mut files = BTreeMap::new();
        read_dir(dir, dir, &mut files).map_err(|e| vec![e])?;
        Package::from_files(files, origin)
    }

    pub fn file(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(String::as_str)
    }

    fn json(&self, path: &str) -> Option<Value> {
        self.file(path).and_then(|t| serde_json::from_str(t).ok())
    }

    /// The dashboards converted to ObserveCraft's format (`dist/dashboards.json`).
    pub fn dashboards(&self) -> Vec<Value> {
        match self.json("dist/dashboards.json") {
            Some(Value::Array(a)) => a,
            _ => vec![],
        }
    }

    /// The log pipeline, in Datadog's pipeline JSON.
    pub fn pipeline(&self) -> Option<Value> {
        self.manifest.assets.pipeline.as_deref().and_then(|p| self.json(p))
    }

    pub fn monitors(&self) -> Vec<Value> {
        self.manifest.assets.monitors.iter().filter_map(|p| self.json(p)).collect()
    }

    pub fn security_rules(&self) -> Vec<Value> {
        self.manifest.assets.security_rules.iter().filter_map(|p| self.json(p)).collect()
    }

    pub fn log_metrics(&self) -> Vec<Value> {
        self.manifest.assets.log_metrics.iter().filter_map(|p| self.json(p)).collect()
    }

    /// The source logs carry (`ddsource`).
    pub fn source(&self) -> &str {
        self.manifest.source.as_deref().unwrap_or(&self.manifest.id)
    }

    /// The logo: the manifest's, else its first tile, else its id.
    pub fn logo(&self) -> &str {
        self.manifest.logo.as_deref().or(self.manifest.tiles.first().map(String::as_str)).unwrap_or(&self.manifest.id)
    }
}

fn read_dir(root: &Path, dir: &Path, out: &mut BTreeMap<String, String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        // Following symlinks: a Kubernetes ConfigMap's files are links.
        let meta = std::fs::metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        if meta.is_dir() {
            read_dir(root, &path, out)?;
        } else if meta.is_file() && meta.len() <= MAX_FILE_BYTES {
            let rel = path.strip_prefix(root).map_err(|e| e.to_string())?.to_string_lossy().replace('\\', "/");
            // Binary files (images) aren't part of what the server reads.
            if let Ok(text) = String::from_utf8(std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?) {
                out.insert(rel, text);
            }
        }
    }
    Ok(())
}

/// Every package under `root`: one directory each, or flat files named `<package>__<path with __ for />` (a
/// Kubernetes ConfigMap holds no directories: `acme-status__connector.yaml`, `acme-status__fixtures__a.json`). The
/// valid ones, and the problems of the others by name.
pub fn load_all(root: &Path, origin: impl Fn(&Path) -> Origin) -> (Vec<Package>, Vec<(String, Vec<String>)>) {
    let mut packages = Vec::new();
    let mut invalid = Vec::new();
    let Ok(entries) = std::fs::read_dir(root) else {
        invalid.push((root.display().to_string(), vec!["the directory can't be read".to_string()]));
        return (packages, invalid);
    };
    let mut dirs = Vec::new();
    let mut flat: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for path in entries.flatten().map(|e| e.path()) {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            if path.join(MANIFEST).is_file() {
                dirs.push(path);
            }
        } else if let Some((package, rest)) = name.split_once("__") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                flat.entry(package.to_string()).or_default().insert(rest.replace("__", "/"), text);
            }
        }
    }
    dirs.sort();
    for dir in dirs {
        match Package::from_dir(&dir, origin(&dir)) {
            Ok(p) => packages.push(p),
            Err(problems) => invalid.push((dir.display().to_string(), problems)),
        }
    }
    for (name, files) in flat {
        let at = root.join(&name);
        match Package::from_files(files, origin(&at)) {
            Ok(p) => packages.push(p),
            Err(problems) => invalid.push((at.display().to_string(), problems)),
        }
    }
    (packages, invalid)
}

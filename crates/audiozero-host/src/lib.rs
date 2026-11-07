//! Finding and resolving third-party plugins (CLAP and VST3).
//!
//! This crate doesn't load plugin code yet. It establishes the rules loading will follow:
//!
//! - The [`Registry`] only holds plugins the user scanned or added. A project names plugins by
//!   identity ([`PluginRef`]), and [`Registry::resolve`] matches that identity. A path written
//!   in a project is never used (security review, finding 1).
//! - [`find_bundles`] lists candidate bundles on disk without opening them. Reading a plugin's
//!   identity means running its code, which will happen in a separate scanner process, so a
//!   crashing or hostile plugin can't take AudioZero down (finding 8).

use audiozero_engine::{PluginFormat, PluginRef};
use std::path::{Path, PathBuf};

/// A plugin the user has made available.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginInfo {
    pub reference: PluginRef,
    pub path: PathBuf,
    pub is_instrument: bool,
}

#[derive(Debug, Default)]
pub struct Registry {
    plugins: Vec<PluginInfo>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a plugin found by a scan, or one the user dragged in. Replaces an entry with the
    /// same identity.
    pub fn add(&mut self, info: PluginInfo) {
        self.plugins
            .retain(|p| !same_identity(&p.reference, &info.reference));
        self.plugins.push(info);
    }

    pub fn plugins(&self) -> &[PluginInfo] {
        &self.plugins
    }

    /// Finds the installed plugin a project refers to, by format and id. `None` means the
    /// plugin is missing: show it as a placeholder that keeps its state.
    pub fn resolve(&self, reference: &PluginRef) -> Option<&PluginInfo> {
        self.plugins
            .iter()
            .find(|p| same_identity(&p.reference, reference))
    }
}

fn same_identity(a: &PluginRef, b: &PluginRef) -> bool {
    a.format == b.format && a.id == b.id
}

pub fn extension(format: PluginFormat) -> &'static str {
    match format {
        PluginFormat::Clap => "clap",
        PluginFormat::Vst3 => "vst3",
    }
}

/// The standard install folders for a format on this platform.
pub fn default_search_paths(format: PluginFormat) -> Vec<PathBuf> {
    let env = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let mut paths = Vec::new();
    let folder = match format {
        PluginFormat::Clap => "CLAP",
        PluginFormat::Vst3 => "VST3",
    };
    if cfg!(target_os = "macos") {
        if let Some(home) = env("HOME") {
            paths.push(home.join("Library/Audio/Plug-Ins").join(folder));
        }
        paths.push(PathBuf::from("/Library/Audio/Plug-Ins").join(folder));
    } else if cfg!(windows) {
        if let Some(common) = env("COMMONPROGRAMFILES") {
            paths.push(common.join(folder));
        }
        if let Some(local) = env("LOCALAPPDATA") {
            paths.push(local.join("Programs").join("Common").join(folder));
        }
    } else {
        let dot = format!(".{}", extension(format));
        if let Some(home) = env("HOME") {
            paths.push(home.join(dot));
        }
        paths.push(PathBuf::from("/usr/lib").join(extension(format)));
        paths.push(PathBuf::from("/usr/local/lib").join(extension(format)));
    }
    paths
}

/// Limits on [`find_bundles`], so a huge or looping folder tree can't stall a scan.
pub const MAX_SCAN_DEPTH: usize = 6;
pub const MAX_SCAN_ENTRIES: usize = 50_000;

/// Lists `.clap` or `.vst3` files and bundles under `roots`, without opening them. Doesn't
/// follow symbolic links, so a link loop can't make it run forever.
pub fn find_bundles(roots: &[PathBuf], format: PluginFormat) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut visited = 0usize;
    for root in roots {
        walk(root, extension(format), 0, &mut visited, &mut found);
    }
    found.sort();
    found.dedup();
    found
}

fn walk(dir: &Path, ext: &str, depth: usize, visited: &mut usize, found: &mut Vec<PathBuf>) {
    if depth > MAX_SCAN_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        *visited += 1;
        if *visited > MAX_SCAN_ENTRIES {
            return;
        }
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let matches = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(ext));
        if matches {
            found.push(path); // a bundle directory or a single file; either way, don't descend
        } else if kind.is_dir() {
            walk(&path, ext, depth + 1, visited, found);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(id: &str) -> PluginRef {
        PluginRef {
            format: PluginFormat::Clap,
            id: id.into(),
            name: "Synth".into(),
            vendor: "Example".into(),
        }
    }

    #[test]
    fn resolves_by_identity_only() {
        let mut registry = Registry::new();
        registry.add(PluginInfo {
            reference: reference("com.example.synth"),
            path: "/plugins/Synth.clap".into(),
            is_instrument: true,
        });
        let found = registry.resolve(&reference("com.example.synth")).unwrap();
        assert_eq!(found.path, PathBuf::from("/plugins/Synth.clap"));
        assert!(registry.resolve(&reference("com.example.other")).is_none());
        let as_vst3 = PluginRef {
            format: PluginFormat::Vst3,
            ..reference("com.example.synth")
        };
        assert!(
            registry.resolve(&as_vst3).is_none(),
            "the format is part of the identity"
        );
    }

    #[test]
    fn re_adding_replaces_the_entry() {
        let mut registry = Registry::new();
        for path in ["/old/Synth.clap", "/new/Synth.clap"] {
            registry.add(PluginInfo {
                reference: reference("id"),
                path: path.into(),
                is_instrument: true,
            });
        }
        assert_eq!(registry.plugins().len(), 1);
        assert_eq!(registry.plugins()[0].path, PathBuf::from("/new/Synth.clap"));
    }

    #[test]
    fn finds_bundles_without_descending_into_them() {
        let root = std::env::temp_dir().join(format!("audiozero-scan-{}", std::process::id()));
        let bundle = root.join("Vendor/Synth.vst3/Contents/x86_64-linux");
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::write(bundle.join("Synth.so"), b"").unwrap();
        std::fs::write(root.join("Single.VST3"), b"").unwrap();
        std::fs::write(root.join("readme.txt"), b"").unwrap();

        let found = find_bundles(std::slice::from_ref(&root), PluginFormat::Vst3);
        assert_eq!(
            found,
            vec![root.join("Single.VST3"), root.join("Vendor/Synth.vst3")]
        );
        assert!(find_bundles(&[root.join("missing")], PluginFormat::Vst3).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn has_default_search_paths() {
        assert!(!default_search_paths(PluginFormat::Clap).is_empty());
        assert!(!default_search_paths(PluginFormat::Vst3).is_empty());
    }
}

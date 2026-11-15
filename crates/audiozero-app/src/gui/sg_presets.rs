//! The user's own Soundgineer presets: JSON files in a folder next to the settings.
//!
//! Everything read from disk is untrusted, so:
//! * only regular files named `*.json` directly in the folder are listed (no symlinks, no
//!   sub-folders), at most [`MAX_PRESETS`] of them, each no larger than [`MAX_FILE_BYTES`];
//! * a file is parsed with [`UserPreset::from_json`], which rebuilds it value by value;
//! * an imported file is never copied: its parsed, checked contents are written out again under
//!   a file name made from the preset's name, so a name can't be a path;
//! * saving never replaces a file (`create_new`), and the file is written in one go.

use audiozero_engine::soundgineer::userpreset::{MAX_FILE_BYTES, UserPreset, file_stem};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The most presets listed from the folder.
pub const MAX_PRESETS: usize = 500;

/// A preset in the user's folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    pub path: PathBuf,
    pub preset: UserPreset,
}

/// Where the presets live: `presets/soundgineer` next to the settings file.
pub fn folder() -> Option<PathBuf> {
    Some(
        crate::settings::file()?
            .parent()?
            .join("presets")
            .join("soundgineer"),
    )
}

/// Reads a preset file, refusing anything that isn't a small regular `.json` file.
pub fn read_file(path: &Path) -> Result<UserPreset, String> {
    if !path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("json"))
    {
        return Err("only .json preset files can be imported".into());
    }
    let meta = fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err("that isn't a file".into());
    }
    if meta.len() == 0 {
        return Err("the file is empty".into());
    }
    if meta.len() > MAX_FILE_BYTES as u64 {
        return Err(format!(
            "the file is too large (the limit is {} KB)",
            MAX_FILE_BYTES / 1024
        ));
    }
    // Read at most one byte more than the limit, in case the file grew since the check.
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(MAX_FILE_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let text = String::from_utf8(bytes).map_err(|_| "the file isn't text".to_owned())?;
    UserPreset::from_json(&text)
}

/// The presets in `dir`, by name. Files that aren't valid presets are left out.
pub fn list(dir: &Path) -> Vec<Stored> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<Stored> = entries
        .filter_map(Result::ok)
        .take(MAX_PRESETS * 2)
        .filter_map(|entry| {
            // `file_type` doesn't follow symlinks, so a link is neither a file nor listed.
            if !entry.file_type().ok()?.is_file() {
                return None;
            }
            let path = entry.path();
            let preset = read_file(&path).ok()?;
            Some(Stored { path, preset })
        })
        .take(MAX_PRESETS)
        .collect();
    found.sort_by_key(|s| s.preset.name.to_lowercase());
    found
}

/// Writes `preset` into `dir` as a new file and returns its path. An existing file is never
/// replaced: a name that's taken gets " 2", " 3" and so on.
pub fn save(dir: &Path, preset: &UserPreset) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let stem = file_stem(&preset.name);
    let json = preset.to_json();
    for n in 1..=999 {
        let name = if n == 1 {
            format!("{stem}.json")
        } else {
            format!("{stem}-{n}.json")
        };
        let path = dir.join(name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                return file
                    .write_all(json.as_bytes())
                    .map(|()| path.clone())
                    .map_err(|e| {
                        let _ = fs::remove_file(&path);
                        format!("{}: {e}", path.display())
                    });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Err("there are too many presets with that name".into())
}

/// Imports the file at `path` into `dir`: reads and checks it, then saves the checked copy.
pub fn import(dir: &Path, path: &Path) -> Result<Stored, String> {
    let preset = read_file(path)?;
    if list(dir).len() >= MAX_PRESETS {
        return Err(format!("there are already {MAX_PRESETS} presets"));
    }
    let path = save(dir, &preset)?;
    Ok(Stored { path, preset })
}

/// Deletes a preset's file, if it's a regular `.json` file directly inside `dir`.
pub fn delete(dir: &Path, path: &Path) -> Result<(), String> {
    let inside = path.parent() == Some(dir);
    let is_json = path.extension().is_some_and(|e| e == "json");
    let regular = fs::symlink_metadata(path).is_ok_and(|m| m.is_file());
    if !(inside && is_json && regular) {
        return Err("that isn't one of your presets".into());
    }
    fs::remove_file(path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("azp-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn preset(name: &str) -> UserPreset {
        UserPreset::capture(name, &BTreeMap::from([("osc1.morph".to_owned(), 0.5)]))
    }

    #[test]
    fn save_list_and_delete() {
        let dir = temp("save").join("presets");
        let a = save(&dir, &preset("Warm Bass")).unwrap();
        let b = save(&dir, &preset("Warm Bass")).unwrap();
        assert_ne!(a, b, "a second save never replaces the first");
        assert_eq!(a.file_name().unwrap(), "warm-bass.json");
        assert_eq!(b.file_name().unwrap(), "warm-bass-2.json");
        let listed = list(&dir);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].preset, preset("Warm Bass"));
        delete(&dir, &a).unwrap();
        assert_eq!(list(&dir).len(), 1);
        assert!(delete(&dir, &dir.join("../x.json")).is_err());
        assert!(delete(&dir, Path::new("/etc/passwd")).is_err());
    }

    #[test]
    fn a_name_can_never_be_a_path() {
        let dir = temp("path");
        let path = save(&dir, &preset("../../escape")).unwrap();
        assert_eq!(path.parent().unwrap(), dir);
        assert_eq!(path.file_name().unwrap(), "escape.json");
    }

    #[test]
    fn import_checks_the_file_and_writes_a_clean_copy() {
        let dir = temp("import");
        let store = dir.join("store");
        let good = dir.join("in.json");
        let mut json = preset("Imported").to_json();
        json = json.replace("\"params\"", "\"extra\": \"ignored\", \"params\"");
        fs::write(&good, &json).unwrap();
        let stored = import(&store, &good).unwrap();
        let written = fs::read_to_string(&stored.path).unwrap();
        assert!(!written.contains("extra"), "only checked data is written");
        assert_eq!(stored.preset.name, "Imported");

        for (name, body) in [
            ("notjson.txt", json.clone()),
            ("empty.json", String::new()),
            ("bad.json", "{ nope".into()),
            ("other.json", r#"{"format":"x"}"#.into()),
            ("huge.json", " ".repeat(MAX_FILE_BYTES + 1)),
        ] {
            let path = dir.join(name);
            fs::write(&path, body).unwrap();
            assert!(import(&store, &path).is_err(), "{name} is refused");
        }
        assert!(import(&store, &dir).is_err(), "a folder is refused");
        assert!(import(&store, &dir.join("missing.json")).is_err());
        assert_eq!(
            list(&store).len(),
            1,
            "nothing was added by the refused ones"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_strangers_are_not_listed() {
        let dir = temp("links");
        let real = dir.join("real.json");
        fs::write(&real, preset("Real").to_json()).unwrap();
        std::os::unix::fs::symlink(&real, dir.join("link.json")).unwrap();
        fs::create_dir(dir.join("folder.json")).unwrap();
        fs::write(dir.join("readme.txt"), "hi").unwrap();
        fs::write(dir.join("broken.json"), "{").unwrap();
        let listed = list(&dir);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].path, real);
    }
}

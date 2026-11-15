//! Soundgineer presets as JSON files: what "Save", "Import" and the generated extras use.
//!
//! A preset file is untrusted input, so it is never copied or deserialised straight into the
//! project. [`UserPreset::from_json`] checks the size, the format tag and the version, then
//! rebuilds the preset one value at a time: names must be Soundgineer's own parameters (others
//! are dropped), values must be finite numbers, and each is fitted to its parameter's range.
//! Nothing in a file can name a path, load code or grow past [`MAX_FILE_BYTES`].
//!
//! ```json
//! { "format": "audiozero-soundgineer-preset", "version": 1, "name": "My Bass",
//!   "params": { "osc1.morph": 0.7, "filter1.cutoff": 300.0 } }
//! ```
//! `params` lists only what differs from the defaults, like the factory presets do.

use super::PARAMS;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// The `format` tag every preset file carries.
pub const FORMAT: &str = "audiozero-soundgineer-preset";
pub const VERSION: u64 = 1;
/// Largest preset file read. A full preset with every curve is about 30 KB.
pub const MAX_FILE_BYTES: usize = 256 * 1024;
pub const MAX_NAME_LENGTH: usize = 64;
/// The most presets read from one bundle (the generated extras).
pub const MAX_BUNDLE: usize = 2_000;

/// A named set of Soundgineer parameter values, every one valid.
#[derive(Debug, Clone, PartialEq)]
pub struct UserPreset {
    pub name: String,
    /// Parameter name to value, fitted to the parameter's range. Only non-default values.
    pub params: BTreeMap<String, f32>,
}

/// A name with control characters removed and its length capped; "Untitled" if nothing is left.
pub fn clean_name(name: &str) -> String {
    let name: String = name
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(MAX_NAME_LENGTH)
        .collect();
    let name = name.trim().to_owned();
    if name.is_empty() {
        "Untitled".into()
    } else {
        name
    }
}

impl UserPreset {
    /// A preset from a device's current settings: whatever differs from the defaults.
    pub fn capture(name: &str, settings: &BTreeMap<String, f32>) -> Self {
        let params = PARAMS
            .iter()
            .filter_map(|p| {
                let v = p.fit(*settings.get(p.name)?);
                (v != p.default).then(|| (p.name.to_owned(), v))
            })
            .collect();
        Self {
            name: clean_name(name),
            params,
        }
    }

    /// The preset as pretty-printed JSON.
    pub fn to_json(&self) -> String {
        let params: Map<String, Value> = self
            .params
            .iter()
            .map(|(k, v)| (k.clone(), Value::from(f64::from(*v))))
            .collect();
        let mut root = Map::new();
        root.insert("format".into(), FORMAT.into());
        root.insert("version".into(), VERSION.into());
        root.insert("name".into(), self.name.clone().into());
        root.insert("params".into(), params.into());
        serde_json::to_string_pretty(&Value::Object(root)).expect("a preset is plain data") + "\n"
    }

    /// Reads one preset from a file's text. Refuses anything that isn't a Soundgineer preset.
    pub fn from_json(text: &str) -> Result<Self, String> {
        if text.len() > MAX_FILE_BYTES {
            return Err(format!(
                "the file is too large (the limit is {} KB)",
                MAX_FILE_BYTES / 1024
            ));
        }
        let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .map_err(|_| "the file isn't valid JSON".to_owned())?;
        Self::from_value(&value)
    }

    /// Reads a list of presets (a bundle), each with its menu category. Entries that aren't
    /// valid are skipped; it's an error only if the text isn't a list at all or is too long.
    pub fn bundle_from_json(text: &str) -> Result<Vec<(String, Self)>, String> {
        let value: Value =
            serde_json::from_str(text).map_err(|_| "the bundle isn't valid JSON".to_owned())?;
        let list = value.as_array().ok_or("a bundle is a JSON list")?;
        if list.len() > MAX_BUNDLE {
            return Err("the bundle has too many presets".into());
        }
        Ok(list
            .iter()
            .filter_map(|v| {
                let category = clean_name(v.get("category")?.as_str()?);
                Some((category, Self::from_value(v).ok()?))
            })
            .collect())
    }

    fn from_value(value: &Value) -> Result<Self, String> {
        let root = value.as_object().ok_or("not a preset")?;
        if root.get("format").and_then(Value::as_str) != Some(FORMAT) {
            return Err("not an AudioZero Soundgineer preset".into());
        }
        match root.get("version").and_then(Value::as_u64) {
            Some(VERSION) => {}
            _ => return Err("this preset is from a different version".into()),
        }
        let name = clean_name(root.get("name").and_then(Value::as_str).unwrap_or_default());
        let raw = root
            .get("params")
            .map(|p| p.as_object().ok_or("\"params\" must be an object"))
            .transpose()?;
        let mut params = BTreeMap::new();
        for (key, v) in raw.into_iter().flatten() {
            if params.len() >= PARAMS.len() {
                return Err("too many parameters".into());
            }
            let Some(info) = PARAMS.iter().find(|p| p.name == key) else {
                continue; // from a newer version, or not ours: ignored
            };
            let number = v
                .as_f64()
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("\"{key}\" must be a number"))?;
            let fitted = info.fit(number as f32);
            if fitted != info.default {
                params.insert(info.name.to_owned(), fitted);
            }
        }
        Ok(Self { name, params })
    }
}

/// The generated extra presets (`scripts/soundgineer/make_presets.py`), as (category, preset).
pub fn extras() -> &'static [(String, UserPreset)] {
    static EXTRAS: std::sync::LazyLock<Vec<(String, UserPreset)>> =
        std::sync::LazyLock::new(|| {
            UserPreset::bundle_from_json(include_str!("extra_presets.json")).unwrap_or_default()
        });
    &EXTRAS
}

/// A file-name stem for a preset name: lower-case letters, digits and dashes only, so a name can
/// never become a path. Never empty.
pub fn file_stem(name: &str) -> String {
    let mut stem = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            stem.push(c.to_ascii_lowercase());
        } else if !stem.ends_with('-') && !stem.is_empty() {
            stem.push('-');
        }
    }
    let stem = stem.trim_end_matches('-');
    if stem.is_empty() {
        "preset".into()
    } else {
        stem.chars().take(48).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> UserPreset {
        UserPreset::capture(
            "My Bass",
            &BTreeMap::from([
                ("osc1.morph".to_owned(), 0.7),
                ("filter1.cutoff".to_owned(), 300.0),
                ("osc1.level".to_owned(), 99.0), // out of range: fitted
                ("nope".to_owned(), 1.0),        // not a parameter: dropped
            ]),
        )
    }

    #[test]
    fn a_preset_round_trips() {
        let p = sample();
        assert!(!p.params.contains_key("nope"));
        assert_eq!(UserPreset::from_json(&p.to_json()).unwrap(), p);
    }

    #[test]
    fn defaults_are_not_stored() {
        let p = UserPreset::capture("Init", &BTreeMap::from([("osc1.enabled".to_owned(), 1.0)]));
        assert!(p.params.is_empty(), "osc1.enabled is on by default");
    }

    #[test]
    fn bad_files_are_refused() {
        let json = |s: &str| UserPreset::from_json(s);
        assert!(json("").is_err());
        assert!(json("[]").is_err());
        assert!(json("{}").is_err());
        assert!(json(r#"{"format":"other","version":1}"#).is_err());
        assert!(json(&format!(r#"{{"format":"{FORMAT}","version":2}}"#)).is_err());
        let bad = format!(r#"{{"format":"{FORMAT}","version":1,"params":{{"osc1.morph":"x"}}}}"#);
        assert!(json(&bad).is_err());
        let list = format!(r#"{{"format":"{FORMAT}","version":1,"params":[1]}}"#);
        assert!(json(&list).is_err());
        assert!(json(&"x".repeat(MAX_FILE_BYTES + 1)).is_err());
        let big = format!(r#"{{"format":"{FORMAT}","version":1,"params":{{"osc1.morph":1e999}}}}"#);
        assert!(json(&big).is_err(), "infinity isn't a number");
    }

    #[test]
    fn values_are_fitted_and_unknown_names_dropped() {
        let text = format!(
            r#"{{"format":"{FORMAT}","version":1,"name":"  \u0007Hi\n ","params":
               {{"osc1.morph":5,"osc1.unison":3.6,"../../etc":1,"filter1.cutoff":-9}}}}"#
        );
        let p = UserPreset::from_json(&text).unwrap();
        assert_eq!(p.name, "Hi");
        assert_eq!(p.params["osc1.morph"], 1.0);
        assert_eq!(p.params["osc1.unison"], 4.0);
        assert!(p.params["filter1.cutoff"] > 0.0);
        assert_eq!(p.params.len(), 3);
    }

    #[test]
    fn names_and_file_stems_are_safe() {
        assert_eq!(clean_name("\u{0}\n"), "Untitled");
        assert_eq!(clean_name(&"a".repeat(500)).len(), MAX_NAME_LENGTH);
        assert_eq!(file_stem("../../Evil Name!"), "evil-name");
        assert_eq!(file_stem("///"), "preset");
        assert!(!file_stem("a/b\\c:d").contains(['/', '\\', ':', '.']));
    }

    #[test]
    fn the_generated_extras_load_in_full() {
        let extras = extras();
        assert_eq!(extras.len(), 72);
        let names: std::collections::HashSet<_> = extras.iter().map(|(_, p)| &p.name).collect();
        assert_eq!(names.len(), extras.len(), "names are unique");
        assert!(extras.iter().all(|(_, p)| !p.params.is_empty()));
    }

    #[test]
    fn bundles_skip_bad_entries() {
        let good = sample().to_json();
        let good = good.replacen('{', r#"{"category":"Bass","#, 1);
        let text = format!(r#"[{good}, {{"format":"x"}}, 5]"#);
        assert_eq!(UserPreset::bundle_from_json(&text).unwrap().len(), 1);
        assert!(UserPreset::bundle_from_json("{}").is_err());
    }
}

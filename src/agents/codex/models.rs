//! Session model names from Codex's own local catalog. No defaults or network calls.
use serde::Deserialize;
use std::{collections::HashMap, fs::File, io::Read, path::Path};

#[derive(Default, Deserialize)]
struct Catalog {
    #[serde(default)]
    models: Vec<Model>,
}
#[derive(Deserialize)]
struct Model {
    slug: String,
    display_name: String,
}

pub(super) fn display_names(home: &Path) -> HashMap<String, String> {
    let read = || -> Option<Catalog> {
        let file = File::open(home.join("models_cache.json")).ok()?;
        serde_json::from_reader(file.take(8 * 1024 * 1024)).ok()
    };
    read()
        .unwrap_or_default()
        .models
        .into_iter()
        .filter_map(|model| {
            let slug = model.slug.trim();
            let name = model.display_name.trim();
            (!slug.is_empty() && !name.is_empty()).then(|| (slug.to_owned(), name.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uses_catalog_names_verbatim_and_reloads_changes_without_inventing_defaults() {
        let home = tempfile::tempdir().unwrap();
        assert!(display_names(home.path()).is_empty());
        let path = home.path().join("models_cache.json");
        std::fs::write(&path, r#"{"models":[{"slug":"gpt-example","display_name":"Official Display-Name"},{"slug":"empty","display_name":"  "}]}"#).unwrap();
        let names = display_names(home.path());
        assert_eq!(
            names.get("gpt-example").map(String::as_str),
            Some("Official Display-Name")
        );
        assert!(!names.contains_key("empty"));
        assert!(!names.contains_key("unknown"));
        std::fs::write(
            &path,
            r#"{"models":[{"slug":"gpt-example","display_name":"New official name"}]}"#,
        )
        .unwrap();
        assert_eq!(
            display_names(home.path())["gpt-example"],
            "New official name"
        );
        std::fs::write(&path, "partial file {").unwrap();
        assert!(display_names(home.path()).is_empty());
    }
}

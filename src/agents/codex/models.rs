//! Session model names from Codex's own local catalog. No defaults or network calls.
use serde::Deserialize;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

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
    File::open(home.join("models_cache.json"))
        .map(read_names)
        .unwrap_or_default()
}

fn read_names(reader: impl Read) -> HashMap<String, String> {
    // serde_json's streaming parser reads individual bytes. Buffer the bounded
    // file source so every discovery pass does not perform hundreds of thousands
    // of read syscalls while skipping the catalog's large model descriptions.
    serde_json::from_reader::<_, Catalog>(BufReader::new(reader.take(8 * 1024 * 1024)))
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
    fn large_ignored_catalog_fields_do_not_cause_a_file_read_for_every_byte() {
        use std::{
            cell::Cell,
            io::{self, Cursor},
            rc::Rc,
        };
        struct Counted {
            source: Cursor<Vec<u8>>,
            reads: Rc<Cell<usize>>,
        }
        impl Read for Counted {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                self.reads.set(self.reads.get() + 1);
                self.source.read(buffer)
            }
        }
        let json = format!(
            r#"{{"models":[{{"slug":"example","display_name":"Catalog name","description":"{}"}}]}}"#,
            "x".repeat(512 * 1024)
        );
        let reads = Rc::new(Cell::new(0));
        let names = read_names(Counted {
            source: Cursor::new(json.into_bytes()),
            reads: reads.clone(),
        });
        assert_eq!(names["example"], "Catalog name");
        assert!(
            reads.get() < 128,
            "catalog required {} underlying reads",
            reads.get()
        );
    }
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

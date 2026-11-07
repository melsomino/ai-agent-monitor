//! Embedded images emitted by Codex tools. User input images are never forwarded.
use crate::domain::media::Image;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    fs::File,
    hash::{Hash, Hasher},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

pub const MAX_RECORD: usize = 24 * 1024 * 1024;
const MAX_IMAGE: usize = 10 * 1024 * 1024;
const MAX_TOTAL: usize = 20 * 1024 * 1024;

pub fn output_images(value: &Value) -> Vec<Image> {
    let payload = &value["payload"];
    if value["type"] != "response_item"
        || !matches!(
            payload["type"].as_str(),
            Some("function_call_output" | "custom_tool_call_output")
        )
    {
        return vec![];
    }
    let parsed;
    let parts = if let Some(parts) = payload["output"].as_array() {
        parts
    } else if let Some(text) = payload["output"].as_str() {
        if text.len() > MAX_RECORD {
            return vec![];
        }
        parsed = serde_json::from_str::<Value>(text).ok();
        let Some(parts) = parsed.as_ref().and_then(Value::as_array) else {
            return vec![];
        };
        parts
    } else {
        return vec![];
    };
    let mut images = Vec::new();
    let mut total = 0;
    for part in parts {
        if !matches!(
            part["type"].as_str(),
            Some("input_image" | "output_image" | "image")
        ) {
            continue;
        }
        let Some(url) = part["image_url"]
            .as_str()
            .or_else(|| part["image_url"]["url"].as_str())
        else {
            continue;
        };
        let Some((prefix, encoded)) = url.split_once(',') else {
            continue;
        };
        let mime = match prefix {
            "data:image/png;base64" => "image/png",
            "data:image/jpeg;base64" => "image/jpeg",
            "data:image/webp;base64" => "image/webp",
            _ => continue,
        };
        if encoded.len() > MAX_IMAGE.div_ceil(3) * 4 {
            continue;
        }
        let Ok(data) = STANDARD.decode(encoded) else {
            continue;
        };
        if data.len() > MAX_IMAGE || data.is_empty() {
            continue;
        }
        let valid = match mime {
            "image/png" => data.starts_with(b"\x89PNG\r\n\x1a\n"),
            "image/jpeg" => data.starts_with(&[0xff, 0xd8, 0xff]),
            _ => data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP"),
        };
        if !valid {
            continue;
        }
        let mut hash = DefaultHasher::new();
        mime.hash(&mut hash);
        data.hash(&mut hash);
        let id = format!("{:016x}-{}", hash.finish(), data.len());
        if images.iter().any(|i: &Image| i.id == id) {
            continue;
        }
        if images.len() >= 10 || total + data.len() > MAX_TOTAL {
            break;
        }
        total += data.len();
        images.push(Image {
            id,
            mime,
            data: data.into(),
        });
    }
    images
}

#[derive(Default)]
pub struct Cache(HashMap<PathBuf, Entry>);
struct Entry {
    device: u64,
    inode: u64,
    offset: u64,
    images: Vec<Image>,
    length: u64,
    modified: Option<std::time::SystemTime>,
}
impl Cache {
    pub fn retain(&mut self, paths: impl Iterator<Item = PathBuf>) {
        let paths: std::collections::HashSet<_> = paths.collect();
        self.0.retain(|p, _| paths.contains(p));
    }
    pub fn latest(&mut self, path: &Path) -> Vec<Image> {
        let Ok(mut file) = File::open(path) else {
            return vec![];
        };
        let Ok(meta) = file.metadata() else {
            return vec![];
        };
        let valid = self.0.get(path).is_some_and(|e| {
            e.device == meta.dev()
                && e.inode == meta.ino()
                && e.offset <= meta.len()
                && (meta.len() > e.length || meta.modified().ok() == e.modified)
        });
        if !valid {
            let offset = meta.len().saturating_sub(32 * 1024 * 1024);
            self.0.insert(
                path.to_owned(),
                Entry {
                    device: meta.dev(),
                    inode: meta.ino(),
                    offset,
                    images: vec![],
                    length: meta.len(),
                    modified: meta.modified().ok(),
                },
            );
            if offset > 0 && file.seek(SeekFrom::Start(offset)).is_ok() {
                let mut reader = BufReader::new(&mut file);
                if let Ok(skipped) = reader.skip_until(b'\n') {
                    self.0.get_mut(path).unwrap().offset += skipped as u64;
                }
            }
        }
        let entry = self.0.get_mut(path).unwrap();
        if file.seek(SeekFrom::Start(entry.offset)).is_err() {
            return vec![];
        }
        let mut reader = BufReader::new(file);
        loop {
            let mut line = Vec::new();
            let Ok(n) = (&mut reader)
                .take(MAX_RECORD as u64 + 1)
                .read_until(b'\n', &mut line)
            else {
                break;
            };
            if n == 0 {
                break;
            }
            if !line.ends_with(b"\n") {
                if n > MAX_RECORD {
                    let Ok(rest) = reader.skip_until(b'\n') else {
                        break;
                    };
                    if rest > 0 {
                        entry.offset += (n + rest) as u64;
                        continue;
                    }
                }
                break; // Retry an incomplete final record after the next append.
            }
            entry.offset += n as u64;
            if n > MAX_RECORD {
                continue;
            }
            if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                if value["type"] == "response_item"
                    && value["payload"]["type"] == "message"
                    && value["payload"]["role"] == "user"
                {
                    entry.images.clear();
                }
                if value["type"] == "event_msg"
                    && matches!(
                        value["payload"]["type"].as_str(),
                        Some("task_started" | "turn_started")
                    )
                {
                    entry.images.clear();
                }
                let new = output_images(&value);
                for image in new {
                    if entry.images.iter().any(|i| i.id == image.id) {
                        continue;
                    }
                    if entry.images.len() >= 10
                        || entry.images.iter().map(|i| i.data.len()).sum::<usize>()
                            + image.data.len()
                            > MAX_TOTAL
                    {
                        break;
                    }
                    entry.images.push(image);
                }
            }
        }
        entry.length = meta.len();
        entry.modified = meta.modified().ok();
        entry.images.clone()
    }
}

#[cfg(test)]
pub(crate) fn fixture() -> Value {
    serde_json::json!({"type":"response_item","payload":{"type":"custom_tool_call_output","output":[{"type":"input_image","image_url":"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jf0kAAAAASUVORK5CYII="}]}})
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn accepts_embedded_tool_images_but_not_user_images_or_external_paths() {
        let image = output_images(&fixture()).remove(0);
        assert_eq!(image.mime, "image/png");
        assert!(image.data.starts_with(b"\x89PNG\r\n\x1a\n"));
        let mut value = fixture();
        let duplicate = value["payload"]["output"][0].clone();
        value["payload"]["output"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert_eq!(output_images(&value).len(), 1);
        let output = value["payload"]["output"].to_string();
        value["payload"]["output"] = Value::String(output);
        assert_eq!(output_images(&value)[0], image);
        value["payload"]["type"] = "message".into();
        value["payload"]["role"] = "user".into();
        assert!(output_images(&value).is_empty());
        for url in [
            "/tmp/private.png",
            "https://example.com/private.png",
            "data:image/png;base64,not valid",
            "data:image/png;base64,aGVsbG8=",
        ] {
            let mut value = fixture();
            value["payload"]["output"][0]["image_url"] = url.into();
            assert!(output_images(&value).is_empty());
        }
    }
    #[test]
    fn cache_handles_appends_partial_records_turn_changes_and_truncation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("rollout.jsonl");
        let record = format!("{}\n", fixture());
        std::fs::write(&path, &record).unwrap();
        let mut cache = Cache::default();
        let first = cache.latest(&path);
        assert_eq!(first.len(), 1);
        assert!(std::sync::Arc::ptr_eq(
            &first[0].data,
            &cache.latest(&path)[0].data
        ));
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"{\"type\":\"response_item\",").unwrap();
        assert_eq!(cache.latest(&path).len(), 1);
        file.write_all(b"\"payload\":{\"type\":\"message\",\"role\":\"user\"}}\n")
            .unwrap();
        assert!(cache.latest(&path).is_empty());
        file.write_all(record.as_bytes()).unwrap();
        assert_eq!(cache.latest(&path).len(), 1);
        std::fs::write(&path, "").unwrap();
        assert!(cache.latest(&path).is_empty());
    }
}

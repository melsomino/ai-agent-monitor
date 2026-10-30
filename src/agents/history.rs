//! Read earlier agent messages on demand, with append-stable transcript cursors.
use anyhow::{Context, Result};
use serde_json::Value;
use std::{
    collections::hash_map::DefaultHasher,
    fs::File,
    hash::{Hash, Hasher},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::Path,
};

pub use crate::domain::history::Message;
pub struct Record {
    pub text: String,
    pub kind: u8,
}
fn hash(text: &str) -> u64 {
    let mut hash = DefaultHasher::new();
    text.hash(&mut hash);
    hash.finish()
}
fn matches_anchor(text: &str, anchor: &str) -> bool {
    text.trim_end() == anchor.trim_end()
        || crate::agents::tracking::text(text).trim_end() == anchor.trim_end()
}
fn cursor(metadata: &std::fs::Metadata, offset: u64, record: &Record) -> String {
    format!(
        "{}:{}:{}:{}:{}",
        metadata.dev(),
        metadata.ino(),
        offset,
        record.kind,
        hash(&record.text)
    )
}
fn decode(cursor: &str, metadata: &std::fs::Metadata) -> Result<(u64, u8, u64)> {
    let parts = cursor
        .split(':')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    anyhow::ensure!(
        parts.len() == 5 && parts[0] == metadata.dev() && parts[1] == metadata.ino(),
        "Transcript changed; reopen the session to reload history"
    );
    Ok((parts[2], parts[3] as u8, parts[4]))
}

/// Reads backward in chunks; large tool records are skipped without loading them.
pub fn jsonl(
    path: &Path,
    before: Option<&str>,
    anchor: Option<&str>,
    parse: fn(&Value) -> Option<Record>,
) -> Result<Option<Message>> {
    let mut file = File::open(path).context("No session transcript available")?;
    let metadata = file.metadata()?;
    let mut position = metadata.len();
    let mut previous = None;
    let mut found_anchor = anchor.is_none_or(str::is_empty);
    if let Some(before) = before {
        let (offset, kind, hash) = decode(before, &metadata)?;
        anyhow::ensure!(offset < metadata.len(), "Transcript was truncated");
        file.seek(SeekFrom::Start(offset))?;
        let mut line = Vec::new();
        BufReader::new((&mut file).take(2 * 1024 * 1024 + 1)).read_until(b'\n', &mut line)?;
        let record = serde_json::from_slice::<Value>(&line)
            .ok()
            .and_then(|value| parse(&value));
        anyhow::ensure!(
            record
                .as_ref()
                .is_some_and(|record| record.kind == kind && self::hash(&record.text) == hash),
            "Transcript changed; reopen the session to reload history"
        );
        position = offset;
        previous = Some((kind, hash));
        found_anchor = true;
    }
    let mut end = position;
    while position > 0 {
        let start = position.saturating_sub(65536);
        file.seek(SeekFrom::Start(start))?;
        let mut chunk = vec![0; (position - start) as usize];
        file.read_exact(&mut chunk)?;
        for index in (0..chunk.len()).rev() {
            if chunk[index] != b'\n' {
                continue;
            }
            let line_start = start + index as u64 + 1;
            if let Some(message) = read_record(&mut file, line_start, end, parse)?
                && let Some(result) = select(
                    &metadata,
                    line_start,
                    message,
                    &mut previous,
                    &mut found_anchor,
                    anchor,
                )
            {
                return Ok(Some(result));
            }
            end = line_start - 1;
        }
        position = start;
    }
    if let Some(message) = read_record(&mut file, 0, end, parse)?
        && let Some(result) = select(
            &metadata,
            0,
            message,
            &mut previous,
            &mut found_anchor,
            anchor,
        )
    {
        return Ok(Some(result));
    }
    anyhow::ensure!(
        found_anchor,
        "Current message is not available in the transcript yet"
    );
    Ok(None)
}
fn read_record(
    file: &mut File,
    start: u64,
    end: u64,
    parse: fn(&Value) -> Option<Record>,
) -> Result<Option<Record>> {
    if end <= start || end - start > 2 * 1024 * 1024 {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(start))?;
    let mut line = vec![0; (end - start) as usize];
    file.read_exact(&mut line)?;
    Ok(serde_json::from_slice::<Value>(&line)
        .ok()
        .and_then(|value| parse(&value))
        .filter(|record| !record.text.trim().is_empty()))
}
fn select(
    metadata: &std::fs::Metadata,
    offset: u64,
    record: Record,
    previous: &mut Option<(u8, u64)>,
    found: &mut bool,
    anchor: Option<&str>,
) -> Option<Message> {
    let fingerprint = hash(&record.text);
    if previous.is_some_and(|(kind, hash)| kind != record.kind && hash == fingerprint) {
        return None;
    }
    *previous = Some((record.kind, fingerprint));
    if !*found {
        if anchor.is_some_and(|anchor| matches_anchor(&record.text, anchor)) {
            *found = true;
        }
        return None;
    }
    Some(Message {
        cursor: cursor(metadata, offset, &record),
        text: record.text.trim_end().to_owned(),
    })
}

pub fn codex(value: &Value) -> Option<Record> {
    let payload = &value["payload"];
    if payload["channel"] == "analysis" || payload["phase"] == "analysis" {
        return None;
    }
    if value["type"] == "event_msg" && payload["type"] == "agent_message" {
        return Some(Record {
            text: payload["message"].as_str()?.to_owned(),
            kind: 0,
        });
    }
    if value["type"] == "response_item"
        && payload["type"] == "message"
        && payload["role"] == "assistant"
    {
        return Some(Record {
            text: payload["content"]
                .as_array()?
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            kind: 1,
        });
    }
    None
}
pub fn claude(value: &Value) -> Option<Record> {
    if value["isSidechain"] == true || value["type"] != "assistant" {
        return None;
    }
    Some(Record {
        text: value["message"]["content"]
            .as_array()?
            .iter()
            .filter(|part| part["type"] == "text")
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        kind: 2,
    })
}
pub fn cline(path: &Path, before: Option<&str>, anchor: Option<&str>) -> Result<Option<Message>> {
    let metadata = std::fs::metadata(path)?;
    anyhow::ensure!(
        metadata.len() <= 8 * 1024 * 1024,
        "Cline task transcript is too large"
    );
    let messages: Value = serde_json::from_reader(File::open(path)?)?;
    let messages = messages
        .as_array()
        .context("Invalid Cline task transcript")?;
    let parse = |value: &Value| -> Option<Record> {
        (value["type"] == "say"
            && matches!(value["say"].as_str(), Some("text" | "completion_result")))
        .then(|| Record {
            text: value["text"].as_str().unwrap_or("").to_owned(),
            kind: 3,
        })
    };
    let mut end = messages.len();
    let mut found = anchor.is_none_or(str::is_empty);
    if let Some(before) = before {
        let (index, _, fingerprint) = decode(before, &metadata)?;
        let record = messages.get(index as usize).and_then(parse);
        anyhow::ensure!(
            record.is_some_and(|record| hash(&record.text) == fingerprint),
            "Cline task history changed"
        );
        end = index as usize;
        found = true;
    }
    for index in (0..end).rev() {
        let Some(record) = parse(&messages[index]).filter(|record| !record.text.trim().is_empty())
        else {
            continue;
        };
        if !found {
            if anchor.is_some_and(|anchor| matches_anchor(&record.text, anchor)) {
                found = true;
            }
            continue;
        }
        return Ok(Some(Message {
            cursor: cursor(&metadata, index as u64, &record),
            text: record.text.trim_end().into(),
        }));
    }
    anyhow::ensure!(
        found,
        "Current message is not available in the transcript yet"
    );
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, io::Write};
    fn pair(text: &str) -> String {
        format!(
            "{}\n{}\n",
            json!({"type":"event_msg","payload":{"type":"agent_message","message":text}}),
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":text}]}})
        )
    }
    #[test]
    fn previous_messages_are_paginated_without_mirrors_and_survive_appends() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rollout.jsonl");
        fs::write(
            &path,
            pair("Old") + &pair("Same") + &pair("Same") + &pair("Latest"),
        )
        .unwrap();
        let first = jsonl(&path, None, Some("Latest"), codex).unwrap().unwrap();
        assert_eq!(first.text, "Same");
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(pair("New arrival").as_bytes())
            .unwrap();
        let second = jsonl(&path, Some(&first.cursor), None, codex)
            .unwrap()
            .unwrap();
        assert_eq!(second.text, "Same");
        let third = jsonl(&path, Some(&second.cursor), None, codex)
            .unwrap()
            .unwrap();
        assert_eq!(third.text, "Old");
        assert!(
            jsonl(&path, Some(&third.cursor), None, codex)
                .unwrap()
                .is_none()
        );
        fs::write(&path, pair("Replacement")).unwrap();
        assert!(jsonl(&path, Some(&first.cursor), None, codex).is_err());
    }
    #[test]
    fn large_tool_records_partial_lines_and_unicode_do_not_hide_agent_messages() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rollout.jsonl");
        let tool = json!({"type":"response_item","payload":{"type":"function_call_output","output":"x".repeat(2*1024*1024+10)}}).to_string();
        fs::write(
            &path,
            pair("Ранее 🌍") + &tool + "\n" + &pair("Latest") + "{partial",
        )
        .unwrap();
        assert_eq!(
            jsonl(&path, None, Some("Latest"), codex)
                .unwrap()
                .unwrap()
                .text,
            "Ранее 🌍"
        );
        assert!(jsonl(&path, None, Some("Not flushed yet"), codex).is_err());
        assert!(codex(&json!({"type":"response_item","payload":{"type":"message","role":"assistant","channel":"analysis","content":[{"text":"internal"}]}})).is_none());
    }
    #[test]
    fn claude_and_cline_history_only_include_agent_text() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("claude.jsonl");
        let message = |text| {
            json!({"type":"assistant","message":{"content":[{"type":"text","text":text},{"type":"tool_use","text":"not a message"}]}}).to_string()+"\n"
        };
        fs::write(&path, message("Earlier") + &json!({"type":"assistant","isSidechain":true,"message":{"content":[{"type":"text","text":"side task"}]}}).to_string()+"\n"+&message("Latest")).unwrap();
        assert_eq!(
            jsonl(&path, None, Some("Latest"), claude)
                .unwrap()
                .unwrap()
                .text,
            "Earlier"
        );
        let path = temp.path().join("ui_messages.json");
        fs::write(&path,json!([{"type":"say","say":"text","text":"Earlier"},{"type":"say","say":"api_req_started","text":"hidden"},{"type":"say","say":"completion_result","text":"Latest"}]).to_string()).unwrap();
        let previous = cline(&path, None, Some("Latest")).unwrap().unwrap();
        assert_eq!(previous.text, "Earlier");
        assert!(
            cline(&path, Some(&previous.cursor), None)
                .unwrap()
                .is_none()
        );
    }
}

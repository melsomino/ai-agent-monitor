//! Incremental read-only projection of append-only Codex JSONL logs.
//! Keep byte cursors, bounded activities and question state, not raw history.
use super::RolloutReader;
use crate::domain::session::{Activity, Session};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    io::{self, BufReader, Cursor, Read, Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::SystemTime,
};

const TAIL: u64 = 4 * 1024 * 1024;
const BLOCK: usize = 64 * 1024;
const MAX_RECORD: u64 = 24 * 1024 * 1024;

#[derive(Clone, PartialEq, Eq)]
struct Stamp {
    device: u64,
    inode: u64,
    size: u64,
    modified: Option<SystemTime>,
    changed: (i64, i64),
}
impl Stamp {
    fn read(path: &Path) -> io::Result<Self> {
        let meta = fs::metadata(path)?;
        if !meta.is_file() {
            return Err(io::Error::other("rollout is not a file"));
        }
        Ok(Self {
            device: meta.dev(),
            inode: meta.ino(),
            size: meta.len(),
            modified: meta.modified().ok(),
            changed: (meta.ctime(), meta.ctime_nsec()),
        })
    }
    fn same_file(&self, other: &Self) -> bool {
        self.device == other.device && self.inode == other.inode
    }
}

/// Only an unfinished record is retained. Large records spill to an unlinked,
/// private temporary file; polling never rereads their already received prefix.
enum Pending {
    Memory(Vec<u8>),
    Disk(File, u64),
}
impl Default for Pending {
    fn default() -> Self {
        Self::Memory(Vec::new())
    }
}
impl Pending {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if let Self::Memory(buffer) = self {
            if buffer.len() + bytes.len() <= BLOCK {
                buffer.extend_from_slice(bytes);
                return Ok(());
            }
            let mut file = tempfile::tempfile()?;
            file.write_all(buffer)?;
            *self = Self::Disk(file, buffer.len() as u64);
        }
        if let Self::Disk(file, size) = self {
            file.write_all(bytes)?;
            *size += bytes.len() as u64;
        }
        Ok(())
    }
    fn consume(
        self,
        parser: &mut RolloutReader,
        session: &mut Session,
        offset: u64,
    ) -> io::Result<()> {
        match self {
            Self::Memory(bytes) => decode(Cursor::new(bytes), parser, session, offset),
            Self::Disk(mut file, size) => {
                file.flush()?;
                file.seek(SeekFrom::Start(0))?;
                if size <= MAX_RECORD {
                    decode(BufReader::new(file), parser, session, offset)
                } else {
                    decode_header(BufReader::new(file), parser, session, offset)
                }
            }
        }
    }
}

#[derive(Default, Deserialize)]
struct Header {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    timestamp: Value,
    #[serde(default)]
    payload: Payload,
}
#[derive(Default, Deserialize)]
struct Payload {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    id: Option<String>,
}
fn decode_header(
    reader: impl Read,
    parser: &mut RolloutReader,
    session: &mut Session,
    offset: u64,
) -> io::Result<()> {
    if let Some(header) = parsed(serde_json::from_reader::<_, Header>(reader))?
        && (header.kind == "event_msg"
            || (header.kind == "response_item"
                && matches!(
                    header.payload.kind.as_str(),
                    "function_call_output" | "custom_tool_call_output"
                )))
    {
        parser.consume(session, &json!({"type":header.kind,"timestamp":header.timestamp,
            "payload":{"type":header.payload.kind,"call_id":header.payload.call_id,"id":header.payload.id}}), offset);
    }
    Ok(())
}
fn decode(
    reader: impl Read,
    parser: &mut RolloutReader,
    session: &mut Session,
    offset: u64,
) -> io::Result<()> {
    if let Some(value) = parsed(serde_json::from_reader::<_, Value>(reader))? {
        parser.consume(session, &value, offset);
    }
    Ok(())
}
fn parsed<T>(result: serde_json::Result<T>) -> io::Result<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.is_io() => Err(io::Error::other(error)),
        Err(_) => Ok(None), // Skip a malformed completed record, not later records.
    }
}

struct Entry {
    stamp: Stamp,
    session: String,
    output: Session,
    parser: RolloutReader,
    scanned: u64,
    committed: u64,
    pending: Pending,
    head: Vec<u8>,
    boundary: Vec<u8>,
    #[cfg(test)]
    read_bytes: u64,
}
#[derive(Default)]
pub(super) struct Cache(HashMap<PathBuf, Entry>);
impl Cache {
    pub(super) fn retain<'a>(&mut self, paths: impl Iterator<Item = &'a PathBuf>) {
        let paths: HashSet<_> = paths.collect();
        self.0.retain(|path, _| paths.contains(path));
    }
    pub(super) fn enrich(&mut self, session: &mut Session, path: &Path) -> io::Result<()> {
        let stamp = match Stamp::read(path) {
            Ok(stamp) => stamp,
            Err(error) => {
                self.0.remove(path);
                return Err(error);
            }
        };
        if let Some(entry) = self.0.get_mut(path)
            && entry.session == session.key()
            && entry.stamp == stamp
        {
            entry.project(session);
            return Ok(());
        }
        let mut file = File::open(path)?;
        let mut entry = match self.0.remove(path) {
            Some(entry)
                if entry.session == session.key() && entry.appended(&mut file, &stamp)? =>
            {
                entry
            }
            _ => Entry::initial(session, &mut file, &stamp)?,
        };
        entry.output.updated_at = session.updated_at;
        entry.advance(&mut file, stamp.size)?;
        // Appends racing this read are picked up next time. Replacement or
        // truncation during a read must not commit a cursor for another file.
        let after = Stamp::read(path)?;
        if !stamp.same_file(&after)
            || after.size < stamp.size
            || guard(&mut file, entry.head.len() as u64)? != entry.head
            || guard(&mut file, entry.scanned)? != entry.boundary
        {
            return Err(io::Error::other("rollout changed during read"));
        }
        entry.stamp = stamp;
        entry.parser.finish(&mut entry.output);
        entry.project(session);
        self.0.insert(path.to_owned(), entry);
        Ok(())
    }
}
impl Entry {
    fn appended(&self, file: &mut File, next: &Stamp) -> io::Result<bool> {
        // Codex logs append in place. Same-size edits, truncation and inode
        // changes reset the reader. Head/cursor guards detect ordinary copy-
        // truncate/regrow; arbitrary edits preserving both guards and growing
        // the file cannot be detected without hashing the complete prefix.
        Ok(self.stamp.same_file(next)
            && next.size > self.stamp.size
            && guard(file, self.head.len() as u64)? == self.head
            && guard(file, self.scanned)? == self.boundary)
    }
    fn initial(base: &Session, file: &mut File, stamp: &Stamp) -> io::Result<Self> {
        let mut output = base.clone();
        file.seek(SeekFrom::Start(0))?;
        let mut head = Vec::new();
        (&mut *file).take(131072).read_to_end(&mut head)?;
        if let Some(line) = head.split(|byte| *byte == b'\n').next()
            && let Ok(value) = serde_json::from_slice::<Value>(line)
        {
            output.client = super::rollout_client(&value);
        }
        let end = last_complete(file, stamp.size)?;
        let start = end.saturating_sub(TAIL);
        let starts_at_boundary = if start == 0 {
            true
        } else {
            file.seek(SeekFrom::Start(start - 1))?;
            let mut byte = [0];
            file.read_exact(&mut byte)?;
            byte[0] == b'\n'
        };
        file.seek(SeekFrom::Start(start))?;
        let mut bytes = Vec::new();
        (&mut *file).take(end - start).read_to_end(&mut bytes)?;
        if bytes.len() as u64 != end - start {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let skip = if !starts_at_boundary {
            bytes
                .iter()
                .position(|b| *b == b'\n')
                .map_or(bytes.len(), |i| i + 1)
        } else {
            0
        };
        let mut parser = RolloutReader::default();
        parser.parse(&mut output, &bytes[skip..], start + skip as u64);
        if parser.activity == Activity::Unknown && end > 0 {
            parser.activity =
                super::last_activity_before(file, end, 0)?.unwrap_or(Activity::Unknown);
        }
        let head = guard(file, stamp.size.min(128))?;
        let boundary = guard(file, end)?;
        Ok(Self {
            stamp: stamp.clone(),
            session: base.key(),
            output,
            parser,
            scanned: end,
            committed: end,
            pending: Pending::default(),
            head,
            boundary,
            #[cfg(test)]
            read_bytes: bytes.len() as u64,
        })
    }
    fn advance(&mut self, file: &mut File, end: u64) -> io::Result<()> {
        file.seek(SeekFrom::Start(self.scanned))?;
        let mut buffer = [0u8; BLOCK];
        while self.scanned < end {
            let limit = (end - self.scanned).min(BLOCK as u64) as usize;
            let count = file.read(&mut buffer[..limit])?;
            if count == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            #[cfg(test)]
            {
                self.read_bytes += count as u64;
            }
            let mut bytes = &buffer[..count];
            while !bytes.is_empty() {
                let size = bytes
                    .iter()
                    .position(|b| *b == b'\n')
                    .map_or(bytes.len(), |i| i + 1);
                let piece = &bytes[..size];
                self.pending.append(piece)?;
                self.scanned += size as u64;
                if piece.ends_with(b"\n") {
                    std::mem::take(&mut self.pending).consume(
                        &mut self.parser,
                        &mut self.output,
                        self.committed,
                    )?;
                    self.committed = self.scanned;
                }
                bytes = &bytes[size..];
            }
        }
        self.head = guard(file, end.min(128))?;
        self.boundary = guard(file, end)?;
        Ok(())
    }
    fn project(&mut self, base: &mut Session) {
        self.output.model = base.model.clone();
        self.output.reasoning_effort = base.reasoning_effort.clone();
        self.parser.finish(&mut self.output);
        base.client = self.output.client;
        base.activity = self.output.activity;
        base.activities = self.output.activities.clone();
        base.model = self.output.model.clone();
        base.reasoning_effort = self.output.reasoning_effort.clone();
    }
}
fn guard(file: &mut File, end: u64) -> io::Result<Vec<u8>> {
    let start = end.saturating_sub(128);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0; (end - start) as usize];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}
fn last_complete(file: &mut File, mut end: u64) -> io::Result<u64> {
    let mut bytes = [0; BLOCK];
    while end > 0 {
        let start = end.saturating_sub(BLOCK as u64);
        file.seek(SeekFrom::Start(start))?;
        let size = (end - start) as usize;
        file.read_exact(&mut bytes[..size])?;
        if let Some(index) = bytes[..size].iter().rposition(|b| *b == b'\n') {
            return Ok(start + index as u64 + 1);
        }
        end = start;
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        session::Client,
        session_activity::{ActionStatus, MessageRole, RequestStatus, SessionActivity},
    };
    use std::fs::OpenOptions;

    fn base() -> Session {
        Session {
            agent: "codex",
            id: "example".into(),
            title: "Database title".into(),
            cwd: PathBuf::new(),
            workspace: None,
            git_branch: None,
            model: None,
            model_display_name: None,
            reasoning_effort: None,
            updated_at: 0,
            client: Client::Unknown,
            activity: Activity::Unknown,
            target: None,
            connection: Default::default(),
            activities: Default::default(),
        }
    }
    fn line(value: Value) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(&value).unwrap();
        bytes.push(b'\n');
        bytes
    }
    fn append(path: &Path, bytes: &[u8]) {
        OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(bytes)
            .unwrap();
    }
    fn task(kind: &str) -> Vec<u8> {
        line(json!({"type":"event_msg","payload":{"type":kind}}))
    }
    fn message(text: &str) -> Vec<u8> {
        line(
            json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"text":text}]}}),
        )
    }
    fn load(cache: &mut Cache, path: &Path) -> Session {
        let mut session = base();
        cache.enrich(&mut session, path).unwrap();
        session
    }
    fn file() -> (tempfile::TempDir, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("rollout.jsonl");
        fs::write(
            &path,
            line(json!({"type":"session_meta","payload":{"source":"vscode"}})),
        )
        .unwrap();
        (directory, path)
    }
    fn agents(session: &Session) -> Vec<&str> {
        session
            .activities
            .iter()
            .filter_map(|a| match a {
                SessionActivity::Message(m) if m.role == MessageRole::Agent => {
                    Some(m.text.as_str())
                }
                _ => None,
            })
            .collect()
    }
    #[test]
    fn appends_read_only_new_bytes_and_preserve_offset_identity_and_database_updates() {
        let (_directory, path) = file();
        append(&path, &task("task_started"));
        append(
            &path,
            &line(
                json!({"type":"response_item","payload":{"type":"function_call_output","output":"x".repeat(TAIL as usize+100)}}),
            ),
        );
        append(&path, &message("Repeated"));
        let mut cache = Cache::default();
        let first = load(&mut cache, &path);
        assert_eq!(first.activity, Activity::Working);
        assert_eq!(first.client, Client::VSCode);
        let before = cache.0[&path].read_bytes;
        let source_offset = fs::metadata(&path).unwrap().len();
        let mut added = vec![0xff, b'\n']; // A bad record must not shift later byte IDs.
        added.extend(message("Repeated"));
        append(&path, &added);
        let next = load(&mut cache, &path);
        assert!(next.activities.iter().any(
            |activity| activity.id() == format!("rollout-message:offset:{}", source_offset + 2)
        ));
        assert_eq!(
            cache.0[&path].read_bytes - before,
            added.len() as u64,
            "must not replay the 4 MiB prefix"
        );
        assert_eq!(agents(&next), ["Repeated", "Repeated"]);
        let ids: Vec<_> = next
            .activities
            .iter()
            .filter(|a| matches!(a, SessionActivity::Message(_)))
            .map(|a| a.id())
            .collect();
        assert_ne!(ids[0], ids[1]);
        let before = cache.0[&path].read_bytes;
        let mut changed = base();
        changed.title = "Renamed in database".into();
        changed.model = Some("database-model".into());
        cache.enrich(&mut changed, &path).unwrap();
        assert_eq!(changed.title, "Renamed in database");
        assert_eq!(changed.model.as_deref(), Some("database-model"));
        assert_eq!(agents(&changed), agents(&next));
        assert_eq!(cache.0[&path].read_bytes, before);
    }
    #[test]
    fn partial_utf8_and_valid_json_wait_for_newline_without_rereading_the_prefix() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("partial.jsonl");
        fs::write(&path, []).unwrap();
        let mut cache = Cache::default();
        load(&mut cache, &path);
        let bytes = message("Hello 🙂");
        let split = bytes.windows(4).position(|w| w == "🙂".as_bytes()).unwrap() + 2;
        append(&path, &bytes[..split]);
        assert!(agents(&load(&mut cache, &path)).is_empty());
        assert_eq!(cache.0[&path].committed, 0);
        assert_eq!(cache.0[&path].scanned, split as u64);
        append(&path, &bytes[split..bytes.len() - 1]);
        assert!(agents(&load(&mut cache, &path)).is_empty());
        append(&path, b"\n");
        let next = load(&mut cache, &path);
        assert_eq!(agents(&next), ["Hello 🙂"]);
        assert_eq!(cache.0[&path].read_bytes, bytes.len() as u64);
        for _ in 0..3 {
            assert_eq!(load(&mut cache, &path).activities, next.activities);
        }
        let incomplete = task("task_started");
        fs::write(&path, &incomplete[..incomplete.len() - 1]).unwrap();
        assert_eq!(load(&mut cache, &path).activity, Activity::Unknown);
        append(&path, b"\n");
        assert_eq!(load(&mut cache, &path).activity, Activity::Working);
    }
    fn question(id: &str) -> Vec<u8> {
        line(
            json!({"type":"response_item","payload":{"type":"function_call","name":"request_user_input_async","call_id":id,
            "arguments":json!({"questions":[{"title":"Which checks?"}]}).to_string()}}),
        )
    }
    fn status(session: &Session, id: &str) -> RequestStatus {
        session
            .activities
            .iter()
            .find_map(|a| match a {
                SessionActivity::Request(r) if r.id == format!("rollout-question:{id}") => {
                    Some(r.status)
                }
                _ => None,
            })
            .unwrap()
    }
    #[test]
    fn questions_and_replies_survive_poll_boundaries_and_appends_larger_than_the_bootstrap_tail() {
        let (_directory, path) = file();
        append(&path, &task("task_started"));
        let mut cache = Cache::default();
        load(&mut cache, &path);
        append(&path, &question("one"));
        let pending = load(&mut cache, &path);
        assert_eq!(status(&pending, "one"), RequestStatus::Pending);
        append(&path, &question("two"));
        load(&mut cache, &path);
        append(
            &path,
            &line(
                json!({"type":"response_item","payload":{"type":"function_call_output","output":"x".repeat(TAIL as usize+100)}}),
            ),
        );
        let reply = format!(
            "<send_user_message_question_reply>\n{}\n</send_user_message_question_reply>",
            json!([{"questionItemId":json!(["request_user_input_async","one",0]).to_string(),"answer":"All"}])
        );
        append(
            &path,
            &line(
                json!({"type":"response_item","timestamp":"2026-11-16T10:00:00Z","payload":{"type":"message","role":"user","content":[{"text":reply}]}}),
            ),
        );
        let next = load(&mut cache, &path);
        assert_eq!(status(&next, "one"), RequestStatus::Answered);
        assert_eq!(status(&next, "two"), RequestStatus::Pending);
        assert_eq!(next.activity, Activity::Confirmation);
        assert_eq!(load(&mut cache, &path).activities, next.activities);
        append(
            &path,
            &line(
                json!({"type":"event_msg","payload":{"type":"user_message","message":"Continue instead"}}),
            ),
        );
        let next = load(&mut cache, &path);
        assert_eq!(status(&next, "two"), RequestStatus::Superseded);
        assert_eq!(next.activity, Activity::Working);
        append(&path, &task("task_complete"));
        assert_eq!(load(&mut cache, &path).activity, Activity::Idle);
    }
    #[test]
    fn oversized_partial_output_spills_and_completes_the_original_action_without_reading_its_prefix_again()
     {
        let (_directory, path) = file();
        append(&path, &task("task_started"));
        append(
            &path,
            &line(
                json!({"type":"response_item","payload":{"type":"function_call","call_id":"compile","name":"exec_command","arguments":"{}"}}),
            ),
        );
        let mut cache = Cache::default();
        load(&mut cache, &path);
        append(&path,b"{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"call_id\":\"compile\",\"output\":\"");
        let block = vec![b'x'; BLOCK];
        let mut writer = OpenOptions::new().append(true).open(&path).unwrap();
        for _ in 0..(MAX_RECORD / BLOCK as u64 + 1) {
            writer.write_all(&block).unwrap();
        }
        writer.flush().unwrap();
        let working = load(&mut cache, &path);
        assert_eq!(working.activity, Activity::Working);
        assert!(matches!(cache.0[&path].pending, Pending::Disk(..)));
        assert!(working.activities.iter().any(|a|matches!(a,SessionActivity::Action(a) if a.id=="rollout-action:compile" && a.status==ActionStatus::Running)));
        let before = cache.0[&path].read_bytes;
        append(&path, b"\"}}\n");
        append(&path, &task("task_complete"));
        let idle = load(&mut cache, &path);
        assert_eq!(idle.activity, Activity::Idle);
        assert!(idle.activities.iter().any(|a|matches!(a,SessionActivity::Action(a) if a.id=="rollout-action:compile" && a.status==ActionStatus::Completed)));
        assert_eq!(
            cache.0[&path].read_bytes - before,
            b"\"}}\n".len() as u64 + task("task_complete").len() as u64
        );
        assert!(matches!(cache.0[&path].pending, Pending::Memory(_)));
    }
    #[test]
    fn canonical_messages_replace_legacy_mirrors_across_batches_and_model_context_is_retained() {
        let (_directory, path) = file();
        let mut cache = Cache::default();
        append(
            &path,
            &line(json!({"type":"event_msg","payload":{"type":"agent_message","message":"Same"}})),
        );
        assert_eq!(agents(&load(&mut cache, &path)), ["Same"]);
        append(&path, &message("Same"));
        append(
            &path,
            &line(json!({"type":"event_msg","payload":{"type":"agent_message","message":"Same"}})),
        );
        let next = load(&mut cache, &path);
        assert_eq!(agents(&next), ["Same"]);
        append(
            &path,
            &line(json!({"type":"turn_context","payload":{"model":"model-one","effort":"high"}})),
        );
        assert_eq!(load(&mut cache, &path).model.as_deref(), Some("model-one"));
        append(
            &path,
            &line(json!({"type":"turn_context","payload":{"model":"model-two","effort":"medium"}})),
        );
        let next = load(&mut cache, &path);
        assert_eq!(next.model.as_deref(), Some("model-two"));
        assert_eq!(next.reasoning_effort.as_deref(), Some("medium"));
    }
    #[test]
    fn rewrites_truncation_rotation_and_removal_reset_cursors_and_discard_old_content() {
        let (_directory, path) = file();
        append(&path, &message("Old"));
        let mut cache = Cache::default();
        load(&mut cache, &path);
        let mut bytes = line(json!({"type":"session_meta","payload":{"source":"vscode"}}));
        bytes.extend(message("New"));
        let size = fs::metadata(&path).unwrap().len();
        fs::write(&path, &bytes).unwrap();
        assert_eq!(size, bytes.len() as u64);
        assert_eq!(agents(&load(&mut cache, &path)), ["New"]);
        fs::write(&path, task("task_complete")).unwrap();
        let next = load(&mut cache, &path);
        assert_eq!(next.activity, Activity::Idle);
        assert!(agents(&next).is_empty());
        let replacement = path.with_extension("replacement");
        fs::write(&replacement, message("Replaced")).unwrap();
        fs::rename(replacement, &path).unwrap();
        assert_eq!(agents(&load(&mut cache, &path)), ["Replaced"]);
        fs::remove_file(&path).unwrap();
        assert!(cache.enrich(&mut base(), &path).is_err());
        assert!(cache.0.is_empty());
        fs::write(&path, []).unwrap();
        load(&mut cache, &path);
        append(
            &path,
            &line(json!({"type":"session_meta","payload":{"originator":"codex-tui"}})),
        );
        assert_eq!(load(&mut cache, &path).client, Client::Terminal);
        cache.retain(std::iter::empty());
        assert!(cache.0.is_empty());
    }
}

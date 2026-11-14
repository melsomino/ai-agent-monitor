//! Project owner frames while decoding, before allocating ignored payloads.
//!
//! IgnoredAny skips tool output/images directly from the reader. Fields whose
//! discriminator arrives later use a bounded memory spool, spilling to an unlinked
//! temporary file. Array positions remain stable for owner patches.
use anyhow::Result;
use serde::Deserialize;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use std::{
    cell::{Cell, RefCell},
    fmt,
    fs::File,
    io::{self, BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write},
};

const BUFFER: usize = 64 * 1024;

// Capture only a deferred field, never a frame. No pathname or private payload
// survives the reader. In the ordinary owner field order this is not used.
enum Spool {
    Memory(Vec<u8>),
    Disk(BufWriter<File>),
}
impl Spool {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if let Self::Memory(buffer) = self {
            if buffer.len() + bytes.len() <= BUFFER {
                buffer.extend_from_slice(bytes);
                return Ok(());
            }
            let mut file = BufWriter::with_capacity(BUFFER, tempfile::tempfile()?);
            file.write_all(buffer)?;
            *self = Self::Disk(file);
        }
        match self {
            Self::Disk(file) => file.write_all(bytes),
            Self::Memory(_) => unreachable!(),
        }
    }
    fn reader(self) -> io::Result<Box<dyn Read>> {
        match self {
            Self::Memory(bytes) => Ok(Box::new(Cursor::new(bytes))),
            Self::Disk(mut file) => {
                file.flush()?;
                let mut file = file.into_inner().map_err(|error| error.into_error())?;
                file.seek(SeekFrom::Start(0))?;
                Ok(Box::new(file))
            }
        }
    }
}
#[derive(Default)]
struct Recorder {
    active: Cell<bool>,
    spool: RefCell<Option<Spool>>,
}
struct RecordingRead<'a, R> {
    inner: R,
    recorder: &'a Recorder,
}
impl<R: Read> Read for RecordingRead<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let size = self.inner.read(bytes)?;
        if self.recorder.active.get() {
            self.recorder
                .spool
                .borrow_mut()
                .as_mut()
                .expect("active capture")
                .append(&bytes[..size])?;
        }
        Ok(size)
    }
}
struct Capture<'a>(&'a Recorder);
impl<'de> DeserializeSeed<'de> for Capture<'_> {
    type Value = Spool;
    fn deserialize<D: de::Deserializer<'de>>(
        self,
        decoder: D,
    ) -> std::result::Result<Spool, D::Error> {
        assert!(!self.0.active.replace(true), "nested raw capture");
        *self.0.spool.borrow_mut() = Some(Spool::Memory(Vec::new()));
        // MapAccess calls this seed immediately after consuming the colon. The
        // recording starts before the first byte of the JSON value is peeked.
        let result = IgnoredAny::deserialize(decoder);
        self.0.active.set(false);
        let spool = self.0.spool.borrow_mut().take().expect("captured value");
        result?;
        Ok(spool)
    }
}

#[derive(Clone, Copy)]
enum Shape {
    All,
    Brief,
    Envelope,
    Params,
    Change,
    Patches,
    Patch,
    State,
    Turns,
    Turn,
    TurnHistory,
    History,
    Entities,
    Items,
    Item,
    Input,
    Part,
}

#[cfg(test)]
pub(super) fn decode(bytes: &[u8]) -> Result<Value> {
    decode_reader(Cursor::new(bytes))
}

pub(super) fn decode_reader(reader: impl Read) -> Result<Value> {
    decode_shape(reader, Shape::Envelope, true)
}

// The decoder and seed share recording state only during one parse, so keep
// their construction in one scope rather than storing any parser across frames.
fn decode_shape(reader: impl Read, shape: Shape, require_end: bool) -> Result<Value> {
    let recorder = Recorder::default();
    let reader = RecordingRead {
        inner: BufReader::with_capacity(BUFFER, reader),
        recorder: &recorder,
    };
    let mut decoder = serde_json::Deserializer::from_reader(reader);
    let value = Seed {
        shape,
        recorder: &recorder,
    }
    .deserialize(&mut decoder)?;
    if require_end {
        decoder.end()?;
    }
    Ok(value)
}

fn child(shape: Shape, key: &str) -> Option<Shape> {
    use Shape::*;
    match (shape, key) {
        (Envelope, "params") => Some(Params),
        (Params, "change") => Some(Change),
        (Change, "conversationState") => Some(State),
        (Change, "patches") => Some(Patches),
        (Change, "type" | "revision" | "baseRevision") => Some(All),
        (Envelope | Params, _) => Some(All),
        (State, "id" | "requests" | "threadRuntimeStatus") => Some(All),
        (State, "turns") => Some(Turns),
        (State, "turnHistory") => Some(TurnHistory),
        (TurnHistory, "history") => Some(History),
        (History, "entitiesByKey") => Some(Entities),
        (Entities, _) => Some(Turn),
        (Turn, "turnId" | "turnStartedAtMs") => Some(All),
        (Turn, "items") => Some(Items),
        (Item, "id" | "type" | "questions" | "status") => Some(All),
        (Item, "command" | "tool" | "summary") => Some(Brief),
        (Item, "content" | "input") => Some(Input),
        (Part, "type" | "text") => Some(All),
        (All, _) => Some(All),
        _ => None,
    }
}

fn patch_shape(path: &[Value]) -> Option<Shape> {
    let mut shape = Shape::State;
    for key in path {
        shape = match shape {
            Shape::Turns if key.is_u64() => Shape::Turn,
            Shape::Items if key.is_u64() => Shape::Item,
            Shape::Input if key.is_u64() => Shape::Part,
            Shape::All => Shape::All,
            Shape::Brief => Shape::Brief,
            _ => child(shape, key.as_str()?)?,
        };
    }
    (!path.is_empty()).then_some(shape)
}

#[derive(Clone, Copy)]
struct Seed<'a> {
    shape: Shape,
    recorder: &'a Recorder,
}
impl Seed<'_> {
    fn child(self, shape: Shape) -> Self {
        Self { shape, ..self }
    }
}

impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;

    fn deserialize<D: de::Deserializer<'de>>(
        self,
        decoder: D,
    ) -> std::result::Result<Value, D::Error> {
        if matches!(self.shape, Shape::Brief) {
            // Validate/skip arbitrarily large metadata without allocating its
            // JSON string. Retain only small public progress descriptions.
            match Capture(self.recorder).deserialize(decoder)? {
                Spool::Memory(bytes) if bytes.len() <= 8192 => {
                    Value::deserialize(&mut serde_json::Deserializer::from_slice(&bytes))
                        .map_err(de::Error::custom)
                }
                _ => Ok(Value::Null),
            }
        } else if matches!(self.shape, Shape::All) {
            Value::deserialize(decoder)
        } else {
            decoder.deserialize_any(self)
        }
    }
}

impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a Codex state object or indexed array")
    }

    fn visit_unit<E: de::Error>(self) -> std::result::Result<Value, E> {
        Ok(Value::Null)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<Value, A::Error> {
        let shape = match self.shape {
            Shape::Turns => Shape::Turn,
            Shape::Items => Shape::Item,
            Shape::Input => Shape::Part,
            Shape::Patches => Shape::Patch,
            _ => return Err(de::Error::custom("unexpected state array")),
        };
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(self.child(shape))? {
            // Ignored tool/token patches contribute only to revision advancement.
            if !matches!(self.shape, Shape::Patches) || !value.is_null() {
                values.push(value);
            }
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<Value, A::Error> {
        let mut result = Map::new();
        let mut content = None;
        let mut input = None;
        let mut patch_value = None;
        while let Some(key) = map.next_key::<String>()? {
            if matches!(self.shape, Shape::Item) && (key == "content" || key == "input") {
                match result.get("type").and_then(Value::as_str) {
                    Some(kind) => {
                        if (key == "content" && kind == "userMessage")
                            || (key == "input" && kind == "steeringUserMessage")
                        {
                            result.insert(key, map.next_value_seed(self.child(Shape::Input))?);
                        } else {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                    None => {
                        let spool = map.next_value_seed(Capture(self.recorder))?;
                        if key == "content" {
                            content = Some(spool);
                        } else {
                            input = Some(spool);
                        }
                    }
                }
            } else if matches!(self.shape, Shape::Patch) {
                match key.as_str() {
                    "value" => {
                        if let Some(path) = result.get("path").and_then(Value::as_array) {
                            if let Some(shape) = patch_shape(path) {
                                result.insert(key, map.next_value_seed(self.child(shape))?);
                            } else {
                                map.next_value::<IgnoredAny>()?;
                            }
                        } else {
                            patch_value = Some(map.next_value_seed(Capture(self.recorder))?);
                        }
                    }
                    "op" | "path" => {
                        result.insert(key, map.next_value()?);
                    }
                    _ => {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
            } else if let Some(shape) = child(self.shape, &key) {
                result.insert(key, map.next_value_seed(self.child(shape))?);
            } else {
                map.next_value::<IgnoredAny>()?;
            }
        }
        if matches!(self.shape, Shape::Item) {
            let deferred = match result.get("type").and_then(Value::as_str) {
                Some("userMessage") => content.map(|spool| ("content", spool)),
                Some("steeringUserMessage") => input.map(|spool| ("input", spool)),
                _ => None,
            };
            if let Some((key, spool)) = deferred {
                let reader = spool.reader().map_err(de::Error::custom)?;
                // A numeric value may have caused serde_json to peek the next
                // delimiter while capturing. Parse exactly one validated value.
                let value = decode_shape(reader, Shape::Input, false).map_err(de::Error::custom)?;
                result.insert(key.into(), value);
            }
        } else if matches!(self.shape, Shape::Patch) {
            let path = result
                .get("path")
                .and_then(Value::as_array)
                .ok_or_else(|| de::Error::custom("invalid patch path"))?;
            let Some(shape) = patch_shape(path) else {
                return Ok(Value::Null);
            };
            if let Some(spool) = patch_value {
                let value = decode_shape(spool.reader().map_err(de::Error::custom)?, shape, false)
                    .map_err(de::Error::custom)?;
                result.insert("value".into(), value);
            }
        }
        Ok(Value::Object(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn progress_metadata_is_bounded_and_never_retains_reasoning_content_or_tool_output() {
        let change = json!({"type":"snapshot","revision":1,"conversationState":{"id":"chat","requests":[],"threadRuntimeStatus":{"type":"active","activeFlags":[]},"turns":[{"turnId":"turn","items":[
            {"type":"commandExecution","command":"cargo test","status":"inProgress","aggregatedOutput":"PRIVATE OUTPUT"},
            {"type":"reasoning","summary":["Checking tests"],"content":["RAW REASONING"]},
            {"type":"commandExecution","command":"x".repeat(100000),"status":"inProgress"}
        ]}]}});
        let decoded =
            decode(&serde_json::to_vec(&json!({"params":{"change":change}})).unwrap()).unwrap();
        let state = super::super::async_questions::project(
            &decoded["params"]["change"]["conversationState"],
        );
        let items = &state["turns"][0]["items"];
        assert_eq!(items[0]["command"], "cargo test");
        assert_eq!(items[1]["summary"], json!(["Checking tests"]));
        assert!(items[2]["command"].is_null());
        let text = state.to_string();
        assert!(!text.contains("PRIVATE OUTPUT") && !text.contains("RAW REASONING"));
        assert!(text.len() < 2000);
    }

    #[test]
    fn snapshots_discard_images_and_tool_output_before_value_allocation() {
        let large = "x".repeat(8 * 1024 * 1024);
        let bytes = format!(
            r#"{{"method":"thread-stream-state-changed","params":{{"change":{{"type":"snapshot","conversationState":{{"id":"chat","requests":[],"threadRuntimeStatus":{{"type":"idle"}},"turns":[{{"turnId":"one","items":[{{"type":"tool","content":"{large}","output":"{large}"}},{{"type":"userMessage","content":[{{"type":"image","url":"{large}"}},{{"type":"text","text":"reply"}}]}}]}}]}}}}}}}}"#
        );
        let value = decode(bytes.as_bytes()).unwrap();
        assert!(serde_json::to_vec(&value).unwrap().len() < 1024);
        let state = &value["params"]["change"]["conversationState"];
        assert_eq!(state["turns"][0]["items"].as_array().unwrap().len(), 2);
        assert!(state["turns"][0]["items"][0].get("content").is_none());
        assert_eq!(state["turns"][0]["items"][1]["content"][1]["text"], "reply");
        assert!(
            state["turns"][0]["items"][1]["content"][0]
                .get("url")
                .is_none()
        );
    }

    #[test]
    fn patches_are_projected_independent_of_json_field_order() {
        let value = decode(br#"{"method":"thread-stream-state-changed","params":{"change":{"type":"patches","baseRevision":1,"revision":2,"patches":[{"value":{"type":"image","url":"secret-image"},"path":["turns",0,"items",0,"content",1],"op":"add"},{"value":"large tool delta","path":["turns",0,"items",1,"output"],"op":"replace"},{"path":["requests",0],"op":"remove"}]}}}"#).unwrap();
        let patches = &value["params"]["change"]["patches"];
        assert_eq!(patches.as_array().unwrap().len(), 2);
        assert_eq!(patches[0]["value"], json!({"type":"image"}));
        assert_eq!(patches[1]["path"], json!(["requests", 0]));
        assert!(patches[1].get("value").is_none());
    }

    #[test]
    fn replies_requests_and_invalid_json_are_not_silently_changed() {
        let value = json!({"type":"response","result":{"nested":[1,"text"]}});
        assert_eq!(decode(&serde_json::to_vec(&value).unwrap()).unwrap(), value);
        assert!(decode(br#"{"method":"thread-stream-state-changed","params":{"change":{"patches":[{"op":"remove","path":false}]}}}"#).is_err());
        assert!(decode(br#"{"method":"thread-stream-state-changed","params":{"change":{"conversationState":{"ignored":invalid}}}}"#).is_err());
    }

    #[test]
    fn deferred_scalars_strings_and_inputs_survive_json_lookahead_and_spilling() {
        for literal in ["1.25e+3", "false", "null", r#""text\\\"with escape""#] {
            let wire = format!(
                r#"{{"params":{{"change":{{"patches":[{{"value":{literal},"op":"replace","path":["requests",0,"id"]}}]}}}}}}"#
            );
            let value = decode(wire.as_bytes()).unwrap();
            assert_eq!(
                value["params"]["change"]["patches"][0]["value"],
                serde_json::from_str::<Value>(literal).unwrap()
            );
        }
        // `content` precedes `type`, and must not be lost or held in a large
        // memory allocation while waiting to learn the item's discriminator.
        let image = "x".repeat(BUFFER * 3);
        let wire = format!(
            r#"{{"params":{{"change":{{"conversationState":{{"turns":[{{"items":[{{"content":[{{"type":"image","url":"{image}"}},{{"text":"answer","type":"text"}}],"type":"userMessage"}}]}}]}}}}}}}}"#
        );
        let value = decode(wire.as_bytes()).unwrap();
        let input =
            &value["params"]["change"]["conversationState"]["turns"][0]["items"][0]["content"];
        assert_eq!(input[1]["text"], "answer");
        assert!(input[0].get("url").is_none());
        let mut spool = Spool::Memory(Vec::new());
        spool.append(&vec![b'x'; BUFFER]).unwrap();
        assert!(matches!(spool, Spool::Memory(ref bytes) if bytes.len() == BUFFER));
        spool.append(b"y").unwrap();
        assert!(matches!(spool, Spool::Disk(_)));
        let mut data = Vec::new();
        spool.reader().unwrap().read_to_end(&mut data).unwrap();
        assert_eq!(data.len(), BUFFER + 1);
        assert_eq!(data[BUFFER], b'y');
    }

    /// Opt-in subprocess benchmark: no live Codex socket, agents or application
    /// restart. A generated image-heavy frame gives both decoders identical input.
    #[test]
    #[ignore = "manual peak-memory comparison on a synthetic 160 MiB owner frame"]
    fn decoder_memory_comparison() {
        use std::io::Write;
        if let Ok(mode) = std::env::var("AAM_DECODER_MEMORY_PROBE") {
            let mut file = tempfile::tempfile().unwrap();
            let spooled = mode == "spooled";
            if spooled {
                file.write_all(br#"{"method":"thread-stream-state-changed","params":{"change":{"type":"patches","patches":[{"value":""#).unwrap();
            } else {
                file.write_all(br#"{"method":"thread-stream-state-changed","params":{"change":{"type":"snapshot","conversationState":{"id":"chat","requests":[],"threadRuntimeStatus":{"type":"idle"},"turns":[{"turnId":"one","items":[{"type":"tool","output":""#).unwrap();
            }
            let chunk = vec![b'x'; 1024 * 1024];
            for _ in 0..160 {
                file.write_all(&chunk).unwrap();
            }
            if spooled {
                file.write_all(br#"","op":"replace","path":["turns",0,"items",0,"output"]}]}}}"#)
                    .unwrap();
            } else {
                file.write_all(br#""}] }]}}}}"#).unwrap();
            }
            drop(chunk);
            use std::io::{Read, Seek, SeekFrom};
            file.seek(SeekFrom::Start(0)).unwrap();
            let streaming = mode == "streamed" || spooled;
            let mut bytes = Vec::new();
            if !streaming {
                bytes.reserve_exact(file.metadata().unwrap().len() as usize);
                file.read_to_end(&mut bytes).unwrap();
            }
            let started = std::time::Instant::now();
            let value: Value = if mode == "baseline" {
                serde_json::from_slice(&bytes).unwrap()
            } else if streaming {
                decode_reader(file).unwrap()
            } else {
                decode(&bytes).unwrap()
            };
            let elapsed = started.elapsed();
            std::hint::black_box(&value);
            let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
            // SAFETY: valid writable rusage storage, inspected only on success.
            assert_eq!(
                unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) },
                0
            );
            // SAFETY: getrusage successfully initialized the storage.
            let peak = unsafe { usage.assume_init() }.ru_maxrss as f64;
            let peak_mib = if cfg!(target_os = "macos") {
                peak / (1024.0 * 1024.0)
            } else {
                peak / 1024.0
            };
            let retained = if mode == "baseline" {
                value["params"]["change"]["conversationState"]["turns"][0]["items"][0]["output"]
                    .as_str()
                    .unwrap()
                    .len()
            } else {
                let encoded = serde_json::to_vec(&value).unwrap();
                assert!(encoded.len() < 1024);
                encoded.len()
            };
            println!(
                "{mode}: peak={peak_mib:.1} MiB decode={elapsed:.2?} retained={retained} bytes"
            );
            return;
        }
        for mode in ["baseline", "projected", "streamed", "spooled"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "agents::codex::projection::tests::decoder_memory_comparison",
                    "--ignored",
                    "--nocapture",
                ])
                .env("AAM_DECODER_MEMORY_PROBE", mode)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            println!("{}", String::from_utf8_lossy(&output.stdout));
        }
    }
}

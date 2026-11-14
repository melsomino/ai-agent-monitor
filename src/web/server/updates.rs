//! Per-connection baselines. Entity IDs are keys; order is a separate value.
use serde_json::{Map, Value, json};

pub const PROTOCOL: u64 = 1;

#[derive(Default)]
pub struct Stream {
    revision: u64,
    state: Option<Value>,
}

pub struct Update {
    state: Value,
    revision: u64,
    pub text: String,
}

impl Stream {
    /// Prepare without advancing the baseline. Only commit after a successful send.
    pub fn prepare(&self, snapshot: Value, reset: bool) -> Option<Update> {
        let state = keyed(snapshot);
        if !reset && self.state.as_ref() == Some(&state) {
            return None;
        }
        let revision = self.revision + 1;
        let full = || {
            json!({"type":"snapshot","protocol":PROTOCOL,"revision":revision,"state":state})
                .to_string()
        };
        let text = if let Some(previous) = self.state.as_ref().filter(|_| !reset) {
            let mut changes = Vec::new();
            diff(previous, &state, &mut Vec::new(), &mut changes);
            let patch = json!({"type":"patch","protocol":PROTOCOL,"baseRevision":self.revision,"revision":revision,"changes":changes}).to_string();
            let full = full();
            if patch.len() < full.len() {
                patch
            } else {
                full
            }
        } else {
            full()
        };
        Some(Update {
            state,
            revision,
            text,
        })
    }

    pub fn commit(&mut self, update: Update) {
        self.revision = update.revision;
        self.state = Some(update.state);
    }
}

fn entities(values: Value) -> (Value, Value) {
    let mut order = Vec::new();
    let mut by_id = Map::new();
    for value in values.as_array().expect("projected entity list") {
        let id = value["id"].as_str().expect("projected entity ID");
        order.push(Value::String(id.to_owned()));
        by_id.insert(id.to_owned(), value.clone());
    }
    (Value::Object(by_id), Value::Array(order))
}

fn keyed(mut snapshot: Value) -> Value {
    let root = snapshot.as_object_mut().expect("projected snapshot");
    // Monitor revisions also advance for unrelated channel/settings work.
    root.remove("revision");
    let (mut sessions, order) = entities(root.remove("sessions").expect("projected sessions"));
    for session in sessions.as_object_mut().unwrap().values_mut() {
        let session = session.as_object_mut().unwrap();
        let (activities, order) =
            entities(session.remove("activities").expect("projected activities"));
        session.insert("activities".into(), activities);
        session.insert("activityOrder".into(), order);
    }
    root.insert("sessions".into(), sessions);
    root.insert("sessionOrder".into(), order);
    snapshot
}

fn diff(previous: &Value, current: &Value, path: &mut Vec<String>, changes: &mut Vec<Value>) {
    if previous == current {
        return;
    }
    if let (Some(previous), Some(current)) = (previous.as_object(), current.as_object()) {
        for key in previous.keys().filter(|key| !current.contains_key(*key)) {
            path.push(key.clone());
            changes.push(json!({"op":"remove","path":path}));
            path.pop();
        }
        for (key, value) in current {
            path.push(key.clone());
            if let Some(old) = previous.get(key) {
                diff(old, value, path, changes);
            } else {
                changes.push(json!({"op":"set","path":path,"value":value}));
            }
            path.pop();
        }
    } else {
        // Arrays (choices/history/order) and scalars are atomic. Null is a value,
        // distinct from removing a field. Paths are key arrays, never JS expressions.
        changes.push(json!({"op":"set","path":path,"value":current}));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Value {
        json!({"revision":7,"sessions":[{"id":"codex:a/b","title":"Session","activities":[{"id":"message","type":"message","html":"A long message".repeat(200)},{"id":"request","type":"request","request":{"phase":"ready","error":null}}]}],"usage":[],"error":null})
    }

    fn apply(state: &mut Value, changes: &[Value]) {
        for change in changes {
            let path = change["path"].as_array().unwrap();
            let mut parent = &mut *state;
            for key in &path[..path.len() - 1] {
                parent = &mut parent[key.as_str().unwrap()];
            }
            let key = path.last().unwrap().as_str().unwrap();
            if change["op"] == "remove" {
                parent.as_object_mut().unwrap().remove(key);
            } else {
                parent[key] = change["value"].clone();
            }
        }
    }

    #[test]
    fn updates_keep_entity_identity_and_drop_revisions_without_visible_changes() {
        let mut stream = Stream::default();
        let initial = stream.prepare(snapshot(), false).unwrap();
        let first: Value = serde_json::from_str(&initial.text).unwrap();
        assert_eq!(first["type"], "snapshot");
        assert_eq!(first["revision"], 1);
        assert_eq!(first["state"]["sessionOrder"], json!(["codex:a/b"]));
        // Preparing a failed send cannot change the base or revision.
        assert_eq!(
            stream.prepare(snapshot(), false).unwrap().text,
            initial.text
        );
        stream.commit(initial);
        let mut next = snapshot();
        next["revision"] = 999.into();
        assert!(stream.prepare(next.clone(), false).is_none());
        next["sessions"][0]["activities"][1]["request"]["phase"] = "sending".into();
        let update = stream.prepare(next.clone(), false).unwrap();
        let patch: Value = serde_json::from_str(&update.text).unwrap();
        assert_eq!(patch["type"], "patch");
        assert_eq!(patch["baseRevision"], 1);
        assert_eq!(patch["revision"], 2);
        assert_eq!(
            patch["changes"],
            json!([{"op":"set","path":["sessions","codex:a/b","activities","request","request","phase"],"value":"sending"}])
        );
        assert!(!update.text.contains("A long message"));
        let mut received = first["state"].clone();
        apply(&mut received, patch["changes"].as_array().unwrap());
        assert_eq!(received, keyed(next));
        stream.commit(update);
        let reset = stream.prepare(snapshot(), true).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&reset.text).unwrap()["type"],
            "snapshot"
        );
    }

    #[test]
    fn add_remove_reorder_null_and_history_round_trip_without_replacing_other_entities() {
        let mut stream = Stream::default();
        let first = stream.prepare(snapshot(), false).unwrap();
        let mut received = first.state.clone();
        stream.commit(first);
        let mut next = snapshot();
        next["sessions"][0]["activities"]
            .as_array_mut()
            .unwrap()
            .insert(0, json!({"id":"earlier","type":"message","html":"Earlier"}));
        next["sessions"][0]["history"] = json!({"messages":[{"html":"History"}],"loading":false});
        next["sessions"][0]["activities"][2]["request"]["error"] = "Retry".into();
        next["sessions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"second","activities":[],"title":"Second"}));
        for step in 0..4 {
            let update = stream.prepare(next.clone(), false).unwrap();
            let patch: Value = serde_json::from_str(&update.text).unwrap();
            assert_eq!(patch["type"], "patch");
            apply(&mut received, patch["changes"].as_array().unwrap());
            assert_eq!(received, keyed(next.clone()));
            stream.commit(update);
            match step {
                0 => next["sessions"].as_array_mut().unwrap().reverse(),
                1 => {
                    next["sessions"].as_array_mut().unwrap().remove(0);
                    next["sessions"][0]["activities"]
                        .as_array_mut()
                        .unwrap()
                        .remove(0);
                }
                2 => next["sessions"][0]["activities"][1]["request"]["error"] = Value::Null,
                _ => {}
            }
        }
    }

    #[test]
    fn devices_have_independent_baselines_and_large_changes_can_use_a_full_snapshot() {
        let mut a = Stream::default();
        let b = Stream::default();
        a.commit(a.prepare(snapshot(), false).unwrap());
        assert!(a.prepare(snapshot(), false).is_none());
        assert!(b.prepare(snapshot(), false).is_some());
        let empty = json!({"sessions":[],"usage":[],"error":null});
        let update = a.prepare(empty, false).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&update.text).unwrap()["type"],
            "snapshot"
        );
    }
}

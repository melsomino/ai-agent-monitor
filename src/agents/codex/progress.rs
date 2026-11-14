//! Small public activity captions, separate from messages and human requests.
use serde_json::Value;

pub(super) fn bounded(text: &str) -> String {
    let mut result = String::new();
    let mut space = false;
    let mut count = 0;
    for c in text.chars() {
        if c.is_whitespace() {
            space = !result.is_empty();
            continue;
        }
        if space {
            if count == 512 {
                break;
            }
            result.push(' ');
            count += 1;
            space = false;
        }
        if count == 512 {
            break;
        }
        result.push(c);
        count += 1;
    }
    result
}

/// Presentation only: unwrap the conventional shell command envelope without
/// evaluating it. Keep unfamiliar or incomplete invocations unchanged.
fn command_caption(command: &str) -> String {
    if let Some(args) = shlex::split(command)
        && args.len() == 3
        && matches!(args[1].as_str(), "-c" | "-lc" | "-cl")
        && matches!(
            args[0].rsplit('/').next(),
            Some("sh" | "bash" | "zsh" | "dash" | "ksh")
        )
    {
        return bounded(&args[2]);
    }
    bounded(command)
}
pub(super) fn fields(item: &Value) -> serde_json::Map<String, Value> {
    let mut result = serde_json::Map::new();
    let keys: &[&str] = match item["type"].as_str() {
        Some("commandExecution") => &["status", "command"],
        Some("dynamicToolCall" | "mcpToolCall") => &["status", "tool"],
        Some("reasoning") => &["summary"],
        Some("fileChange" | "webSearch") => &["status"],
        _ => &[],
    };
    for key in keys {
        let value = &item[*key];
        let size = if let Some(text) = value.as_str() {
            text.len()
        } else if let Some(parts) = value.as_array() {
            parts
                .iter()
                .map(|v| v.as_str().map_or(8193, str::len))
                .sum()
        } else {
            8193
        };
        if size <= 8192 {
            result.insert((*key).into(), value.clone());
        }
    }
    result
}
pub(super) fn empty_summary(item: &Value) -> bool {
    item["type"] == "reasoning"
        && !item["summary"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .any(|text| !text.trim().is_empty())
}
pub(super) fn caption(item: &Value) -> Option<String> {
    // This is the last displayed activity caption, not a claim that the
    // underlying command is still executing. Completion alone does not erase it.
    let described = matches!(
        item["status"].as_str(),
        Some("inProgress" | "in_progress" | "running" | "completed" | "failed" | "declined")
    );
    let text = match item["type"].as_str()? {
        "reasoning" => item["summary"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .rev()
            .find(|s| !s.trim().is_empty())
            .map(|s| format!("Thinking: {}", bounded(s)))
            .unwrap_or_else(|| "Thinking…".into()),
        "commandExecution" if described => format!(
            "Running {}",
            item["command"]
                .as_str()
                .map(command_caption)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "command…".into())
        ),
        "dynamicToolCall" | "mcpToolCall" if described => format!(
            "Running {}…",
            item["tool"]
                .as_str()
                .map(bounded)
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "tool".into())
        ),
        "fileChange" if described => "Editing files…".into(),
        "webSearch" if described => "Searching the web…".into(),
        _ => return None,
    };
    Some(bounded(&text))
}
pub(super) fn current(state: &Value) -> Option<String> {
    let history = state["turnHistory"]["history"]["entitiesByKey"].as_object();
    let turns: std::collections::BTreeMap<_, _> = history
        .into_iter()
        .flat_map(|m| m.values())
        .chain(state["turns"].as_array().into_iter().flatten())
        .filter_map(|turn| Some((turn["turnId"].as_str()?, turn)))
        .collect();
    let turn = turns
        .values()
        .max_by_key(|turn| (turn["turnStartedAtMs"].as_u64(), turn["turnId"].as_str()))?;
    let mut thinking = false;
    for item in turn["items"].as_array()?.iter().rev() {
        if matches!(item["type"].as_str(), Some("agentMessage" | "userMessage")) {
            break;
        }
        if empty_summary(item) {
            thinking = true;
            continue;
        }
        if let Some(caption) = caption(item) {
            return Some(caption);
        }
    }
    thinking.then(|| "Thinking…".into())
}

/// Retain distinct same-type items available in a snapshot, even if several
/// completed before the observer consumed its notification. Never inspect output.
pub(super) fn recent_events(
    state: &Value,
) -> std::collections::VecDeque<crate::domain::session_activity::SessionActivity> {
    use crate::domain::session_activity::*;
    let mut turns: Vec<_> = state["turnHistory"]["history"]["entitiesByKey"]
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .chain(state["turns"].as_array().into_iter().flatten())
        .collect();
    turns.sort_by_key(|t| (t["turnStartedAtMs"].as_i64(), t["turnId"].as_str()));
    let mut result = std::collections::VecDeque::new();
    for turn in turns {
        for (index, item) in turn["items"].as_array().into_iter().flatten().enumerate() {
            let Some(text) = caption(item) else {
                continue;
            };
            let time = timestamp(&item["startedAt"])
                .or_else(|| timestamp(&item["startedAtMs"]))
                .or_else(|| timestamp(&turn["turnStartedAtMs"]))
                .unwrap_or_else(|| seconds(0));
            upsert(
                &mut result,
                SessionActivity::Action(SessionAction {
                    id: format!(
                        "codex-item:{}:{}",
                        turn["turnId"].as_str().unwrap_or("unknown"),
                        item["id"]
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("index:{index}"))
                    ),
                    started_at: time,
                    updated_at: time,
                    kind: match item["type"].as_str() {
                        Some("commandExecution") => ActionKind::Command,
                        Some("reasoning") => ActionKind::Reasoning,
                        _ => ActionKind::Tool,
                    },
                    current: false,
                    text,
                    status: match item["status"].as_str() {
                        Some("completed") => ActionStatus::Completed,
                        Some("failed" | "declined") => ActionStatus::Failed,
                        _ => ActionStatus::Running,
                    },
                }),
            );
        }
    }
    result
}

/// A provider item, rather than its displayed caption, identifies an activity.
pub(super) fn current_event(
    state: &Value,
) -> Option<crate::domain::session_activity::SessionAction> {
    use crate::domain::session_activity::*;
    let history = &state["turnHistory"]["history"]["entitiesByKey"];
    let turn = history
        .as_object()
        .into_iter()
        .flat_map(|m| m.values())
        .chain(state["turns"].as_array().into_iter().flatten())
        .max_by_key(|t| (t["turnStartedAtMs"].as_i64(), t["turnId"].as_str()))?;
    let text = current(state)?;
    let (index, item) = turn["items"]
        .as_array()?
        .iter()
        .enumerate()
        .rev()
        .find(|(_, item)| caption(item).as_deref() == Some(text.as_str()))?;
    let started = timestamp(&item["startedAt"])
        .or_else(|| timestamp(&item["startedAtMs"]))
        .unwrap_or_else(now);
    Some(SessionAction {
        id: format!(
            "codex-item:{}:{}",
            turn["turnId"].as_str().unwrap_or("unknown"),
            item["id"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("index:{index}"))
        ),
        started_at: started,
        updated_at: started,
        text,
        current: true,
        kind: match item["type"].as_str() {
            Some("commandExecution") => ActionKind::Command,
            Some("reasoning") => ActionKind::Reasoning,
            _ => ActionKind::Tool,
        },
        status: match item["status"].as_str() {
            Some("completed") => ActionStatus::Completed,
            Some("failed" | "declined") => ActionStatus::Failed,
            _ => ActionStatus::Running,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn shell_envelopes_are_unwrapped_only_for_display() {
        for command in [
            "/bin/zsh -lc \"git grep -n 'example'\"",
            "bash -c 'git grep -n '\"'\"'example'\"'\"''",
        ] {
            let item = json!({"type":"commandExecution","status":"inProgress","command":command});
            assert_eq!(
                caption(&item).as_deref(),
                Some("Running git grep -n 'example'")
            );
            assert_eq!(fields(&item)["command"], command);
        }
        for command in [
            "git status",
            "python -c 'print(1)'",
            "/bin/zsh -lc 'unfinished",
            "zsh -lc 'pwd' extra",
        ] {
            assert_eq!(command_caption(command), command);
        }
    }
    #[test]
    fn repeated_commands_are_distinct_source_items_even_with_identical_times_and_text() {
        let state = json!({"turns":[{"turnId":"turn","turnStartedAtMs":1000,"items":[{"id":"one","type":"commandExecution","command":"cargo test","status":"completed"},{"id":"two","type":"commandExecution","command":"cargo test","status":"inProgress"}]}]});
        let events = recent_events(&state);
        assert_eq!(events.len(), 2);
        assert_ne!(events[0].id(), events[1].id());
        assert_eq!(events[0].started_at(), events[1].started_at());
        assert!(current_event(&state).unwrap().id.ends_with(":two"));
    }
    #[test]
    fn last_activity_survives_tool_completion_until_new_description_or_message() {
        let mut state = json!({"turns":[{"turnId":"new","turnStartedAtMs":2,"items":[{"type":"commandExecution","command":"cargo test\n--offline","status":"inProgress","aggregatedOutput":"PRIVATE OUTPUT"}]}],"turnHistory":{"history":{"entitiesByKey":{"old":{"turnId":"old","turnStartedAtMs":1,"items":[{"type":"reasoning","summary":["Old summary"]}]}}}}});
        assert_eq!(
            current(&state).as_deref(),
            Some("Running cargo test --offline")
        );
        assert!(!fields(&state["turns"][0]["items"][0]).contains_key("aggregatedOutput"));
        state["turns"][0]["items"][0]["status"] = json!("completed");
        assert_eq!(
            current(&state).as_deref(),
            Some("Running cargo test --offline")
        );
        state["turns"][0]["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"reasoning","summary":[]}));
        assert_eq!(
            current(&state).as_deref(),
            Some("Running cargo test --offline"),
            "empty summaries must not erase the last caption"
        );
        state["turns"][0]["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"reasoning","summary":["Checking results"]}));
        assert_eq!(
            current(&state).as_deref(),
            Some("Thinking: Checking results")
        );
        state["turns"][0]["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"agentMessage"}));
        assert!(
            current(&state).is_none(),
            "a new agent message supersedes the last activity"
        );
        state["turns"].as_array_mut().unwrap().push(json!({"turnId":"next","turnStartedAtMs":3,"items":[{"type":"userMessage"},{"type":"reasoning","summary":[]}]}));
        assert_eq!(
            current(&state).as_deref(),
            Some("Thinking…"),
            "no old-turn caption may leak into new work"
        );
        assert_eq!(caption(&json!({"type":"reasoning","summary":["Checking tests"],"content":["RAW REASONING"]})).as_deref(),Some("Thinking: Checking tests"));
        assert_eq!(
            caption(&json!({"type":"reasoning","content":["RAW REASONING"]})).as_deref(),
            Some("Thinking…")
        );
        assert_eq!(
            caption(&json!({"type":"reasoning","summary":[]})).as_deref(),
            Some("Thinking…")
        );
        assert!(fields(&json!({"type":"commandExecution","command":"x".repeat(9000)})).is_empty());
        assert_eq!(bounded(&"\u{44f}".repeat(1000)).chars().count(), 512);
    }
}

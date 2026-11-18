//! Read-only question visibility when the IDE owner cannot be reached. A rollout
//! never grants a response route; answering still requires a live owner.
use crate::domain::session_activity::*;
use crate::domain::{interaction::Interaction, session::Approval};
use serde_json::Value;

#[derive(Default)]
pub(super) struct Reader {
    calls: Vec<Call>,
}
struct Call {
    id: String,
    started: ActivityTime,
    updated: ActivityTime,
    questions: Vec<(String, String, bool)>,
    superseded: bool,
}
impl Reader {
    pub fn consume(&mut self, value: &Value, time: ActivityTime) {
        let payload = &value["payload"];
        if value["type"] == "response_item"
            && payload["type"] == "function_call"
            && payload["name"] == "request_user_input_async"
        {
            let Some(id) = payload["call_id"].as_str() else {
                return;
            };
            let Some(arguments) = payload["arguments"]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
            else {
                return;
            };
            let questions: Vec<_> = arguments["questions"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .filter_map(|(index, q)| {
                    let title = q["title"].as_str()?.trim();
                    (!title.is_empty()).then(|| {
                        (
                            serde_json::json!(["request_user_input_async", id, index]).to_string(),
                            title.chars().take(16000).collect(),
                            false,
                        )
                    })
                })
                .collect();
            if !questions.is_empty() && !self.calls.iter().any(|call| call.id == id) {
                if self.calls.len() >= 128 {
                    let Some(index) = self.calls.iter().position(|call| {
                        call.superseded || call.questions.iter().all(|(_, _, answered)| *answered)
                    }) else {
                        return;
                    };
                    self.calls.remove(index);
                }
                self.calls.push(Call {
                    id: id.into(),
                    started: time,
                    updated: time,
                    questions,
                    superseded: false,
                });
            }
        } else if let Some(text) = user_text(value) {
            if let Some(data) = text
                .trim()
                .strip_prefix("<send_user_message_question_reply>")
                .and_then(|s| s.strip_suffix("</send_user_message_question_reply>"))
                && let Ok(replies) = serde_json::from_str::<Value>(data.trim())
            {
                let rows: Vec<_> = if let Some(rows) = replies.as_array() {
                    rows.iter().collect()
                } else {
                    vec![&replies]
                };
                for call in &mut self.calls {
                    for (id, _, answered) in &mut call.questions {
                        let was_answered = *answered;
                        *answered |= rows
                            .iter()
                            .any(|r| r["questionItemId"] == *id && r.get("answer").is_some());
                        if *answered != was_answered {
                            call.updated = time;
                        }
                    }
                }
            } else if !text.trim().is_empty() {
                for call in &mut self.calls {
                    if !call.superseded {
                        call.updated = time;
                    }
                    call.superseded = true;
                }
            }
        } else if value["type"] == "response_item"
            && payload["type"] == "function_call_output"
            && let Some(id) = payload["call_id"].as_str()
            && let Some(call) = self.calls.iter_mut().find(|c| c.id == id)
        {
            // A failed tool invocation did not create a question. Do not infer
            // success/failure from arbitrary command output or assistant prose.
            if let Some(output) = payload["output"]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                && output["accepted"] == false
            {
                call.superseded = true;
                call.updated = time;
            }
        }
    }
    pub fn activities(&self) -> impl Iterator<Item = SessionActivity> + '_ {
        self.calls.iter().map(|call| {
            let status = if call.superseded {
                RequestStatus::Superseded
            } else if call.questions.iter().all(|(_, _, answered)| *answered) {
                RequestStatus::Answered
            } else {
                RequestStatus::Pending
            };
            let body = call
                .questions
                .iter()
                .filter(|(_, _, answered)| status != RequestStatus::Pending || !answered)
                .map(|(_, title, _)| title.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let id = format!("rollout-question:{}", call.id);
            SessionActivity::Request(SessionRequest {
                id: id.clone(),
                started_at: call.started,
                updated_at: call.updated,
                approval: Approval {
                    id: format!("transcript:{id}"),
                    body,
                    expires: u64::MAX,
                    can_allow: false,
                },
                interaction: Interaction::default(),
                status,
            })
        })
    }
}
fn user_text(value: &Value) -> Option<String> {
    let payload = &value["payload"];
    if value["type"] == "event_msg" && payload["type"] == "user_message" {
        return payload["message"].as_str().map(str::to_owned);
    }
    if value["type"] != "response_item" || payload["type"] != "message" || payload["role"] != "user"
    {
        return None;
    }
    let text = payload["content"]
        .as_array()?
        .iter()
        .filter_map(|p| p["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed = text.trim();
    // Rollouts also encode injected environment/instruction context as user
    // messages. That context is not a user's cancellation or replacement prompt.
    if trimmed.starts_with("<environment_context>")
        || trimmed.starts_with("# AGENTS.md instructions")
        || trimmed.starts_with("<INSTRUCTIONS>")
    {
        None
    } else {
        Some(text)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn call(id: &str) -> Value {
        json!({"type":"response_item","payload":{"type":"function_call","name":"request_user_input_async","call_id":id,"arguments":json!({"questions":[{"title":"Which checks?","options":["All","Changed"]}]}).to_string()}})
    }
    #[test]
    fn real_question_calls_are_visible_without_advertising_answer_controls() {
        let mut reader = Reader::default();
        reader.consume(&call("one"), seconds(1));
        let SessionActivity::Request(request) = reader.activities().next().unwrap() else {
            panic!()
        };
        assert_eq!(request.approval.body, "Which checks?");
        assert!(request.approval.id.starts_with("transcript:"));
        assert!(!request.approval.can_allow);
        assert!(request.interaction.questions.is_empty());
        assert_eq!(request.status, RequestStatus::Pending);
    }
    #[test]
    fn matching_replies_and_new_user_messages_close_only_existing_calls() {
        let mut reader = Reader::default();
        reader.consume(&call("one"), seconds(1));
        reader.consume(&call("two"), seconds(2));
        let reply = format!(
            "<send_user_message_question_reply>\n{}\n</send_user_message_question_reply>",
            json!([{"questionItemId":json!(["request_user_input_async","one",0]).to_string(),"answer":"All"}])
        );
        reader.consume(
            &json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":reply}]}}),
            seconds(3),
        );
        let events: Vec<_> = reader.activities().collect();
        assert!(
            matches!(&events[0],SessionActivity::Request(r) if r.status==RequestStatus::Answered)
        );
        assert!(events[1].pending());
        let mut reader = Reader::default();
        reader.consume(&call("one"), seconds(1));
        reader.consume(&json!({"type":"event_msg","payload":{"type":"user_message","message":"Continue instead"}}),seconds(2));
        reader.consume(&call("two"), seconds(3));
        let events: Vec<_> = reader.activities().collect();
        assert!(!events[0].pending());
        assert!(events[1].pending());
    }
    #[test]
    fn ordinary_command_calls_never_manufacture_permission_requests() {
        let mut reader = Reader::default();
        reader.consume(&json!({"type":"response_item","payload":{"type":"function_call","name":"exec_command","call_id":"one","arguments":"{}"}}),seconds(1));
        assert_eq!(reader.activities().count(), 0);
    }
    #[test]
    fn injected_context_does_not_cancel_a_pending_question() {
        let mut reader = Reader::default();
        reader.consume(&call("one"), seconds(1));
        reader.consume(&json!({"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>updated context</environment_context>"}]}}),seconds(2));
        assert!(reader.activities().next().unwrap().pending());
    }
    #[test]
    fn completed_history_does_not_hide_a_new_question() {
        let mut reader = Reader::default();
        for i in 0..128 {
            reader.consume(&call(&format!("old:{i}")), seconds(i));
        }
        reader.consume(&serde_json::json!({"type":"event_msg","payload":{"type":"user_message","message":"A new task"}}),seconds(129));
        reader.consume(&call("latest"), seconds(130));
        assert!(
            reader
                .activities()
                .any(|a| a.pending() && a.id().ends_with("latest"))
        );
    }
}

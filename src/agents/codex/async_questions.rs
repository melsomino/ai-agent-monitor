//! Asynchronous questions live on agent messages, even after the turn is idle.
use serde_json::{Value, json};

pub const METHOD: &str = "agent-monitor/asyncUserInput";
fn item(value: &Value) -> Value {
    let mut result = json!({"type":value["type"],"id":value["id"],"questions":value["questions"]});
    if value["type"] == "userMessage" {
        result["content"] = value["content"].clone();
    }
    result
}
fn items(value: &Value) -> Value {
    Value::Array(value.as_array().into_iter().flatten().map(item).collect())
}
fn turn(value: &Value) -> Value {
    json!({"turnId":value["turnId"],"turnStartedAtMs":value["turnStartedAtMs"],"items":items(&value["items"])})
}
fn entities(value: &Value) -> Value {
    Value::Object(
        value
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, value)| (key.clone(), turn(value)))
            .collect(),
    )
}
fn history(value: &Value) -> Value {
    json!({"entitiesByKey":entities(&value["entitiesByKey"])})
}
fn turn_history(value: &Value) -> Value {
    json!({"history":history(&value["history"])})
}
pub fn project(state: &Value) -> Value {
    json!({"id":state["id"],"requests":state["requests"],"threadRuntimeStatus":state["threadRuntimeStatus"],
        "turns":state["turns"].as_array().into_iter().flatten().map(turn).collect::<Vec<_>>(),
        "turnHistory":turn_history(&state["turnHistory"])})
}
/// Ignore token deltas and tool output. Keep array positions so relevant Immer
/// patches still address the same item, without retaining full conversation history.
pub fn project_patch(path: &[Value], value: &Value) -> Option<Value> {
    let rest = match path.first()?.as_str()? {
        "turns" => {
            if path.len() == 1 {
                return Some(Value::Array(
                    value.as_array().into_iter().flatten().map(turn).collect(),
                ));
            }
            if path.len() == 2 {
                return Some(turn(value));
            }
            &path[2..]
        }
        "turnHistory" => match &path[1..] {
            [] => return Some(turn_history(value)),
            [field] if field == "history" => return Some(history(value)),
            [field, map] if field == "history" && map == "entitiesByKey" => {
                return Some(entities(value));
            }
            [field, map, _] if field == "history" && map == "entitiesByKey" => {
                return Some(turn(value));
            }
            [field, map, _, rest @ ..] if field == "history" && map == "entitiesByKey" => rest,
            _ => return None,
        },
        _ => return None,
    };
    match rest {
        [field] if field == "turnId" || field == "turnStartedAtMs" => Some(value.clone()),
        [field] if field == "items" => Some(items(value)),
        [field, _] if field == "items" => Some(item(value)),
        [field, _, attribute, ..]
            if field == "items"
                && matches!(
                    attribute.as_str(),
                    Some("type" | "id" | "questions" | "content")
                ) =>
        {
            Some(value.clone())
        }
        _ => None,
    }
}

pub fn pending(state: &Value) -> Option<Value> {
    let history = state["turnHistory"]["history"]["entitiesByKey"].as_object();
    let unique: std::collections::BTreeMap<_, _> = state["turns"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(history.into_iter().flat_map(|map| map.values()))
        .filter_map(|turn| Some((turn["turnId"].as_str()?, turn)))
        .collect();
    let mut turns: Vec<_> = unique.values().copied().collect();
    turns.sort_by(|a, b| {
        a["turnStartedAtMs"]
            .as_u64()
            .cmp(&b["turnStartedAtMs"].as_u64())
            .then_with(|| a["turnId"].as_str().cmp(&b["turnId"].as_str()))
    });
    let latest = *turns.last()?;
    let mut questions = Vec::new();
    for turn in turns {
        for item in turn["items"].as_array().into_iter().flatten() {
            if item["type"] == "userMessage" {
                let text = item["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|content| content["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                if let Some(replies) = reply_ids(&text) {
                    questions
                        .retain(|question: &Value| !replies.iter().any(|id| question["id"] == *id));
                } else {
                    questions.clear();
                }
            } else if item["type"] == "agentMessage" {
                let Some(id) = item["id"].as_str() else {
                    continue;
                };
                for (index, question) in item["questions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    let Some(title) = question["title"].as_str() else {
                        continue;
                    };
                    let id = json!(["request_user_input_async", id, index]).to_string();
                    let options: Vec<_> = question["options"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|value| {
                            value
                                .as_str()
                                .map(|label| json!({"label":label,"description":""}))
                        })
                        .collect();
                    questions.push(json!({"id":id,"question":title,"options":options,"isOther":true,"isSecret":false}));
                }
            }
        }
    }
    if questions.is_empty() {
        return None;
    }
    Some(
        json!({"method":METHOD,"id":latest["turnId"],"params":{"threadId":state["id"],"turnId":latest["turnId"],"questions":questions}}),
    )
}
fn reply_ids(text: &str) -> Option<Vec<String>> {
    let data = text
        .trim()
        .strip_prefix("<send_user_message_question_reply>")?
        .strip_suffix("</send_user_message_question_reply>")?;
    let data: Value = serde_json::from_str(data).ok()?;
    let rows = data.as_array().cloned().unwrap_or_else(|| vec![data]);
    rows.iter()
        .map(|reply| reply["questionItemId"].as_str().map(str::to_owned))
        .collect()
}
pub fn reply_text(
    request: &Value,
    response: &crate::agents::interaction::Response,
) -> Option<String> {
    let crate::agents::interaction::Response::Answers(answers) = response else {
        return None;
    };
    let interaction = super::requests::interaction(request);
    if !interaction.valid_answers(answers) {
        return None;
    }
    let replies: Vec<_> = interaction.questions.iter().map(|question| json!({
        "questionItemId":question.id,"question":question.title,"answer":answers[&question.id][0]
    })).collect();
    Some(format!(
        "<send_user_message_question_reply>\n{}\n</send_user_message_question_reply>",
        serde_json::to_string(&replies).ok()?
    ))
}

/// Versions match the installed client's coordination protocol: start-turn v2,
/// steer-turn v1. Answer text is admitted by the existing owner, not a new server.
pub fn reply_envelope(
    state: &Value,
    text: &str,
    client_id: &str,
) -> Option<(&'static str, u64, Value)> {
    let session = state["id"].as_str()?;
    let input = json!([{"type":"text","text":text,"text_elements":[]}]);
    match state["threadRuntimeStatus"]["type"].as_str()? {
        "idle" => Some((
            "thread-follower-start-turn",
            2,
            json!({"conversationId":session,"turnStart":{
                "request":{"threadId":session,"input":input,"clientUserMessageId":client_id},"context":{"inheritThreadSettings":true}
            }}),
        )),
        "active" => Some((
            "thread-follower-steer-turn",
            1,
            json!({"conversationId":session,"input":input,"clientUserMessageId":client_id}),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> Value {
        json!({"id":"chat","requests":[],"threadRuntimeStatus":{"type":"idle"},"turns":[],"turnHistory":{"history":{"entitiesByKey":{"turn:one":{"turnId":"one","turnStartedAtMs":1,"items":[{"type":"userMessage","id":"u"},{"type":"agentMessage","id":"call_123","questions":[{"title":"Pick?","options":["A","B"]}]},{"type":"agentMessage","id":"final","text":"Sent"}]}}}}})
    }
    #[test]
    fn completed_idle_turns_still_expose_async_questions_with_native_ids() {
        let state = project(&state());
        let request = pending(&state).unwrap();
        assert_eq!(
            request["params"]["questions"][0]["id"],
            r#"["request_user_input_async","call_123",0]"#
        );
        assert_eq!(
            request["params"]["questions"][0]["options"][1]["label"],
            "B"
        );
        assert!(
            state["turnHistory"]["history"]["entitiesByKey"]["turn:one"]["items"][2]
                .get("text")
                .is_none()
        );
    }
    #[test]
    fn a_new_turn_or_later_user_input_clears_the_prompt_instead_of_reviving_history() {
        let mut state = state();
        state["turnHistory"]["history"]["entitiesByKey"]["turn:two"] = json!({"turnId":"two","turnStartedAtMs":2,"items":[{"type":"userMessage","id":"new","content":[{"type":"text","text":"new prompt"}]}]});
        assert!(pending(&project(&state)).is_none());
        state["turnHistory"]["history"]["entitiesByKey"]
            .as_object_mut()
            .unwrap()
            .remove("turn:two");
        state["turnHistory"]["history"]["entitiesByKey"]["turn:one"]["items"]
            .as_array_mut()
            .unwrap()
            .push(json!({"type":"userMessage","id":"reply"}));
        assert!(pending(&project(&state)).is_none());
    }
    #[test]
    fn replies_remove_only_answered_questions_and_keep_the_other_question() {
        let mut state = state();
        state["turnHistory"]["history"]["entitiesByKey"]["turn:one"]["items"][1]["questions"]
            .as_array_mut()
            .unwrap()
            .push(json!({"title":"Second?","options":["C","D"]}));
        let request = pending(&state).unwrap();
        let id = request["params"]["questions"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let text = format!(
            "<send_user_message_question_reply>\n{}\n</send_user_message_question_reply>",
            json!([{"questionItemId":id,"question":"Pick?","answer":"B"}])
        );
        state["turnHistory"]["history"]["entitiesByKey"]["turn:two"] = json!({"turnId":"two","turnStartedAtMs":2,"items":[{"type":"userMessage","id":"reply","content":[{"type":"text","text":text}]}]});
        let remaining = pending(&project(&state)).unwrap();
        assert_eq!(
            remaining["params"]["questions"].as_array().unwrap().len(),
            1
        );
        assert_eq!(remaining["params"]["questions"][0]["question"], "Second?");
        let id = remaining["params"]["questions"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let response = crate::agents::interaction::Response::Answers(
            std::collections::BTreeMap::from([(id.clone(), vec!["Свой ответ".into()])]),
        );
        let text = reply_text(&remaining, &response).unwrap();
        assert_eq!(reply_ids(&text), Some(vec![id]));
        assert!(text.contains("Свой ответ"));
        assert!(super::super::requests::wire_response(&remaining, &response).is_none());
    }
    #[test]
    fn async_replies_use_the_owner_protocol_versions_and_inherit_existing_settings() {
        let mut state = state();
        let (method, version, params) = reply_envelope(&state, "reply", "client-id").unwrap();
        assert_eq!((method, version), ("thread-follower-start-turn", 2));
        assert_eq!(
            params,
            json!({"conversationId":"chat","turnStart":{
                "request":{"threadId":"chat","input":[{"type":"text","text":"reply","text_elements":[]}],"clientUserMessageId":"client-id"},
                "context":{"inheritThreadSettings":true}
            }})
        );
        state["threadRuntimeStatus"]["type"] = json!("active");
        let (method, version, params) = reply_envelope(&state, "reply", "client-id").unwrap();
        assert_eq!((method, version), ("thread-follower-steer-turn", 1));
        assert_eq!(params["input"][0]["text"], "reply");
        assert!(params.get("turnStart").is_none());
        state["threadRuntimeStatus"]["type"] = json!("systemError");
        assert!(reply_envelope(&state, "reply", "client-id").is_none());
    }
    #[test]
    fn text_deltas_are_ignored_but_question_and_turn_updates_are_projected() {
        let prefix = vec![
            json!("turnHistory"),
            json!("history"),
            json!("entitiesByKey"),
            json!("turn:one"),
            json!("items"),
            json!(1),
        ];
        let mut path = prefix.clone();
        path.push(json!("text"));
        assert!(project_patch(&path, &json!("token delta")).is_none());
        let mut path = prefix;
        path.push(json!("questions"));
        assert_eq!(project_patch(&path, &json!([])), Some(json!([])));
    }
}

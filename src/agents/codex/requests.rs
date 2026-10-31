//! Translate pending Codex requests into neutral controls and exact wire responses.
use crate::agents::interaction::{Action, Choice, Interaction, Question, Response};
use serde_json::{Value, json};
use std::collections::HashSet;

fn decisions(request: &Value) -> Vec<Value> {
    let params = &request["params"];
    if let Some(decisions) = params["availableDecisions"].as_array() {
        return decisions.clone();
    }
    let mut decisions = vec![json!("accept")];
    if request["method"] == "item/fileChange/requestApproval"
        || params["networkApprovalContext"].is_object()
    {
        decisions.push(json!("acceptForSession"));
    }
    if request["method"] == "item/commandExecution/requestApproval" {
        if let Some(amendment) = params["proposedExecpolicyAmendment"]
            .as_array()
            .filter(|items| !items.is_empty() && items.iter().all(Value::is_string))
        {
            decisions
                .push(json!({"acceptWithExecpolicyAmendment":{"execpolicy_amendment":amendment}}));
        }
        if let Some(amendments) = params["proposedNetworkPolicyAmendments"].as_array() {
            for amendment in amendments {
                decisions.push(
                    json!({"applyNetworkPolicyAmendment":{"network_policy_amendment":amendment}}),
                );
            }
        }
    }
    decisions.push(json!("decline"));
    decisions
}
fn decision_label(value: &Value, request: &Value) -> Option<String> {
    match value.as_str() {
        Some("accept") => Some("Allow once".into()),
        Some("acceptForSession") => Some(
            if request["method"] == "item/fileChange/requestApproval" {
                "Allow all edits"
            } else {
                "Allow this conversation"
            }
            .into(),
        ),
        Some("decline") => Some("Deny".into()),
        Some("cancel") => Some(
            if decisions(request).iter().any(|choice| choice == "decline") {
                "Cancel turn"
            } else {
                "Deny"
            }
            .into(),
        ),
        _ if value.get("acceptWithExecpolicyAmendment").is_some() => {
            Some("Allow similar commands".into())
        }
        _ if value.get("applyNetworkPolicyAmendment").is_some() => {
            let amendment = &value["applyNetworkPolicyAmendment"]["network_policy_amendment"];
            Some(
                if amendment["action"] == "deny" {
                    "Always deny"
                } else {
                    "Always allow"
                }
                .into(),
            )
        }
        _ => None,
    }
}
fn decision_detail(value: &Value) -> String {
    if let Some(arguments) = value["acceptWithExecpolicyAmendment"]["execpolicy_amendment"]
        .as_array()
        .filter(|items| !items.is_empty() && items.iter().all(Value::is_string))
    {
        let prefix = arguments
            .iter()
            .map(|argument| shlex::try_quote(argument.as_str().unwrap()))
            .collect::<Result<Vec<_>, _>>();
        return prefix
            .map(|words| format!("Future commands starting with: {}", words.join(" ")))
            .unwrap_or_default();
    }
    value["applyNetworkPolicyAmendment"]["network_policy_amendment"]["host"]
        .as_str()
        .map(|host| format!("Host: {host}"))
        .unwrap_or_default()
}

pub fn interaction(request: &Value) -> Interaction {
    let params = &request["params"];
    let mut result = Interaction::default();
    match request["method"].as_str() {
        Some("item/commandExecution/requestApproval" | "item/fileChange/requestApproval") => {
            if request["method"] == "item/commandExecution/requestApproval" {
                result.command = params["command"]
                    .as_str()
                    .filter(|command| !command.trim().is_empty())
                    .map(str::to_owned)
                    .or_else(|| {
                        let arguments = params["command"].as_array()?;
                        if arguments.is_empty() {
                            return None;
                        }
                        arguments
                            .iter()
                            .map(|argument| shlex::try_quote(argument.as_str()?).ok())
                            .collect::<Option<Vec<_>>>()
                            .map(|words| words.join(" "))
                    });
            }
            for (index, value) in decisions(request).into_iter().enumerate() {
                if let Some(label) = decision_label(&value, request) {
                    result.actions.push(Action {
                        id: if value == "accept" {
                            "allow".into()
                        } else {
                            format!("decision:{index}")
                        },
                        label,
                        detail: decision_detail(&value),
                    });
                }
            }
        }
        Some("item/permissions/requestApproval") if params["permissions"].is_object() => {
            let detail = serde_json::to_string_pretty(&params["permissions"]).unwrap_or_default();
            result.actions = vec![
                Action {
                    id: "permissions:turn".into(),
                    label: "Allow once".into(),
                    detail: detail.clone(),
                },
                Action {
                    id: "permissions:session".into(),
                    label: "Allow this conversation".into(),
                    detail,
                },
                Action {
                    id: "permissions:decline".into(),
                    label: "Deny".into(),
                    detail: String::new(),
                },
            ];
        }
        Some("item/tool/requestUserInput" | super::async_questions::METHOD) => {
            let Some(questions) = params["questions"]
                .as_array()
                .filter(|items| !items.is_empty())
            else {
                return result;
            };
            let mut ids = HashSet::new();
            for question in questions {
                let (Some(id), Some(text)) =
                    (question["id"].as_str(), question["question"].as_str())
                else {
                    return Interaction::default();
                };
                if id.is_empty() || !ids.insert(id) {
                    return Interaction::default();
                }
                let choices: Vec<_> = question["options"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|option| {
                        Some(Choice {
                            label: option["label"].as_str()?.to_owned(),
                            description: option["description"].as_str().unwrap_or("").into(),
                        })
                    })
                    .collect();
                result.questions.push(Question {
                    id: id.into(),
                    title: match question["header"]
                        .as_str()
                        .filter(|header| !header.is_empty())
                    {
                        Some(header) => format!("{header}: {text}"),
                        None => text.into(),
                    },
                    freeform: choices.is_empty() || question["isOther"] == true,
                    choices,
                    secret: question["isSecret"] == true,
                });
            }
        }
        _ => {}
    }
    result
}

pub fn wire_response(
    request: &Value,
    response: &Response,
) -> Option<(&'static str, &'static str, Value)> {
    let controls = interaction(request);
    match response {
        Response::Action(id) if controls.actions.iter().any(|action| action.id == *id) => {
            match request["method"].as_str()? {
                "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
                    let value =
                        decisions(request)
                            .into_iter()
                            .enumerate()
                            .find_map(|(index, value)| {
                                ((value == "accept" && id == "allow")
                                    || *id == format!("decision:{index}"))
                                .then_some(value)
                            })?;
                    Some((
                        if request["method"] == "item/fileChange/requestApproval" {
                            "thread-follower-file-approval-decision"
                        } else {
                            "thread-follower-command-approval-decision"
                        },
                        "decision",
                        value,
                    ))
                }
                "item/permissions/requestApproval" => {
                    let value = match id.as_str() {
                        "permissions:turn" => {
                            json!({"permissions":request["params"]["permissions"],"scope":"turn"})
                        }
                        "permissions:session" => {
                            json!({"permissions":request["params"]["permissions"],"scope":"session"})
                        }
                        "permissions:decline" => json!({"permissions":{},"scope":"turn"}),
                        _ => return None,
                    };
                    Some((
                        "thread-follower-permissions-request-approval-response",
                        "response",
                        value,
                    ))
                }
                _ => None,
            }
        }
        Response::Answers(answers)
            if request["method"] == "item/tool/requestUserInput"
                && controls.valid_answers(answers) =>
        {
            let answers: serde_json::Map<String, Value> = answers
                .iter()
                .map(|(id, answers)| (id.clone(), json!({"answers":answers})))
                .collect();
            Some((
                "thread-follower-submit-user-input",
                "response",
                json!({"answers":answers}),
            ))
        }
        _ => None,
    }
}

/// The owner compares request IDs with strict equality; preserve their JSON type.
pub fn follower_response(
    request: &Value,
    conversation: &str,
    response: &Response,
) -> Option<(&'static str, Value)> {
    let id = request.get("id")?;
    if !id.is_string() && !id.is_number() {
        return None;
    }
    let (method, field, value) = wire_response(request, response)?;
    let mut params = json!({"conversationId":conversation,"requestId":id});
    params[field] = value;
    Some((method, params))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offered_decisions_are_authoritative_and_preserve_amendments() {
        let request = json!({"method":"item/commandExecution/requestApproval","params":{"availableDecisions":["decline",{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["git","status"]}}]}});
        assert!(wire_response(&request, &Response::Action("allow".into())).is_none());
        let (_, field, value) =
            wire_response(&request, &Response::Action("decision:1".into())).unwrap();
        assert_eq!(field, "decision");
        assert_eq!(value, request["params"]["availableDecisions"][1]);
        assert_eq!(interaction(&request).actions.len(), 2);
    }
    #[test]
    fn policy_choices_show_readable_scope_and_keep_original_wire_value() {
        let decision = json!({"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["git","commit","-m","Remove Codex approval hook"]}});
        let request = json!({"method":"item/commandExecution/requestApproval","params":{"availableDecisions":["accept",decision,"decline"]}});
        let actions = interaction(&request).actions;
        assert_eq!(actions[1].label, "Allow similar commands");
        assert_eq!(
            actions[1].detail,
            "Future commands starting with: git commit -m 'Remove Codex approval hook'"
        );
        assert_eq!(
            wire_response(&request, &Response::Action(actions[1].id.clone()))
                .unwrap()
                .2,
            decision
        );
        assert!(actions[0].detail.is_empty());
        assert!(actions[2].detail.is_empty());
    }

    #[test]
    fn displayed_command_is_exact_and_separate_from_the_allowed_prefix() {
        let command = "git commit -m 'Hello world'\n";
        let request = json!({"method":"item/commandExecution/requestApproval","params":{"command":command,"availableDecisions":[{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["git","commit"]}}]}});
        let view = interaction(&request);
        assert_eq!(view.command.as_deref(), Some(command));
        assert_eq!(
            view.actions[0].detail,
            "Future commands starting with: git commit"
        );
        let mut args = request.clone();
        args["params"]["command"] = json!(["echo", "hello world"]);
        assert_eq!(
            interaction(&args).command.as_deref(),
            Some("echo 'hello world'")
        );
        args["params"]["command"] = json!(["echo", null]);
        assert!(interaction(&args).command.is_none());
        args["params"].as_object_mut().unwrap().remove("command");
        assert!(interaction(&args).command.is_none());
    }

    #[test]
    fn user_input_answers_use_question_ids_and_reject_unoffered_choices() {
        let request = json!({"method":"item/tool/requestUserInput","params":{"questions":[{"id":"q","question":"Pick?","isOther":false,"options":[{"label":"A","description":"first"}]}]}});
        let answer = |text: &str| {
            Response::Answers(std::collections::BTreeMap::from([(
                "q".into(),
                vec![text.into()],
            )]))
        };
        assert!(wire_response(&request, &answer("B")).is_none());
        let (method, field, value) = wire_response(&request, &answer("A")).unwrap();
        assert_eq!(method, "thread-follower-submit-user-input");
        assert_eq!(field, "response");
        assert_eq!(value, json!({"answers":{"q":{"answers":["A"]}}}));
    }
    #[test]
    fn permission_grants_never_add_unrequested_access() {
        let request = json!({"method":"item/permissions/requestApproval","params":{"permissions":{"network":{"enabled":true}}}});
        let (_, _, allow) =
            wire_response(&request, &Response::Action("permissions:turn".into())).unwrap();
        assert_eq!(allow["permissions"], request["params"]["permissions"]);
        assert_eq!(allow["scope"], "turn");
        let (_, _, deny) =
            wire_response(&request, &Response::Action("permissions:decline".into())).unwrap();
        assert_eq!(deny["permissions"], json!({}));
    }
    #[test]
    fn numeric_and_string_request_ids_keep_their_exact_wire_types() {
        for id in [json!(42), json!("42")] {
            let request = json!({"id":id,"method":"item/commandExecution/requestApproval","params":{"availableDecisions":["accept",{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["cargo","test"]}},"cancel"]}});
            let (method, params) =
                follower_response(&request, "chat", &Response::Action("allow".into())).unwrap();
            assert_eq!(method, "thread-follower-command-approval-decision");
            assert_eq!(params["requestId"], id);
            assert_eq!(params["decision"], "accept");
            assert_eq!(
                interaction(&request)
                    .actions
                    .iter()
                    .map(|action| action.label.as_str())
                    .collect::<Vec<_>>(),
                ["Allow once", "Allow similar commands", "Deny"]
            );
            assert_eq!(
                follower_response(&request, "chat", &Response::Action("decision:2".into()))
                    .unwrap()
                    .1["decision"],
                "cancel"
            );
        }
    }
    #[test]
    fn default_buttons_match_codex_and_do_not_invent_session_or_cancel_actions() {
        let command = json!({"method":"item/commandExecution/requestApproval","params":{}});
        assert_eq!(
            interaction(&command)
                .actions
                .iter()
                .map(|action| action.label.as_str())
                .collect::<Vec<_>>(),
            ["Allow once", "Deny"]
        );
        let file = json!({"method":"item/fileChange/requestApproval","params":{}});
        assert_eq!(
            interaction(&file)
                .actions
                .iter()
                .map(|action| action.label.as_str())
                .collect::<Vec<_>>(),
            ["Allow once", "Allow all edits", "Deny"]
        );
    }
}

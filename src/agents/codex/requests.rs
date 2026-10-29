//! Translate pending Codex requests into neutral controls and exact wire responses.
use crate::agents::interaction::{Action, Choice, Interaction, Question, Response};
use serde_json::{Value, json};
use std::collections::HashSet;

fn decisions(request: &Value) -> Vec<Value> {
    let params = &request["params"];
    if let Some(decisions) = params["availableDecisions"].as_array() {
        return decisions.clone();
    }
    let mut decisions = vec![
        json!("accept"),
        json!("acceptForSession"),
        json!("decline"),
        json!("cancel"),
    ];
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
    decisions
}
fn decision_label(value: &Value) -> Option<String> {
    match value.as_str() {
        Some("accept") => Some("Allow".into()),
        Some("acceptForSession") => Some("Allow for this session".into()),
        Some("decline") => Some("Decline".into()),
        Some("cancel") => Some("Cancel turn".into()),
        _ if value.get("acceptWithExecpolicyAmendment").is_some() => {
            Some("Allow similar commands".into())
        }
        _ if value.get("applyNetworkPolicyAmendment").is_some() => {
            let amendment = &value["applyNetworkPolicyAmendment"]["network_policy_amendment"];
            Some(format!(
                "Always {} {}",
                if amendment["action"] == "deny" {
                    "deny"
                } else {
                    "allow"
                },
                amendment["host"].as_str().unwrap_or("this host")
            ))
        }
        _ => None,
    }
}
pub fn interaction(request: &Value) -> Interaction {
    let params = &request["params"];
    let mut result = Interaction::default();
    match request["method"].as_str() {
        Some("item/commandExecution/requestApproval" | "item/fileChange/requestApproval") => {
            for (index, value) in decisions(request).into_iter().enumerate() {
                if let Some(label) = decision_label(&value) {
                    result.actions.push(Action {
                        id: if value == "accept" {
                            "allow".into()
                        } else {
                            format!("decision:{index}")
                        },
                        label,
                        detail: if value.is_object() {
                            serde_json::to_string_pretty(&value).unwrap_or_default()
                        } else {
                            String::new()
                        },
                    });
                }
            }
        }
        Some("item/permissions/requestApproval") if params["permissions"].is_object() => {
            let detail = serde_json::to_string_pretty(&params["permissions"]).unwrap_or_default();
            result.actions = vec![
                Action {
                    id: "permissions:turn".into(),
                    label: "Allow for this turn".into(),
                    detail: detail.clone(),
                },
                Action {
                    id: "permissions:session".into(),
                    label: "Allow for this session".into(),
                    detail,
                },
                Action {
                    id: "permissions:decline".into(),
                    label: "Decline".into(),
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
}

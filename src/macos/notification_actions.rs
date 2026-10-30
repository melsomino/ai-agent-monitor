//! Bind each notification to an immutable request and its offered actions.
use crate::agents::{
    interaction::{Interaction, Response},
    sessions::Session,
};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Reply(usize),
    Text,
    Form,
    More,
    Editor,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Button {
    pub title: String,
    pub kind: Kind,
}
#[derive(Clone, Debug)]
pub struct Plan {
    pub buttons: Vec<Button>,
    replies: Vec<Response>,
    text_question: Option<String>,
    category: String,
}
impl Plan {
    pub fn new(interaction: &Interaction) -> Self {
        let mut plan = Self {
            buttons: vec![],
            replies: vec![],
            text_question: None,
            category: next_category(),
        };
        for action in &interaction.actions {
            plan.reply(action.label.clone(), Response::Action(action.id.clone()));
        }
        if let [question] = interaction.questions.as_slice()
            && !question.secret
        {
            for choice in &question.choices {
                plan.reply(
                    choice.label.clone(),
                    Response::Answers(BTreeMap::from([(
                        question.id.clone(),
                        vec![choice.label.clone()],
                    )])),
                );
            }
            if question.freeform {
                plan.text_question = Some(question.id.clone());
                plan.buttons.push(Button {
                    title: if question.choices.is_empty() {
                        "Reply…"
                    } else {
                        "Other answer…"
                    }
                    .into(),
                    kind: Kind::Text,
                });
            }
        } else if !interaction.questions.is_empty() {
            plan.buttons.push(Button {
                title: "Answer questions…".into(),
                kind: Kind::Form,
            });
        }
        if plan.buttons.is_empty() {
            plan.buttons.push(Button {
                title: "Answer in editor…".into(),
                kind: Kind::Editor,
            });
        }
        // Four is the system detail-view budget; preserve an entry to all the
        // original choices instead of silently truncating the available answers.
        if plan.buttons.len() > 4 {
            plan.buttons.truncate(3);
            plan.buttons.push(Button {
                title: "More answers…".into(),
                kind: Kind::More,
            });
        }
        plan
    }
    fn reply(&mut self, title: String, response: Response) {
        let index = self.replies.len();
        self.replies.push(response);
        self.buttons.push(Button {
            title,
            kind: Kind::Reply(index),
        });
    }
    pub fn category(&self) -> String {
        self.category.clone()
    }
    pub fn response(&self, index: usize, text: Option<&str>) -> Option<Response> {
        match &self.buttons.get(index)?.kind {
            Kind::Reply(index) if text.is_none() => self.replies.get(*index).cloned(),
            Kind::Text => Some(Response::Answers(BTreeMap::from([(
                self.text_question.clone()?,
                vec![text.filter(|text| !text.trim().is_empty())?.to_owned()],
            )]))),
            _ => None,
        }
    }
}
fn next_category() -> String {
    format!(
        "AGENT_DYNAMIC_{}_{}",
        std::process::id(),
        crate::app::ids::nonce()
    )
}
#[derive(Clone)]
pub struct Binding {
    pub session: Session,
    pub plan: Plan,
}
#[derive(Default)]
pub struct Bindings {
    values: HashMap<String, Binding>,
}
impl Bindings {
    pub fn bind(&mut self, session: &Session) -> Option<Plan> {
        let key = session.approval_key()?;
        let plan = Plan::new(&session.interaction());
        self.values.insert(
            key,
            Binding {
                session: session.clone(),
                plan: plan.clone(),
            },
        );
        Some(plan)
    }
    pub fn lookup(&self, key: &str, category: &str) -> Option<Binding> {
        self.values
            .get(key)
            .filter(|b| b.plan.category() == category)
            .cloned()
    }
    pub fn remove(&mut self, key: &str) -> bool {
        self.values.remove(key).is_some()
    }
    pub fn plans(&self) -> Vec<Plan> {
        self.values.values().map(|b| b.plan.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::interaction::{Action, Choice, Question};
    fn question(count: usize) -> Interaction {
        Interaction {
            actions: vec![],
            questions: vec![Question {
                id: "q".into(),
                title: "Pick".into(),
                choices: (0..count)
                    .map(|i| Choice {
                        label: format!("Option {i}"),
                        description: String::new(),
                    })
                    .collect(),
                freeform: true,
                secret: false,
            }],
        }
    }
    #[test]
    fn three_choices_and_custom_text_fit_and_address_the_original_question() {
        let plan = Plan::new(&question(3));
        assert_eq!(plan.buttons.len(), 4);
        assert_eq!(
            plan.response(1, None),
            Some(Response::Answers(BTreeMap::from([(
                "q".into(),
                vec!["Option 1".into()]
            )])))
        );
        assert_eq!(
            plan.response(3, Some("Мой ответ")),
            Some(Response::Answers(BTreeMap::from([(
                "q".into(),
                vec!["Мой ответ".into()]
            )])))
        );
        assert!(plan.response(3, Some(" ")).is_none());
        assert!(plan.response(0, Some("forged")).is_none());
        assert!(plan.response(4, None).is_none());
    }
    #[test]
    fn large_secret_and_multi_question_prompts_keep_a_complete_form_route() {
        let mut interaction = question(5);
        let plan = Plan::new(&interaction);
        assert_eq!(plan.buttons.len(), 4);
        assert_eq!(plan.buttons[3].kind, Kind::More);
        interaction.questions[0].secret = true;
        assert_eq!(
            Plan::new(&interaction).buttons,
            vec![Button {
                title: "Answer questions…".into(),
                kind: Kind::Form
            }]
        );
        interaction.questions.push(interaction.questions[0].clone());
        assert_eq!(Plan::new(&interaction).buttons[0].kind, Kind::Form);
    }
    #[test]
    fn each_request_has_its_own_immutable_category_and_answers() {
        let first = Plan::new(&question(2));
        let mut other = question(2);
        other.questions[0].id = "other".into();
        let second = Plan::new(&other);
        assert_eq!(first.category(), first.clone().category());
        assert_ne!(first.category(), second.category());
        assert_ne!(first.response(0, None), second.response(0, None));
        other.questions[0].choices[0].label = "Different".into();
        assert_ne!(first.category(), Plan::new(&other).category());
        let plan = Plan::new(&Interaction {
            actions: vec![Action {
                id: "decline".into(),
                label: "Decline".into(),
                detail: String::new(),
            }],
            questions: vec![],
        });
        assert_eq!(
            plan.response(0, None),
            Some(Response::Action("decline".into()))
        );
    }
}

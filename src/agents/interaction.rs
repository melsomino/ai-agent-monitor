//! Provider-neutral actions and structured questions for a pending agent request.
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    Action(String),
    Answers(BTreeMap<String, Vec<String>>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub label: String,
    pub description: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub id: String,
    pub title: String,
    pub choices: Vec<Choice>,
    pub freeform: bool,
    pub secret: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Action {
    pub id: String,
    pub label: String,
    pub detail: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Interaction {
    pub actions: Vec<Action>,
    pub questions: Vec<Question>,
}
impl Interaction {
    pub fn allow() -> Self {
        Self {
            actions: vec![Action {
                id: "allow".into(),
                label: "Allow".into(),
                detail: String::new(),
            }],
            questions: vec![],
        }
    }
    pub fn valid_answers(&self, answers: &BTreeMap<String, Vec<String>>) -> bool {
        !self.questions.is_empty()
            && answers.len() == self.questions.len()
            && self.questions.iter().all(|question| {
                answers.get(&question.id).is_some_and(|values| {
                    values.len() == 1
                        && !values[0].trim().is_empty()
                        && (question.freeform
                            || question
                                .choices
                                .iter()
                                .any(|choice| choice.label == values[0]))
                })
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn structured_answers_require_all_questions_and_only_offered_choices() {
        let mut interaction = Interaction {
            actions: vec![],
            questions: vec![Question {
                id: "one".into(),
                title: "Choose".into(),
                choices: vec![Choice {
                    label: "Yes".into(),
                    description: String::new(),
                }],
                freeform: false,
                secret: false,
            }],
        };
        assert!(interaction.valid_answers(&BTreeMap::from([("one".into(), vec!["Yes".into()])])));
        assert!(!interaction.valid_answers(&BTreeMap::new()));
        assert!(!interaction.valid_answers(&BTreeMap::from([("one".into(), vec!["No".into()])])));
        interaction.questions[0].freeform = true;
        assert!(
            interaction.valid_answers(&BTreeMap::from([("one".into(), vec!["Custom".into()])]))
        );
        assert!(!interaction.valid_answers(&BTreeMap::from([("one".into(), vec![" ".into()])])));
    }
}

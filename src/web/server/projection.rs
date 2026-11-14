//! A provider-neutral, bounded projection for the shared HTML renderer.
use crate::{
    actors::monitor::{self, HistoryClient, HistoryKey, Submission},
    domain::{
        session::{Activity, Client, Session},
        session_activity::{MessageRole, RequestStatus, SessionActivity},
    },
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use serde_json::{Value, json};
pub fn request_context(session: &Session) -> Option<String> {
    use sha2::{Digest, Sha256};
    let request = session.pending_request()?;
    let bytes = serde_json::to_vec(&(
        session.key(),
        &request.approval.id,
        &request.approval.body,
        request.approval.expires,
        request.approval.can_allow,
        &request.interaction,
    ))
    .expect("serializable request");
    Some(format!("{:x}", Sha256::digest(bytes)))
}
pub fn html(text: &str, images: &[crate::domain::media::Image], session: &str) -> String {
    use pulldown_cmark::{Event, Options, Parser, html::push_html};
    let events = Parser::new_ext(text, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH).map(
        |event| match event {
            Event::Html(s) | Event::InlineHtml(s) => Event::Text(s),
            other => other,
        },
    );
    let mut out = String::new();
    push_html(&mut out, events);
    for image in images {
        out.push_str(&format!(
            "<p><img src=\"/api/image/{}/{}\" alt=\"Agent image\"></p>",
            URL_SAFE_NO_PAD.encode(session),
            URL_SAFE_NO_PAD.encode(&image.id)
        ));
    }
    out
}
fn icon(client: Client) -> String {
    let (color, label) = match client {
        Client::VSCode => ("#238cde", "&lt;/&gt;"),
        Client::Terminal => ("#34343e", "&gt;_"),
        _ => ("#7763ac", "AI"),
    };
    format!("data:image/svg+xml;base64,{}",STANDARD.encode(format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"32\" height=\"32\"><rect width=\"32\" height=\"32\" rx=\"8\" fill=\"{color}\"/><text x=\"16\" y=\"21\" text-anchor=\"middle\" font-family=\"sans-serif\" font-size=\"13\" fill=\"white\">{label}</text></svg>")))
}
fn session(s: &Session, state: &monitor::State, device: u64) -> Value {
    let key = s.key();
    let context = request_context(s);
    let submission = s
        .approval_key()
        .as_ref()
        .and_then(|k| state.submissions.get(k));
    let (phase, error) = match submission {
        Some(Submission::Sending) => ("sending", None),
        Some(Submission::Reconciling) => ("reconciling", None),
        Some(Submission::Failed(e)) => ("failed", Some(e)),
        None => ("ready", None),
    };
    let request=s.pending_request().map(|r|json!({"context":context,"body":r.approval.body,"command":r.interaction.command,"actions":r.interaction.actions,"questions":r.interaction.questions,"enabled":s.authoritative()&&r.approval.can_allow&&r.approval.expires>monitor::now(),"phase":phase,"error":error}));
    let activities:Vec<_>=s.activities.iter().map(|a| {
        let mut value=match a {
            SessionActivity::Message(m)=>json!({"type":"message","role":if m.role==MessageRole::Agent {"agent"}else{"user"},"html":html(&m.text,&m.images,&key)}),
            SessionActivity::Action(a)=>json!({"type":"action","text":a.text,"current":a.current,"status":format!("{:?}",a.status).to_lowercase()}),
            SessionActivity::Request(r)=>json!({"type":"request","status":if r.status==RequestStatus::Pending {"pending"}else{"resolved"},"request":if r.status==RequestStatus::Pending {request.clone()}else{None}}),
        };value["id"]=a.id().into();value["startedAt"]=a.started_at().to_rfc3339().into();value["updatedAt"]=a.updated_at().to_rfc3339().into();value
    }).collect();
    let history = state.history.get(&HistoryKey {
        session: key.clone(),
        client: HistoryClient::Web(device),
    });
    let history=history.map(|h|json!({"messages":h.messages.iter().map(|m|json!({"html":html(&m.text,&m.images,&key)})).collect::<Vec<_>>(),"loading":h.loading,"exhausted":h.exhausted,"error":h.error}));
    let status = match s.activity {
        Activity::Working => "working",
        Activity::Confirmation => "request",
        Activity::Unknown => "unknown",
        Activity::Idle if state.unseen_finished.contains_key(&key) => "finished",
        Activity::Idle => "idle",
    };
    let directory = s.cwd.to_string_lossy();
    let home = crate::app::core::home();
    let short = directory
        .strip_prefix(home.to_string_lossy().as_ref())
        .map(|tail| format!("~{tail}"))
        .unwrap_or_else(|| directory.to_string());
    json!({"id":key,"title":s.title,"app":s.client.label(),"appId":s.client.application_id(),"icon":icon(s.client),"status":status,"completion":state.unseen_finished.get(&s.key()),"activities":activities,"history":history,"directory":short,"fullDirectory":directory,"branch":s.git_branch,"model":s.model_label(),"authoritative":s.authoritative()&&s.can_send_message(),"expanded":status=="working"||status=="request"})
}
pub fn snapshot(state: &monitor::State, device: u64) -> Value {
    let sessions: Vec<_> = state
        .sessions
        .iter()
        .filter(|s| state.preferences.applications.enabled(s.client))
        .map(|s| session(s, state, device))
        .collect();
    let usage=state.usage.as_ref().map(|u|u.windows.iter().map(|w|json!({"label":w.label(),"remaining":w.remaining(monitor::now() as i64),"reset":w.resets_at.and_then(|t|chrono::DateTime::from_timestamp(t,0)).map(|t|t.format("%b %d").to_string()).unwrap_or_default()})).collect::<Vec<_>>()).unwrap_or_default();
    json!({"version":crate::app::core::VERSION,"revision":state.revision,"sessions":sessions,"usage":usage,"animate":state.preferences.animation,"error":state.discovery_error,"remote":true})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn messages_escape_raw_html_and_do_not_include_local_paths_for_images() {
        let rendered = html("<script>alert(1)</script>\n\n**Hello**", &[], "s");
        assert!(!rendered.contains("<script>"));
        assert!(rendered.contains("<strong>Hello</strong>"));
    }
}

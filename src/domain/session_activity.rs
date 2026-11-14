//! Bounded chronological session content. Identity, not type/text, defines updates.
use super::{interaction::Interaction, media::Image, session::Approval};
use chrono::{DateTime, Utc};
use std::{collections::VecDeque, time::SystemTime};

pub type ActivityId = String;
pub type ActivityTime = DateTime<Utc>;
pub const MAX_RECENT_ACTIVITIES: usize = 30;
pub const MAX_RECENT_TEXT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_RECENT_IMAGE_BYTES: usize = 40 * 1024 * 1024;
pub fn now() -> ActivityTime {
    SystemTime::now().into()
}
pub fn seconds(value: i64) -> ActivityTime {
    DateTime::from_timestamp(value, 0).unwrap_or(DateTime::UNIX_EPOCH)
}
pub fn millis(value: i64) -> ActivityTime {
    DateTime::from_timestamp_millis(value).unwrap_or(DateTime::UNIX_EPOCH)
}
pub fn timestamp(value: &serde_json::Value) -> Option<ActivityTime> {
    if let Some(text) = value.as_str() {
        DateTime::parse_from_rfc3339(text)
            .ok()
            .map(|t| t.with_timezone(&Utc))
    } else {
        value.as_i64().map(millis)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Agent,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Command,
    Tool,
    Reasoning,
    Other,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionStatus {
    Running,
    Completed,
    Failed,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestStatus {
    Pending,
    Answered,
    Superseded,
    Resolved,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionMessage {
    pub id: ActivityId,
    pub started_at: ActivityTime,
    pub updated_at: ActivityTime,
    pub role: MessageRole,
    pub text: String,
    pub images: Vec<Image>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionAction {
    pub id: ActivityId,
    pub started_at: ActivityTime,
    pub updated_at: ActivityTime,
    pub kind: ActionKind,
    /// Eligible for the live caption; execution status remains independent.
    pub current: bool,
    pub text: String,
    pub status: ActionStatus,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRequest {
    pub id: ActivityId,
    pub started_at: ActivityTime,
    pub updated_at: ActivityTime,
    /// Provider-owned, revalidated delivery ticket. Separate from activity identity.
    pub approval: Approval,
    pub interaction: Interaction,
    pub status: RequestStatus,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionActivity {
    Message(SessionMessage),
    Action(SessionAction),
    Request(SessionRequest),
}
impl SessionActivity {
    pub fn id(&self) -> &str {
        match self {
            Self::Message(v) => &v.id,
            Self::Action(v) => &v.id,
            Self::Request(v) => &v.id,
        }
    }
    pub fn started_at(&self) -> ActivityTime {
        match self {
            Self::Message(v) => v.started_at,
            Self::Action(v) => v.started_at,
            Self::Request(v) => v.started_at,
        }
    }
    pub fn updated_at(&self) -> ActivityTime {
        match self {
            Self::Message(v) => v.updated_at,
            Self::Action(v) => v.updated_at,
            Self::Request(v) => v.updated_at,
        }
    }
    pub fn set_times(&mut self, started: ActivityTime, updated: ActivityTime) {
        match self {
            Self::Message(v) => {
                v.started_at = started;
                v.updated_at = updated;
            }
            Self::Action(v) => {
                v.started_at = started;
                v.updated_at = updated;
            }
            Self::Request(v) => {
                v.started_at = started;
                v.updated_at = updated;
            }
        }
    }
    pub fn pending(&self) -> bool {
        matches!(self,Self::Request(v) if v.status==RequestStatus::Pending)
    }
    fn text_bytes(&self) -> usize {
        match self {
            Self::Message(v) => v.text.len(),
            Self::Action(v) => v.text.len(),
            Self::Request(v) => {
                v.approval.body.len()
                    + v.interaction.command.as_ref().map_or(0, String::len)
                    + v.interaction
                        .actions
                        .iter()
                        .map(|a| a.label.len() + a.detail.len())
                        .sum::<usize>()
                    + v.interaction
                        .questions
                        .iter()
                        .map(|q| {
                            q.title.len()
                                + q.choices
                                    .iter()
                                    .map(|c| c.label.len() + c.description.len())
                                    .sum::<usize>()
                        })
                        .sum::<usize>()
            }
        }
    }
}

/// Update by stable ID and retain the original start time. Ties sort by ID.
pub fn upsert(activities: &mut VecDeque<SessionActivity>, mut incoming: SessionActivity) {
    if let Some(index) = activities.iter().position(|old| old.id() == incoming.id()) {
        let old = &activities[index];
        if std::mem::discriminant(old) != std::mem::discriminant(&incoming) {
            return;
        }
        if incoming.updated_at() < old.updated_at() {
            return;
        }
        if let (SessionActivity::Request(previous), SessionActivity::Request(next)) =
            (old, &mut incoming)
            && previous.status != RequestStatus::Pending
            && next.status == RequestStatus::Pending
            && ((previous.approval == next.approval && previous.interaction == next.interaction)
                || (previous.approval.id.starts_with("transcript:")
                    && next.approval.id == previous.approval.id))
        {
            return;
        }
        if let (SessionActivity::Message(old), SessionActivity::Message(next)) =
            (old, &mut incoming)
            && next.images.is_empty()
            && !old.images.is_empty()
        {
            next.images = old.images.clone();
        }
        incoming.set_times(old.started_at(), incoming.updated_at());
        activities[index] = incoming;
    } else {
        activities.push_back(incoming);
    }
    activities.make_contiguous().sort_by(|a, b| {
        a.started_at()
            .cmp(&b.started_at())
            .then_with(|| a.id().cmp(b.id()))
    });
    trim(activities);
}
pub fn trim(activities: &mut VecDeque<SessionActivity>) {
    // Evict only a chronological prefix. A pending request is a barrier: never
    // skip over it to evict newer context, even when the soft budgets overflow.
    let mut normal = activities.iter().filter(|a| !a.pending()).count();
    let mut bytes: usize = activities
        .iter()
        .filter(|a| !a.pending())
        .map(SessionActivity::text_bytes)
        .sum();
    while normal > MAX_RECENT_ACTIVITIES || bytes > MAX_RECENT_TEXT_BYTES {
        if activities.front().is_none_or(SessionActivity::pending) {
            break;
        }
        if let Some(oldest) = activities.pop_front() {
            normal -= 1;
            bytes -= oldest.text_bytes();
        }
    }
    let mut image_bytes = 0;
    for activity in activities.iter_mut().rev() {
        if let SessionActivity::Message(message) = activity {
            let bytes: usize = message.images.iter().map(|i| i.data.len()).sum();
            if image_bytes + bytes > MAX_RECENT_IMAGE_BYTES {
                message.images.clear();
            } else {
                image_bytes += bytes;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message(id: &str, time: i64, text: &str) -> SessionActivity {
        SessionActivity::Message(SessionMessage {
            id: id.into(),
            started_at: seconds(time),
            updated_at: seconds(time),
            role: MessageRole::Agent,
            text: text.into(),
            images: vec![],
        })
    }
    fn request(id: &str, time: i64, status: RequestStatus) -> SessionActivity {
        SessionActivity::Request(SessionRequest {
            id: id.into(),
            started_at: seconds(time),
            updated_at: seconds(time),
            approval: Approval {
                id: id.into(),
                body: "Allow?".into(),
                expires: u64::MAX,
                can_allow: true,
            },
            interaction: Interaction::allow(),
            status,
        })
    }
    #[test]
    fn identical_events_are_distinct_and_ties_sort_by_id() {
        let mut recent = VecDeque::new();
        for id in ["c", "a", "b"] {
            upsert(&mut recent, message(id, 10, "Identical text"));
        }
        assert_eq!(
            recent.iter().map(SessionActivity::id).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert_eq!(recent.len(), 3);
    }
    #[test]
    fn updates_keep_start_and_position_and_reject_stale_payloads() {
        let mut recent = VecDeque::new();
        upsert(&mut recent, message("one", 1, "first"));
        upsert(&mut recent, message("two", 2, "second"));
        upsert(&mut recent, message("one", 30, "updated"));
        assert_eq!(recent[0].id(), "one");
        assert_eq!(recent[0].started_at(), seconds(1));
        assert_eq!(recent[0].updated_at(), seconds(30));
        upsert(&mut recent, message("one", 20, "stale"));
        assert!(matches!(&recent[0],SessionActivity::Message(m) if m.text=="updated"));
    }
    #[test]
    fn pending_request_blocks_eviction_of_the_entire_newer_tail() {
        let mut recent = VecDeque::new();
        upsert(&mut recent, request("pending", 0, RequestStatus::Pending));
        for i in 1..=100 {
            upsert(&mut recent, message(&format!("message:{i}"), i, "text"));
        }
        assert_eq!(recent.len(), 101);
        assert_eq!(recent[0].id(), "pending");
        assert_eq!(recent[1].started_at(), seconds(1));
        upsert(
            &mut recent,
            request("pending", 101, RequestStatus::Resolved),
        );
        assert_eq!(recent.len(), MAX_RECENT_ACTIVITIES);
        assert!(recent.iter().all(|a| a.id() != "pending"));
    }
    #[test]
    fn terminal_requests_are_not_reopened_by_delayed_pending_snapshots() {
        let mut recent = VecDeque::new();
        upsert(&mut recent, request("one", 1, RequestStatus::Answered));
        upsert(&mut recent, request("one", 2, RequestStatus::Pending));
        assert!(!recent[0].pending());
        let mut edited = request("one", 3, RequestStatus::Pending);
        if let SessionActivity::Request(r) = &mut edited {
            r.approval.body = "Edited permission".into();
        }
        upsert(&mut recent, edited);
        assert!(recent[0].pending());
    }
    #[test]
    fn text_budget_waits_for_the_pending_boundary_to_be_released() {
        let mut recent = VecDeque::new();
        upsert(&mut recent, request("pending", 0, RequestStatus::Pending));
        upsert(
            &mut recent,
            message("large", 1, &"x".repeat(MAX_RECENT_TEXT_BYTES + 1)),
        );
        assert_eq!(recent.len(), 2);
        assert!(recent[0].pending());
        upsert(&mut recent, request("pending", 2, RequestStatus::Answered));
        assert!(recent.is_empty());
    }
    #[test]
    fn eviction_removes_only_the_prefix_before_the_first_pending_boundary() {
        let mut recent = VecDeque::new();
        for i in 0..10 {
            upsert(&mut recent, message(&format!("old:{i}"), i, "old"));
        }
        upsert(&mut recent, request("first", 10, RequestStatus::Pending));
        for i in 11..100 {
            if i == 40 {
                upsert(&mut recent, request("second", i, RequestStatus::Pending));
            } else {
                upsert(&mut recent, message(&format!("new:{i}"), i, "new"));
            }
        }
        assert_eq!(recent.front().unwrap().id(), "first");
        assert_eq!(recent.len(), 90);
        upsert(&mut recent, request("first", 100, RequestStatus::Resolved));
        assert_eq!(recent.front().unwrap().id(), "second");
        assert_eq!(recent.len(), 60);
        upsert(&mut recent, request("second", 101, RequestStatus::Resolved));
        assert_eq!(recent.len(), MAX_RECENT_ACTIVITIES);
        assert_eq!(recent.front().unwrap().started_at(), seconds(70));
    }
    #[test]
    fn ordinary_history_still_obeys_both_budgets_without_a_barrier() {
        let mut recent = VecDeque::new();
        for i in 0..100 {
            upsert(&mut recent, message(&format!("message:{i}"), i, "text"));
        }
        assert_eq!(recent.len(), MAX_RECENT_ACTIVITIES);
        assert_eq!(recent.front().unwrap().started_at(), seconds(70));
        upsert(
            &mut recent,
            message("large", 100, &"x".repeat(MAX_RECENT_TEXT_BYTES + 1)),
        );
        assert!(recent.is_empty());
    }
}

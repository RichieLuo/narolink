use super::*;
use crate::engine::work::Step;

fn snapshot() -> Snapshot {
    Snapshot {
        run: RunSummary {
            id: "run".into(),
            provider: Provider::Claude,
            session_id: None,
            title: "Test".into(),
            model: None,
        },
        status: Status::Running,
        prompt: "Original prompt".into(),
        output: "Original reply".into(),
        draft: String::new(),
        work: Work::default(),
        images: vec![],
        local_files: vec![],
        activity: String::new(),
        active_permission_mode: None,
        error: None,
        approval: None,
        question: None,
        follow_ups: vec![],
    }
}

fn follow_up(id: &str, mode: SendMode) -> FollowUp {
    FollowUp {
        id: id.into(),
        mode,
        delivery: Delivery::Sent,
        prompt: id.into(),
        output: format!("Reply to {id}"),
        work: Work::default(),
        images: vec![],
        local_files: vec![],
        image_start: 0,
        options: None,
        error: None,
    }
}

#[test]
fn live_snapshot_exposes_only_the_labels_of_local_files() {
    let mut snapshot = snapshot();
    snapshot.local_files.push(attachments::LocalAttachment {
        path: "/private/work/notes.md".into(),
        directory: false,
    });
    let mut follow_up = follow_up("queued", SendMode::Queue);
    follow_up.local_files.push(attachments::LocalAttachment {
        path: r"C:\private\assets".into(),
        directory: true,
    });
    snapshot.follow_ups.push(follow_up);

    let public = serde_json::to_value(snapshot).unwrap();
    assert_eq!(
        public["local_files"],
        serde_json::json!([{"name": "notes.md", "directory": false}])
    );
    assert_eq!(
        public["follow_ups"][0]["local_files"],
        serde_json::json!([{"name": "assets", "directory": true}])
    );
}

#[test]
fn repeated_steering_keeps_previous_replies_before_their_follow_up() {
    let mut snapshot = snapshot();
    snapshot.follow_ups = vec![
        follow_up("steer-1", SendMode::Steer),
        follow_up("steer-2", SendMode::Steer),
        follow_up("queue", SendMode::Queue),
    ];
    snapshot.draft = "Streaming continuation".into();
    let replies = snapshot.replies();
    assert_eq!(replies.len(), 4);
    assert_eq!(replies[0].after, None);
    assert_eq!(replies[0].text, "Original reply");
    assert_eq!(replies[1].after, Some("steer-1"));
    assert_eq!(replies[1].text, "Reply to steer-1");
    assert_eq!(replies[2].after, Some("steer-2"));
    assert_eq!(replies[2].text, "Reply to steer-2");
    assert_eq!(replies[3].after, Some("queue"));
    assert_eq!(replies[3].text, "Reply to queue\n\nStreaming continuation");
}

#[test]
fn tool_finishing_after_steering_updates_its_original_reply() {
    let mut info = snapshot();
    info.work.record(Step {
        id: "tool".into(),
        kind: StepKind::Tool,
        status: StepStatus::Running,
        after: None,
        text_offset: Some(3),
        text: "Read".into(),
        exit_code: None,
    });
    info.follow_ups.push(follow_up("steer", SendMode::Steer));
    let mut state = State {
        info,
        composer_permission: None,
        images: HashMap::new(),
        next_image_index: 0,
        sent: 1,
        output_bytes: 0,
        approval: None,
        questions: VecDeque::new(),
        finished: None,
    };
    let mut step = state.info.work.steps[0].clone();
    step.status = StepStatus::Complete;
    step.after = Some("steer".into());
    step.text_offset = Some(20);
    state.work_for_step("tool").record(step);
    assert_eq!(state.info.work.steps[0].status, StepStatus::Complete);
    assert_eq!(state.info.work.steps[0].text_offset, Some(3));
    assert_eq!(state.info.work.steps[0].after, None);
    assert!(state.work().steps.is_empty());
    assert!(state.work_for_step("new-tool").steps.is_empty());
}

use super::*;
use std::cell::RefCell;

#[test]
fn streams_thinking_and_tools_without_repeating_text_or_exposing_signatures() {
    let cancel = AtomicBool::new(false);
    let work = RefCell::new(Work::default());
    let text = RefCell::new(String::new());
    let emit = |event| match event {
        Event::Step(step) => work.borrow_mut().record(step),
        Event::Text(delta) => text.borrow_mut().push_str(&delta),
        _ => {}
    };
    let turn = Turn {
        cancel: &cancel,
        images: &[],
        emit: &emit,
        next_steering: &|| None,
    };
    let mut output = Output::default();
    output.stream(
        &json!({"type":"message_start","message":{"id":"m1"}}),
        &turn,
    );
    output.stream(
        &json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}),
        &turn,
    );
    output.stream(&json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"Inspecting "}}), &turn);
    output.stream(&json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"the repository"}}), &turn);
    output.stream(&json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"private signature"}}), &turn);
    assert_eq!(work.borrow().steps[0].status, StepStatus::Running);
    assert_eq!(work.borrow().steps[0].text, "Inspecting the repository");
    output.stream(&json!({"type":"content_block_stop","index":0}), &turn);
    output.stream(&json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"Checking"}}), &turn);
    output.stream(&json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"t1","name":"Bash","input":{}}}), &turn);
    output.stream(&json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"command\":"}}), &turn);
    output.stream(&json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"\"git status --short\"}"}}), &turn);
    output.stream(&json!({"type":"content_block_stop","index":2}), &turn);
    let snapshot = json!({"id":"m1","content":[
        {"type":"thinking","thinking":"Inspecting the repository","signature":"private signature"},
        {"type":"text","text":"Checking the repository"},
        {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git status --short"}}
    ]});
    output.snapshot(&snapshot, &turn);
    assert_eq!(text.borrow().as_str(), "Checking the repository");
    assert_eq!(work.borrow().steps.len(), 2);
    assert_eq!(work.borrow().steps[0].status, StepStatus::Complete);
    assert_eq!(work.borrow().steps[0].kind, StepKind::Thinking);
    assert_eq!(work.borrow().steps[1].kind, StepKind::Command);
    assert_eq!(work.borrow().steps[1].text, "git status --short");
    assert_eq!(work.borrow().steps[1].status, StepStatus::Running);
    output.finish_tool("t1", true, &turn);
    output.snapshot(&snapshot, &turn);
    assert_eq!(work.borrow().steps[1].status, StepStatus::Failed);
    assert!(
        !serde_json::to_string(&*work.borrow())
            .unwrap()
            .contains("private")
    );
    output.snapshot(
        &json!({"id":"m2","content":[{"type":"text","text":"Done"}]}),
        &turn,
    );
    assert_eq!(text.borrow().as_str(), "Checking the repository\n\nDone");
}

#[test]
fn snapshots_capture_tools_without_partial_events_and_keep_bounds() {
    let cancel = AtomicBool::new(false);
    let work = RefCell::new(Work::default());
    let emit = |event| {
        if let Event::Step(step) = event {
            work.borrow_mut().record(step);
        }
    };
    let turn = Turn {
        cancel: &cancel,
        images: &[],
        emit: &emit,
        next_steering: &|| None,
    };
    let mut output = Output::default();
    output.snapshot(
        &json!({"id":"snapshot","content":[
            {"type":"tool_use","id":"read","name":"Read","input":{"file_path":"src/main.rs"}},
            {"type":"redacted_thinking","data":"secret","thinking":"hidden"},
            {"type":"thinking","thinking":"Visible snapshot","signature":"secret"}
        ]}),
        &turn,
    );
    output.finish_tool("read", false, &turn);
    assert_eq!(work.borrow().steps[0].text, "Read · src/main.rs");
    assert_eq!(work.borrow().steps[0].status, StepStatus::Complete);
    assert_eq!(work.borrow().steps.len(), 2);
    assert_eq!(work.borrow().steps[1].text, "Visible snapshot");
    assert!(
        !serde_json::to_string(&*work.borrow())
            .unwrap()
            .contains("secret")
    );
    let step = tool_step(&json!({"type":"tool_use","id":"large","name":"Bash","input":{"command":"中".repeat(MAX_STEP_TEXT)}}), StepStatus::Running).unwrap();
    assert!(step.text.len() <= MAX_STEP_TEXT);
    assert!(tool_step(&json!({"name":"Read"}), StepStatus::Running).is_none());
}

fn row(id: &str, parent: Option<&str>, kind: &str, content: Value) -> HistoryRow {
    let value =
        json!({"uuid":id,"parentUuid":parent,"type":kind,"message":{"id":id,"content":content}});
    HistoryRow {
        id: id.into(),
        parent: parent.map(str::to_owned),
        message: history_message(&value),
        results: tool_results(&value).collect(),
        notice: None,
        model: None,
    }
}

#[test]
fn history_merges_work_and_tool_results_with_their_reply() {
    let mut messages = Vec::new();
    let mut agent_ids = HashMap::new();
    for entry in [
        row("user", None, "user", json!("Check the code")),
        row(
            "thinking",
            Some("user"),
            "assistant",
            json!([
                {"type":"thinking","thinking":"Inspecting the existing code","signature":"private signature"},
                {"type":"redacted_thinking","data":"private data"},
                {"type":"text","text":"Checking"},
                {"type":"tool_use","id":"tool","name":"Read","input":{"file_path":"src/main.rs"}}
            ]),
        ),
        row(
            "result",
            Some("thinking"),
            "user",
            json!([
                {"type":"tool_result","tool_use_id":"tool","is_error":false,"content":"private file contents"}
            ]),
        ),
        row(
            "answer",
            Some("result"),
            "assistant",
            json!([{"type":"text","text":"Done"}]),
        ),
        row("follow-up", Some("answer"), "user", json!("Run tests")),
        row(
            "command",
            Some("follow-up"),
            "assistant",
            json!([
                {"type":"tool_use","id":"command","name":"Bash","input":{"command":"cargo test"}}
            ]),
        ),
        row(
            "failed",
            Some("command"),
            "user",
            json!([
                {"type":"tool_result","tool_use_id":"command","is_error":true,"content":"failed"}
            ]),
        ),
    ] {
        merge_history(&mut messages, entry, &mut agent_ids);
    }
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[1].text, "Checking\n\nDone");
    assert_eq!(messages[1].work.steps.len(), 2);
    assert_eq!(
        messages[1].work.steps[0].text,
        "Inspecting the existing code"
    );
    assert_eq!(messages[1].work.steps[1].status, StepStatus::Complete);
    assert_eq!(messages[3].work.steps[0].status, StepStatus::Failed);
    assert!(
        !serde_json::to_string(&messages)
            .unwrap()
            .contains("private")
    );
}

#[test]
fn history_filters_hidden_work_and_interrupts_unfinished_calls() {
    for flag in ["isMeta", "isSidechain", "isVisibleInTranscriptOnly"] {
        let mut value = json!({"type":"assistant","message":{"content":[
            {"type":"tool_use","id":"hidden","name":"Bash","input":{"command":"secret"}}
        ]}});
        value[flag] = json!(true);
        assert!(history_message(&value).is_none());
    }
    let mut messages = Vec::new();
    merge_history(
        &mut messages,
        row(
            "tool",
            None,
            "assistant",
            json!([
                {"type":"tool_use","id":"pending","name":"Glob","input":{"pattern":"**/*.rs"}}
            ]),
        ),
        &mut HashMap::new(),
    );
    messages[0].work.finish(false);
    assert_eq!(messages[0].work.steps[0].status, StepStatus::Interrupted);
}

#[test]
fn per_block_snapshots_do_not_repeat_streamed_text_after_thinking() {
    let cancel = AtomicBool::new(false);
    let text = RefCell::new(String::new());
    let emit = |event| {
        if let Event::Text(delta) = event {
            text.borrow_mut().push_str(&delta);
        }
    };
    let turn = Turn {
        cancel: &cancel,
        images: &[],
        emit: &emit,
        next_steering: &|| None,
    };
    let mut output = Output::default();
    output.stream(
        &json!({"type":"message_start","message":{"id":"m1"}}),
        &turn,
    );
    output.stream(
        &json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}),
        &turn,
    );
    output.stream(&json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"检查代码 🛠️"}}), &turn);
    let snapshot = json!({"id":"m1","content":[{"type":"text","text":"检查代码 🛠️，然后测试。"}]});
    output.snapshot(&snapshot, &turn);
    output.snapshot(&snapshot, &turn);
    assert_eq!(*text.borrow(), "检查代码 🛠️，然后测试。");
    output.reset_for_steer();
    output.snapshot(
        &json!({"id":"m2","content":[{"type":"text","text":"按新要求处理。"}]}),
        &turn,
    );
    assert_eq!(*text.borrow(), "检查代码 🛠️，然后测试。按新要求处理。");
}

#[test]
fn history_keeps_tools_between_the_text_that_surrounded_them() {
    let mut messages = vec![];
    let mut agent_ids = HashMap::new();
    for entry in [
        row(
            "first",
            None,
            "assistant",
            json!([
                {"type":"text","text":"先检查 🛠️"},
                {"type":"tool_use","id":"read","name":"Read","input":{"file_path":"src/main.rs"}}
            ]),
        ),
        row(
            "second",
            Some("first"),
            "assistant",
            json!([
                {"type":"text","text":"再修改"},
                {"type":"tool_use","id":"edit","name":"Edit","input":{"file_path":"src/main.rs"}},
                {"type":"text","text":"完成"}
            ]),
        ),
    ] {
        merge_history(&mut messages, entry, &mut agent_ids);
    }
    let entry = &messages[0];
    let parts = entry.work.reply_parts(&entry.text);
    let text_parts: Vec<_> = parts
        .iter()
        .filter_map(|part| {
            if let work::ReplyPart::Text { text, .. } = part {
                Some(*text)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(text_parts, ["先检查 🛠️", "\n\n再修改", "\n完成"]);
    assert_eq!(
        entry.work.steps[0].text_offset,
        Some("先检查 🛠️".chars().count())
    );
    assert_eq!(
        entry.work.steps[1].text_offset,
        Some("先检查 🛠️\n\n再修改".chars().count())
    );
}

#[test]
fn similar_text_blocks_keep_their_own_stream_indexes() {
    let cancel = AtomicBool::new(false);
    let text = RefCell::new(String::new());
    let emit = |event| {
        if let Event::Text(delta) = event {
            text.borrow_mut().push_str(&delta);
        }
    };
    let turn = Turn {
        cancel: &cancel,
        images: &[],
        emit: &emit,
        next_steering: &|| None,
    };
    let mut output = Output::default();
    output.stream(
        &json!({"type":"message_start","message":{"id":"same-prefix"}}),
        &turn,
    );
    output.stream(
        &json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}),
        &turn,
    );
    output.stream(
        &json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":"ab"}}),
        &turn,
    );
    let snapshot = json!({"id":"same-prefix","content":[{"type":"text","text":"abc"},{"type":"text","text":"ab"}]});
    output.snapshot(&snapshot, &turn);
    assert_eq!(*text.borrow(), "aabbc");
    output.snapshot(&snapshot, &turn);
    assert_eq!(*text.borrow(), "aabbc");
}

#[test]
fn per_block_snapshot_with_normalized_trailing_whitespace_does_not_repeat_text() {
    for (streamed, complete) in [
        ("检查完成。\n", "检查完成。"),
        ("检查完成。\r\n", "检查完成。"),
        ("检查完成。  ", "检查完成。"),
        ("检查完成。", "检查完成。\n"),
    ] {
        let cancel = AtomicBool::new(false);
        let text = RefCell::new(String::new());
        let emit = |event| {
            if let Event::Text(delta) = event {
                text.borrow_mut().push_str(&delta);
            }
        };
        let turn = Turn {
            cancel: &cancel,
            images: &[],
            emit: &emit,
            next_steering: &|| None,
        };
        let mut output = Output::default();
        output.stream(
            &json!({"type":"message_start","message":{"id":"normalized"}}),
            &turn,
        );
        output.stream(
            &json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}),
            &turn,
        );
        output.stream(&json!({"type":"content_block_delta","index":1,"delta":{"type":"text_delta","text":streamed}}), &turn);
        let snapshot = json!({"id":"normalized","content":[{"type":"text","text":complete}]});
        output.snapshot(&snapshot, &turn);
        output.snapshot(&snapshot, &turn);
        assert_eq!(
            text.borrow().trim_end(),
            "检查完成。",
            "streamed={streamed:?}, complete={complete:?}"
        );
    }
}

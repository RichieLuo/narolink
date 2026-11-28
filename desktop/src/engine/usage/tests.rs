use super::*;
use serde_json::json;
use std::{fs, io::Write};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("naro-usage-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn date(at: i64) -> String {
    chrono::DateTime::from_timestamp_nanos(at).to_rfc3339()
}

fn now() -> i64 {
    chrono::DateTime::<chrono::Utc>::from(SystemTime::now())
        .timestamp_nanos_opt()
        .unwrap()
}

fn write_rows(path: &Path, rows: &[Value]) {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    for row in rows {
        writeln!(file, "{row}").unwrap();
    }
}

#[test]
fn codex_prefers_request_records_and_counts_legacy_snapshots_once() {
    let temp = Fixture::new();
    let at = now();
    let timestamp = date(at - NANOS_PER_SECOND);
    let input = json!({
        "input_tokens": 120, "cached_input_tokens": 60,
        "cache_write_input_tokens": 20, "output_tokens": 30, "reasoning_output_tokens": 10
    });
    let record = json!({
        "timestamp": timestamp, "type": "token_usage_record",
        "payload": {"response_id": "req-1", "usage": input}
    });
    let snapshot = json!({
        "timestamp": timestamp, "type": "event_msg",
        "payload": {"type": "token_count", "info": {
            "total_token_usage": input, "last_token_usage": input
        }}
    });
    let old = json!({
        "timestamp": date(at - 3 * DAY), "type": "event_msg",
        "payload": {"type": "token_count", "info": {
            "total_token_usage": input, "last_token_usage": input
        }}
    });
    write_rows(
        &temp.0.join("rollout-new.jsonl"),
        &[record.clone(), snapshot.clone(), snapshot],
    );
    write_rows(&temp.0.join("rollout-old.jsonl"), &[old.clone(), old]);
    write_rows(&temp.0.join("rollout-fork.jsonl"), &[record]);
    let report = UsageCache::default().collect(at, [vec![temp.0.clone()], vec![], vec![]]);
    let counts = report.windows[0].providers[0].counts.unwrap();
    assert_eq!(
        counts,
        TokenCounts {
            input_tokens: 120,
            output_tokens: 30,
            cache_read_tokens: 60,
            cache_write_tokens: 20
        }
    );
    assert_eq!(report.windows[1].total.unwrap().total(), 300);
    assert_eq!(report.windows[2].total.unwrap().total(), 300);
}

#[test]
fn claude_merges_content_blocks_and_updates_when_a_log_grows() {
    let temp = Fixture::new();
    let at = now();
    let row = |id: &str, time, output| {
        json!({
            "timestamp": date(time),
            "type": "assistant",
            "message": {"id": id, "usage": {
                "input_tokens": 5, "cache_read_input_tokens": 50, "cache_creation_input_tokens": 15,
                "output_tokens": output
            }}
        })
    };
    let file = temp.0.join("session.jsonl");
    write_rows(
        &file,
        &[row("m1", at - DAY / 2, 4), row("old", at - 40 * DAY, 500)],
    );
    let mut cache = UsageCache::default();
    let mut load = || cache.collect(at, [vec![], vec![temp.0.clone()], vec![]]);
    assert_eq!(load().windows[0].total.unwrap().total(), 74);
    write_rows(
        &file,
        &[row("m1", at - DAY / 2, 35), row("m2", at - 10 * DAY, 1)],
    );
    let report = load();
    assert_eq!(report.windows[0].total.unwrap().total(), 105);
    assert_eq!(
        report.windows[0].providers[1]
            .counts
            .unwrap()
            .cache_write_tokens,
        15
    );
    assert_eq!(report.windows[1].total.unwrap().total(), 105);
    assert_eq!(report.windows[2].total.unwrap().total(), 176);
    assert_eq!(report.windows[2].providers[0].counts, None);
}

fn varint(mut number: u64, encoded: &mut Vec<u8>) {
    while number >= 128 {
        encoded.push((number as u8 & 127) | 128);
        number >>= 7;
    }
    encoded.push(number as u8);
}

fn protobuf_number(encoding: &mut Vec<u8>, number: u64, value: u64) {
    varint(number << 3, encoding);
    varint(value, encoding);
}

fn protobuf_bytes(encoding: &mut Vec<u8>, number: u64, value: &[u8]) {
    varint((number << 3) | 2, encoding);
    varint(value.len() as u64, encoding);
    encoding.extend_from_slice(value);
}

#[test]
fn antigravity_reads_timestamped_step_requests_without_double_counting_copies() {
    let temp = Fixture::new();
    let at = now();
    let mut timestamp = Vec::new();
    protobuf_number(
        &mut timestamp,
        1,
        ((at - NANOS_PER_SECOND) / NANOS_PER_SECOND) as u64,
    );
    let mut request = Vec::new();
    for (field, total) in [(2, 100), (3, 10), (4, 7), (5, 80), (9, 8), (10, 5)] {
        protobuf_number(&mut request, field, total);
    }
    protobuf_bytes(&mut request, 11, b"generation-1");
    let mut metadata = Vec::new();
    protobuf_bytes(&mut metadata, 8, &timestamp);
    protobuf_bytes(&mut metadata, 9, &request);
    for name in ["cli.db", "desktop.db"] {
        let db = rusqlite::Connection::open(temp.0.join(name)).unwrap();
        db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE steps (metadata BLOB);")
            .unwrap();
        db.execute("INSERT INTO steps VALUES (?1)", [&metadata])
            .unwrap();
    }
    let report = UsageCache::default().collect(at, [vec![], vec![], vec![temp.0.clone()]]);
    let provider = &report.windows[0].providers[2];
    assert!(!provider.partial);
    assert_eq!(
        provider.counts.unwrap(),
        TokenCounts {
            input_tokens: 187,
            output_tokens: 13,
            cache_read_tokens: 80,
            cache_write_tokens: 7
        }
    );
}

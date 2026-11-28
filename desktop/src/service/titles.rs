use crate::{engine::Provider, settings};
use rusqlite::{Connection, OpenFlags, params};
use std::{collections::HashMap, fs};

const FILE: &str = "session-titles.sqlite3";

fn provider_key(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::Codex => Some("codex"),
        Provider::Claude => Some("claude"),
        Provider::Antigravity => None, // ACP owns these titles.
    }
}

pub(super) fn for_provider(provider: Provider) -> HashMap<String, String> {
    let read = || -> Option<_> {
        let provider = provider_key(provider)?;
        let path = settings::state_dir().ok()?.join(FILE);
        let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
        let mut statement = db
            .prepare("SELECT session_id, title FROM session_titles WHERE provider = ?1")
            .ok()?;
        let titles = statement
            .query_map([provider], |row| Ok((row.get(0)?, row.get(1)?)))
            .ok()?;
        Some(titles.filter_map(Result::ok).collect())
    };
    read().unwrap_or_default()
}

pub(super) fn save(provider: Provider, id: &str, title: &str) -> Option<String> {
    let provider = provider_key(provider)?;
    let directory = settings::state_dir().ok()?;
    fs::create_dir_all(&directory).ok()?;
    let db = Connection::open(directory.join(FILE)).ok()?;
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS session_titles (
            provider TEXT NOT NULL,
            session_id TEXT NOT NULL,
            title TEXT NOT NULL,
            PRIMARY KEY (provider, session_id)
        );",
    )
    .ok()?;
    db.execute(
        "INSERT INTO session_titles (provider, session_id, title) VALUES (?1, ?2, ?3)
         ON CONFLICT (provider, session_id) DO NOTHING",
        params![provider, id, title],
    )
    .ok()?;
    db.query_row(
        "SELECT title FROM session_titles WHERE provider = ?1 AND session_id = ?2",
        params![provider, id],
        |row| row.get(0),
    )
    .ok()
}

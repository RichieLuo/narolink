use super::{Service, State};
use crate::{
    engine::{self, Provider},
    settings,
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::sync::MutexGuard;

#[derive(Deserialize, Serialize)]
pub struct PermissionTarget {
    pub session_id: Option<String>,
    pub run_id: Option<String>,
}

const DB_ERROR: &str = "无法读取或保存本机权限设置";
const NEW_SESSION: &str = "";

fn provider_key(provider: Provider) -> &'static str {
    match provider {
        Provider::Codex => "codex",
        Provider::Claude => "claude",
        Provider::Antigravity => "antigravity",
    }
}

impl Service {
    fn permission_db(&self) -> Result<MutexGuard<'_, Option<Connection>>, String> {
        let mut db = self.0.permissions.lock().unwrap();
        if db.is_none() {
            let connection = Connection::open(settings::state_dir()?.join("permissions.sqlite3"))
                .map_err(|_| DB_ERROR)?;
            connection
                .execute_batch(
                    "CREATE TABLE IF NOT EXISTS permissions (
                    provider TEXT NOT NULL,
                    session_id TEXT NOT NULL,
                    mode TEXT,
                    PRIMARY KEY (provider, session_id)
                );",
                )
                .map_err(|_| DB_ERROR)?;
            *db = Some(connection);
        }
        Ok(db)
    }

    pub fn permission_mode(
        &self,
        provider: Provider,
        target: &PermissionTarget,
    ) -> Result<Option<String>, String> {
        self.with_permission_target(provider, target, |session, run| {
            if let Some(run) = run {
                return Ok(run.composer_permission.clone());
            }
            self.permission_db()?
                .as_ref()
                .unwrap()
                .query_row(
                    "SELECT mode FROM permissions WHERE provider = ?1 AND session_id = ?2",
                    params![provider_key(provider), session.unwrap_or(NEW_SESSION)],
                    |row| row.get(0),
                )
                .optional()
                .map(Option::flatten)
                .map_err(|_| DB_ERROR.into())
        })
    }

    pub fn save_permission_mode(
        &self,
        provider: Provider,
        target: &PermissionTarget,
        mode: Option<&str>,
    ) -> Result<(), String> {
        engine::validate_permission_mode(provider, mode)?;
        self.with_permission_target(provider, target, |session, run| {
            let mut db = self.permission_db()?;
            let transaction = db.as_mut().unwrap().transaction().map_err(|_| DB_ERROR)?;
            for id in std::iter::once(NEW_SESSION).chain(session) {
                transaction
                    .execute(
                        "INSERT INTO permissions (provider, session_id, mode) VALUES (?1, ?2, ?3)
                     ON CONFLICT (provider, session_id) DO UPDATE SET mode = excluded.mode",
                        params![provider_key(provider), id, mode],
                    )
                    .map_err(|_| DB_ERROR)?;
            }
            transaction.commit().map_err(|_| DB_ERROR)?;
            if let Some(run) = run {
                run.composer_permission = mode.map(str::to_owned);
            }
            Ok(())
        })
    }

    pub(super) fn save_initial_permission(
        &self,
        provider: Provider,
        id: &str,
        mode: &str,
    ) -> Result<(), String> {
        self.permission_db()?.as_ref().unwrap().execute(
            "INSERT OR IGNORE INTO permissions (provider, session_id, mode) VALUES (?1, ?2, ?3)",
            params![provider_key(provider), id, mode],
        ).map_err(|_| DB_ERROR)?;
        Ok(())
    }

    fn with_permission_target<T>(
        &self,
        provider: Provider,
        target: &PermissionTarget,
        action: impl FnOnce(Option<&str>, Option<&mut State>) -> Result<T, String>,
    ) -> Result<T, String> {
        if let Some(id) = &target.session_id {
            if !engine::likely_id(id) {
                return Err("会话 ID 无效".into());
            }
            return action(Some(id), None);
        }
        if let Some(id) = &target.run_id {
            let run = self.run(id)?;
            let mut state = run.view.lock().unwrap();
            if state.info.run.provider != provider {
                return Err("运行任务与引擎不匹配".into());
            }
            if let Some(id) = &state.info.run.session_id {
                return action(Some(id), None);
            }
            return action(None, Some(&mut state));
        }
        action(None, None)
    }
}

//! Keeps capture sessions between runs, so a restart picks up a game that is still logged in.
//!
//! The game sends its session key once, at login, and everything after is encrypted with it. A
//! restarted archiver would see nothing until the next login. So each connection's key seed and
//! the exporter's state are kept in the warp database, and [`CaptureSession::resume_from`] picks
//! the connection up again once its traffic decrypts with the kept key.
//!
//! What's kept is the exporter's own state ([`ExporterState`]), not the game's commands, which
//! change from one game version to the next.
//!
//! [`CaptureSession::resume_from`]: crate::session::CaptureSession::resume_from
//! [`ExporterState`]: reliquary_archiver::export::fribbels::ExporterState

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use rusqlite::{Connection, params};
use tracing::{info, warn};

use crate::session::SavedSession;

/// Sessions older than this are dropped: the game has long since logged in again
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// The table lives beside the warp history, in the same file, but on its own: it isn't part of
/// the warp schema and its versions. A session that doesn't read back is dropped.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS capture_sessions (
    uid      INTEGER PRIMARY KEY,
    conv_id  INTEGER NOT NULL,
    -- The session key's seed, a u64 kept as its bits
    key_seed INTEGER NOT NULL,
    -- Unix seconds
    saved_at INTEGER NOT NULL,
    -- The exporter's state as JSON
    state    TEXT NOT NULL
);
";

pub struct ResumeStore {
    conn: Connection,
}

impl ResumeStore {
    pub fn open(path: &Path) -> rusqlite::Result<ResumeStore> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::setup(conn)
    }

    #[cfg(test)]
    fn open_in_memory() -> rusqlite::Result<ResumeStore> {
        Self::setup(Connection::open_in_memory()?)
    }

    fn setup(conn: Connection) -> rusqlite::Result<ResumeStore> {
        conn.execute_batch(SCHEMA)?;
        Ok(ResumeStore { conn })
    }

    /// The sessions kept, after dropping the ones too old to matter
    pub fn load(&self) -> rusqlite::Result<Vec<SavedSession>> {
        self.conn.execute("DELETE FROM capture_sessions WHERE saved_at < ?1", [
            now() - MAX_AGE.as_secs() as i64
        ])?;

        let mut statement = self.conn.prepare("SELECT uid, conv_id, key_seed, state FROM capture_sessions")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, u32>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;

        let mut sessions = Vec::new();
        for row in rows {
            let (uid, conv_id, seed, state) = row?;
            match serde_json::from_str(&state) {
                Ok(state) => sessions.push(SavedSession {
                    conv_id,
                    uid,
                    seed: seed as u64,
                    state,
                }),
                Err(e) => warn!(uid, %e, "kept capture session doesn't read back, dropping it"),
            }
        }
        Ok(sessions)
    }

    /// Keeps the session, in place of the account's last one
    pub fn save(&self, session: &SavedSession) -> rusqlite::Result<()> {
        let state = serde_json::to_string(&session.state).expect("exporter state serializes");
        self.conn.execute(
            "INSERT OR REPLACE INTO capture_sessions (uid, conv_id, key_seed, saved_at, state) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session.uid, session.conv_id, session.seed as i64, now(), state],
        )?;
        Ok(())
    }
}

/// Loads the sessions kept at `path` and starts a thread that keeps what it's sent. Sessions sent
/// while it is writing are coalesced, the newest of each account wins.
pub fn start(path: PathBuf) -> Option<(Vec<SavedSession>, mpsc::Sender<SavedSession>)> {
    let store = ResumeStore::open(&path)
        .inspect_err(|e| warn!(%e, "can't open the database to keep capture sessions in"))
        .ok()?;
    let saved = store
        .load()
        .inspect_err(|e| warn!(%e, "can't read kept capture sessions"))
        .unwrap_or_default();
    if !saved.is_empty() {
        info!(sessions = saved.len(), "found capture sessions from the last run");
    }

    let (tx, rx) = mpsc::channel::<SavedSession>();
    std::thread::Builder::new()
        .name("resume-store".into())
        .spawn(move || {
            while let Ok(first) = rx.recv() {
                let mut latest = HashMap::from([(first.uid, first)]);
                latest.extend(rx.try_iter().map(|session| (session.uid, session)));
                for session in latest.values() {
                    if let Err(e) = store.save(session) {
                        warn!(uid = session.uid, %e, "can't keep the capture session");
                    }
                }
            }
        })
        .ok()?;

    Some((saved, tx))
}

fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

#[cfg(test)]
mod tests {
    use reliquary_archiver::export::fribbels::OptimizerExporter;

    use super::*;

    #[test]
    fn a_kept_session_reads_back() {
        let store = ResumeStore::open_in_memory().unwrap();
        let session = SavedSession {
            conv_id: 0xdead_beef,
            uid: 600_000_001,
            seed: u64::MAX - 5,
            state: OptimizerExporter::new().state(),
        };
        store.save(&session).unwrap();
        store
            .save(&SavedSession {
                conv_id: 7,
                ..session.clone()
            })
            .unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.len(), 1, "an account keeps one session");
        assert_eq!((loaded[0].conv_id, loaded[0].uid, loaded[0].seed), (7, 600_000_001, u64::MAX - 5));
    }
}

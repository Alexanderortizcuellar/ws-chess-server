use rusqlite::{params, Connection};
use serde_json::json;
use std::sync::{Arc, Mutex};
use tokio::task;

use crate::error::{AppError, AppResult};

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone)]
pub struct StoredGame {
    pub game_id: String,
    pub headers_json: String,
    pub moves_uci: String,
    pub result: String,
    pub final_fen: String,
}

impl Db {
    pub fn new(path: &str) -> AppResult<Self> {
        let conn = Connection::open(path)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn init(&self) -> AppResult<()> {
        let conn = self.conn.clone();
        task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| AppError::Internal(e.to_string()))?;
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS games (
                    game_id      TEXT PRIMARY KEY,
                    headers_json TEXT NOT NULL,
                    moves_uci    TEXT NOT NULL,
                    result       TEXT NOT NULL,
                    final_fen    TEXT NOT NULL
                );
                "#,
            )?;
            Ok(())
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }

    pub async fn upsert_game(&self, g: StoredGame) -> AppResult<()> {
        let conn = self.conn.clone();
        task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| AppError::Internal(e.to_string()))?;
            conn.execute(
                r#"
                INSERT INTO games (game_id, headers_json, moves_uci, result, final_fen)
                VALUES (?1, ?2, ?3, ?4, ?5)
                ON CONFLICT(game_id) DO UPDATE SET
                    headers_json=excluded.headers_json,
                    moves_uci=excluded.moves_uci,
                    result=excluded.result,
                    final_fen=excluded.final_fen;
                "#,
                params![g.game_id, g.headers_json, g.moves_uci, g.result, g.final_fen],
            )?;
            Ok::<(), AppError>(())
        })
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
    }

    pub fn build_default_headers(game_id: &str, white: &str, black: &str, time_ms: i64) -> String {
        json!({
            "Event": "Online Game",
            "Site": "Local WS Server",
            "GameId": game_id,
            "White": white,
            "Black": black,
            "TimeControlMs": time_ms
        })
        .to_string()
    }
}

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
    routing::get,
    Router,
};
use futures_util::{sink::SinkExt, stream::StreamExt};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::{Arc, Mutex}, time::Duration};
use tokio::sync::{mpsc, Notify, RwLock};
use uuid::Uuid;

use crate::{
    error::{AppError, AppResult},
    game_logic::{self, GameCore, GameEvent, GameSnapshot, GameStatus},
    persistence::{Db, StoredGame},
    clock::{ChessClock, ClockConfig},
};

#[derive(Clone)]
pub struct AppState {
    pub(crate) db: Arc<Db>,
    pub(crate) lobby: Arc<RwLock<HashMap<String, Arc<Mutex<GameRoom>>>>>,
}

impl AppState {
    pub fn new(db: Arc<Db>) -> Self {
        Self { db, lobby: Arc::new(RwLock::new(HashMap::new())) }
    }
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new().route("/ws", get(ws_handler)).with_state(state)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

#[derive(Debug, Serialize)]
pub struct GameSummary {
    pub game_id: String,
    pub white: Option<String>,
    pub black: Option<String>,
    pub clock_config: ClockConfig,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientCommand {
    CreateGame { config: ClockConfig },
    JoinGame { game_id: String },
    MakeMove { game_id: String, uci: String },
    Resign { game_id: String },
    Abort { game_id: String },
    GetState { game_id: String },
    ListGames,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage {
    State { snapshot: GameSnapshot },
    Error { message: String, snapshot: Option<GameSnapshot> },
    GameList {
        challenges: Vec<GameSummary>,
        ongoing: Vec<GameSummary>,
    },
}

#[derive(Clone)]
struct ClientHandle {
    id: String,
    tx: mpsc::UnboundedSender<Message>,
}

pub(crate) struct GameRoom {
    id: String,
    core: GameCore,
    clock: ChessClock,
    white: Option<ClientHandle>,
    black: Option<ClientHandle>,
    spectators: Vec<ClientHandle>,
    notify: Arc<Notify>,
    clock_config: ClockConfig,

    ended: bool,
    result_str: String,
    terminal_status: Option<GameStatus>,
    terminal_events: Vec<GameEvent>,
}

impl GameRoom {
    fn new(id: String, config: ClockConfig) -> Self {
        let clock = ChessClock::new(config.clone());

        Self {
            id,
            core: GameCore::new(),
            clock,
            white: None,
            black: None,
            spectators: vec![],
            notify: Arc::new(Notify::new()),
            clock_config: config,
            ended: false,
            result_str: "*".into(),
            terminal_status: None,
            terminal_events: vec![],
        }
    }

    fn broadcast(&self, msg: &ServerMessage) {
        let text = serde_json::to_string(msg).unwrap();
        let m = Message::Text(text);
        if let Some(w) = &self.white { let _ = w.tx.send(m.clone()); }
        if let Some(b) = &self.black { let _ = b.tx.send(m.clone()); }
        for s in &self.spectators { let _ = s.tx.send(m.clone()); }
    }

    fn snapshot(&mut self) -> GameSnapshot {
        self.clock.consume_elapsed();

        let (status, events) = if let Some(ts) = self.terminal_status.clone() {
            (ts, self.terminal_events.clone())
        } else {
            let (status, events) = self.core.evaluate_status();
            match &status {
                GameStatus::Checkmate { winner } => {
                    self.ended = true;
                    self.result_str = if winner == "white" { "1-0" } else { "0-1" }.into();
                    self.clock.stop();
                    self.terminal_status = Some(status.clone());
                    self.terminal_events = events.clone();
                }
                GameStatus::Draw { .. } => {
                    self.ended = true;
                    self.result_str = "1/2-1/2".into();
                    self.clock.stop();
                    self.terminal_status = Some(status.clone());
                    self.terminal_events = events.clone();
                }
                _ => {}
            }
            (status, events)
        };

        let stm = self.core.side_to_move();
        GameSnapshot {
            game_id: self.id.clone(),
            fen: self.core.fen(),
            move_history: self.core.moves_uci.clone(),
            last_move: self.core.last_move.clone(),
            white_time_ms: self.clock.white_ms,
            black_time_ms: self.clock.black_ms,
            status,
            events,
            side_to_move: game_logic::color_to_str(stm).into(),
        }
    }
}

async fn handle_socket(socket: WebSocket, state: Arc<AppState>) {
    let (tx, mut rx_out) = mpsc::unbounded_channel::<Message>();
    let client_id = Uuid::new_v4().to_string();
    let client = ClientHandle { id: client_id, tx };

    let (mut ws_sender, mut ws_receiver) = socket.split();

    let forward = tokio::spawn(async move {
        while let Some(msg) = rx_out.recv().await {
            if ws_sender.send(msg).await.is_err() {
                break;
            }
        }
    });

    while let Some(msg) = ws_receiver.next().await {
        let msg = match msg {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!("WebSocket receive error: {:?}", e);
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                let cmd: Result<ClientCommand, _> = serde_json::from_str(&text);
                let cmd = match cmd {
                    Ok(c) => c,
                    Err(_) => {
                        let _ = send_error(&client, "Invalid JSON command", None);
                        continue;
                    }
                };

                if let Err(e) = process_command(cmd, &state, &client).await {
                    tracing::error!("Error processing command: {:?}", e);
                    let _ = send_error(&client, &e.to_string(), None);
                }
            }
            Message::Close(_) => break,
            Message::Ping(p) => {
                let _ = client.tx.send(Message::Pong(p));
            }
            _ => {}
        }
    }

    forward.abort();
}

async fn process_command(cmd: ClientCommand, state: &Arc<AppState>, client: &ClientHandle) -> AppResult<()> {
    match cmd {
        ClientCommand::CreateGame { config } => handle_create_game(config, state, client).await,
        ClientCommand::JoinGame { game_id } => handle_join_game(game_id, state, client).await,
        ClientCommand::GetState { game_id } => handle_get_state(game_id, state, client).await,
        ClientCommand::MakeMove { game_id, uci } => handle_make_move(game_id, uci, state, client).await,
        ClientCommand::Resign { game_id } => handle_resign(game_id, state, client).await,
        ClientCommand::Abort { game_id } => handle_abort(game_id, state, client).await,
        ClientCommand::ListGames => handle_list_games(state, client).await,
    }
}

async fn handle_list_games(state: &Arc<AppState>, client: &ClientHandle) -> AppResult<()> {
    let lobby = state.lobby.read().await;
    let mut challenges = Vec::new();
    let mut ongoing = Vec::new();

    for room_mutex in lobby.values() {
        let r = room_mutex.lock().map_err(|e| AppError::Internal(e.to_string()))?;
        if r.ended {
            continue;
        }

        let summary = GameSummary {
            game_id: r.id.clone(),
            white: r.white.as_ref().map(|c| format!("client-{}", &c.id[..8])),
            black: r.black.as_ref().map(|c| format!("client-{}", &c.id[..8])),
            clock_config: r.clock_config.clone(),
        };

        if r.white.is_none() || r.black.is_none() {
            challenges.push(summary);
        } else {
            ongoing.push(summary);
        }
    }

    let msg = ServerMessage::GameList {
        challenges,
        ongoing,
    };
    let text = serde_json::to_string(&msg)?;
    let _ = client.tx.send(Message::Text(text));
    Ok(())
}

async fn handle_create_game(config: ClockConfig, state: &Arc<AppState>, client: &ClientHandle) -> AppResult<()> {
    let game_id = Uuid::new_v4().to_string();
    let room = Arc::new(Mutex::new(GameRoom::new(game_id.clone(), config)));

    {
        let mut r = room.lock().map_err(|e| AppError::Internal(e.to_string()))?;
        r.white = Some(client.clone());
    }

    {
        let mut lobby = state.lobby.write().await;
        lobby.insert(game_id, room.clone());
    }

    spawn_timer_task(state.clone(), room.clone());

    let stored = {
        let mut r = room.lock().map_err(|e| AppError::Internal(e.to_string()))?;
        let snap = r.snapshot();
        r.broadcast(&ServerMessage::State { snapshot: snap });
        build_stored_game(&r)
    };
    state.db.upsert_game(stored).await?;
    Ok(())
}

async fn handle_join_game(game_id: String, state: &Arc<AppState>, client: &ClientHandle) -> AppResult<()> {
    let room = {
        let lobby = state.lobby.read().await;
        lobby.get(&game_id).cloned()
    };
    let Some(room) = room else {
        return send_error(client, "Game not found", None);
    };

    let stored = {
        let mut r = room.lock().map_err(|e| AppError::Internal(e.to_string()))?;
        let is_white = r.white.as_ref().map(|w| w.id.as_str()) == Some(&client.id);
        let is_black = r.black.as_ref().map(|b| b.id.as_str()) == Some(&client.id);

        if r.black.is_none() && !is_white {
            r.black = Some(client.clone());
        } else if r.white.is_none() && !is_black {
            r.white = Some(client.clone());
        } else if !is_white && !is_black {
            r.spectators.push(client.clone());
        }

        if r.white.is_some() && r.black.is_some() && !r.ended {
            r.clock.start();
        }

        let snap = r.snapshot();
        r.broadcast(&ServerMessage::State { snapshot: snap });
        build_stored_game(&r)
    };
    state.db.upsert_game(stored).await?;
    Ok(())
}

async fn handle_get_state(game_id: String, state: &Arc<AppState>, client: &ClientHandle) -> AppResult<()> {
    let room = {
        let lobby = state.lobby.read().await;
        lobby.get(&game_id).cloned()
    };
    let Some(room) = room else {
        return send_error(client, "Game not found", None);
    };
    {
        let mut r = room.lock().map_err(|e| AppError::Internal(e.to_string()))?;
        let snap = r.snapshot();
        let msg = ServerMessage::State { snapshot: snap };
        let text = serde_json::to_string(&msg)?;
        let _ = client.tx.send(Message::Text(text));
    }
    Ok(())
}

async fn handle_make_move(game_id: String, uci: String, state: &Arc<AppState>, client: &ClientHandle) -> AppResult<()> {
    let room = {
        let lobby = state.lobby.read().await;
        lobby.get(&game_id).cloned()
    };
    let Some(room) = room else {
        return send_error(client, "Game not found", None);
    };

    let to_persist = {
        let mut r = room.lock().map_err(|e| AppError::Internal(e.to_string()))?;
        if r.ended {
            return send_error(client, "Game already ended", Some(r.snapshot()));
        }

        let mover_color = if r.white.as_ref().map(|p| p.id.as_str()) == Some(&client.id) {
            chess::Color::White
        } else if r.black.as_ref().map(|p| p.id.as_str()) == Some(&client.id) {
            chess::Color::Black
        } else {
            return send_error(client, "Spectators cannot move", Some(r.snapshot()));
        };

        if r.core.side_to_move() != mover_color {
            return send_error(client, "Not your turn", Some(r.snapshot()));
        }

        if !r.clock.is_running() {
            r.clock.start();
        }

        r.clock.set_active(mover_color);
        r.clock.consume_elapsed();

        if r.clock.remaining_for(mover_color) <= 0 {
            end_by_time(&mut r, game_logic::opposite_color(mover_color));
        } else {
            match game_logic::parse_uci(&uci) {
                Ok(mv) => {
                    if let Err(e) = r.core.apply_move(mv, uci) {
                        return send_error(client, &e, Some(r.snapshot()));
                    } else {
                        r.clock.switch_turn();
                    }
                }
                Err(e) => return send_error(client, &e, Some(r.snapshot())),
            }
        }

        r.notify.notify_one();

        let snap = r.snapshot();
        r.broadcast(&ServerMessage::State { snapshot: snap });
        build_stored_game(&r)
    };

    state.db.upsert_game(to_persist).await?;
    Ok(())
}

async fn handle_resign(game_id: String, state: &Arc<AppState>, client: &ClientHandle) -> AppResult<()> {
    let room = {
        let lobby = state.lobby.read().await;
        lobby.get(&game_id).cloned()
    };
    let Some(room) = room else {
        return send_error(client, "Game not found", None);
    };

    let to_persist = {
        let mut r = room.lock().map_err(|e| AppError::Internal(e.to_string()))?;
        if r.ended {
            return send_error(client, "Game already ended", Some(r.snapshot()));
        }

        let resign_color = if r.white.as_ref().map(|p| p.id.as_str()) == Some(&client.id) {
            chess::Color::White
        } else if r.black.as_ref().map(|p| p.id.as_str()) == Some(&client.id) {
            chess::Color::Black
        } else {
            return send_error(client, "Spectators cannot resign", Some(r.snapshot()));
        };

        end_by_resign(&mut r, game_logic::opposite_color(resign_color));
        r.notify.notify_one();
        let snap = r.snapshot();
        r.broadcast(&ServerMessage::State { snapshot: snap });
        build_stored_game(&r)
    };

    state.db.upsert_game(to_persist).await?;
    Ok(())
}

async fn handle_abort(game_id: String, state: &Arc<AppState>, client: &ClientHandle) -> AppResult<()> {
    let room = {
        let lobby = state.lobby.read().await;
        lobby.get(&game_id).cloned()
    };
    let Some(room) = room else {
        return send_error(client, "Game not found", None);
    };

    {
        let mut r = room.lock().map_err(|e| AppError::Internal(e.to_string()))?;
        if r.ended {
            return send_error(client, "Game already ended", Some(r.snapshot()));
        }

        let is_player = r.white.as_ref().map(|p| p.id.as_str()) == Some(&client.id)
            || r.black.as_ref().map(|p| p.id.as_str()) == Some(&client.id);

        if !is_player {
            return send_error(client, "Only players can abort", Some(r.snapshot()));
        }

        end_by_abort(&mut r);
        r.notify.notify_one();
        let snap = r.snapshot();
        r.broadcast(&ServerMessage::State { snapshot: snap });
    }

    state.db.delete_game(game_id.clone()).await?;

    {
        let mut lobby = state.lobby.write().await;
        lobby.remove(&game_id);
    }

    Ok(())
}

fn send_error(client: &ClientHandle, msg: &str, snapshot: Option<GameSnapshot>) -> AppResult<()> {
    let payload = ServerMessage::Error { message: msg.into(), snapshot };
    let text = serde_json::to_string(&payload)?;
    let _ = client.tx.send(Message::Text(text));
    Ok(())
}

fn end_by_resign(room: &mut GameRoom, winner: chess::Color) {
    room.ended = true;
    room.clock.stop();
    let w = game_logic::color_to_str(winner).to_string();
    room.terminal_events = vec![GameEvent::Resignation { winner: w.clone() }];
    room.terminal_status = Some(GameStatus::Resigned { winner: w.clone() });
    room.result_str = if w == "white" { "1-0" } else { "0-1" }.into();
}

fn end_by_abort(room: &mut GameRoom) {
    room.ended = true;
    room.clock.stop();
    room.terminal_events = vec![GameEvent::Aborted];
    room.terminal_status = Some(GameStatus::Aborted);
    room.result_str = "0-0".into();
}

fn end_by_time(room: &mut GameRoom, winner: chess::Color) {
    room.ended = true;
    room.clock.stop();
    let w = game_logic::color_to_str(winner).to_string();
    room.terminal_events = vec![GameEvent::TimeExpiration { winner: w.clone() }];
    room.terminal_status = Some(GameStatus::TimeExpired { winner: w.clone() });
    room.result_str = if w == "white" { "1-0" } else { "0-1" }.into();
}

fn build_stored_game(room: &GameRoom) -> StoredGame {
    let white_name = room.white.as_ref().map(|c| format!("client-{}", &c.id[..8])).unwrap_or_else(|| "white".into());
    let black_name = room.black.as_ref().map(|c| format!("client-{}", &c.id[..8])).unwrap_or_else(|| "black".into());
    let headers_json = Db::build_default_headers(&room.id, &white_name, &black_name, room.clock_config.base_time_ms());
    StoredGame {
        game_id: room.id.clone(),
        headers_json,
        moves_uci: room.core.moves_uci.join(" "),
        result: room.result_str.clone(),
        final_fen: room.core.fen(),
    }
}

fn spawn_timer_task(state: Arc<AppState>, room: Arc<Mutex<GameRoom>>) {
    tokio::spawn(async move {
        loop {
            let (notify, dur, active_color, ended, running) = {
                let Ok(mut r) = room.lock() else { break; };
                r.clock.consume_elapsed();
                let ended = r.ended;
                let running = r.clock.is_running();
                let active = r.core.side_to_move();
                let dur = if running {
                    r.clock.active_deadline_duration()
                } else {
                    Duration::from_secs(1)
                };
                (r.notify.clone(), dur, active, ended, running)
            };

            if ended { break; }

            tokio::select! {
                _ = tokio::time::sleep(dur) => {
                    if running {
                        let maybe = {
                            let Ok(mut r) = room.lock() else { break; };
                            r.clock.set_active(active_color);
                            r.clock.consume_elapsed();
                            if r.clock.remaining_for(active_color) <= 0 && !r.ended {
                                end_by_time(&mut r, game_logic::opposite_color(active_color));
                                let snap = r.snapshot();
                                r.broadcast(&ServerMessage::State { snapshot: snap });
                                Some(build_stored_game(&r))
                            } else { None }
                        };
                        if let Some(stored) = maybe {
                            let _ = state.db.upsert_game(stored).await;
                        }
                    }
                }
                _ = notify.notified() => {}
            }
        }
    });
}

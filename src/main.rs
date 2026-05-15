mod error;
mod game_logic;
mod persistence;
mod server_lobby;
mod clock;

use std::net::SocketAddr;
use std::sync::Arc;

use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ws_chess_server=debug,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let db = Arc::new(persistence::Db::new("chess_games.sqlite").expect("Failed to open DB"));
    db.init().await.expect("DB init failed");

    let state = Arc::new(server_lobby::AppState::new(db));
    let app = server_lobby::router(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("WebSocket chess server listening on ws://{}/ws", addr);

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

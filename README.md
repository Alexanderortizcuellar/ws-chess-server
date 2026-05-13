# WebSocket Chess Server

A robust, real-time chess server built with Rust, Axum, and Tokio.

## Features
- **Real-time Gameplay:** WebSocket-based communication for low latency.
- **Full Chess Logic:** Powered by the `chess` crate (supports checkmate, stalemate, 50-move rule, threefold repetition).
- **Time Controls:** Integrated chess clocks for each game.
- **Persistence:** All games are saved to a SQLite database.
- **Concurrency:** Efficiently handles multiple simultaneous games using async tasks and robust locking.

## Setup & Running
1. **Install Rust:** Ensure you have the latest stable Rust toolchain.
2. **Run the Server:**
   ```bash
   cargo run
   ```
   The server will start listening on `ws://127.0.0.1:3000/ws`.

## Testing
- **Test Client:** Open `test_client.html` in your browser to interact with the server manually.
- **API Reference:** See [API_SPEC.md](./API_SPEC.md) for detailed command and message documentation.

## Architecture
- `src/main.rs`: Entry point and server initialization.
- `src/server_lobby.rs`: Manages game rooms, client connections, and command routing.
- `src/game_logic.rs`: Board state, move validation, and game rules.
- `src/time_notify.rs`: Precise chess clock management.
- `src/persistence.rs`: SQLite database interactions.
- `src/error.rs`: Custom error handling types.

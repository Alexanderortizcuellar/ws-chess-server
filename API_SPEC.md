# WebSocket Chess Server API Specification

This document defines the communication protocol for the WebSocket Chess Server.

## Connection
- **Endpoint:** `ws://<host>:<port>/ws`
- **Protocol:** JSON over WebSockets.
- **Heartbeats:** The server supports standard WebSocket Ping/Pong frames.

---

## Client Commands (Sent to Server)

All commands must be JSON objects with a `type` field.

### 1. `create_game`
Creates a new game room and assigns the sender as the White player.
- **Payload:**
  ```json
  {
    "type": "create_game",
    "time_ms": 600000
  }
  ```
- **`time_ms`**: Initial time for each player in milliseconds (e.g., 600000 for 10 minutes).

### 2. `join_game`
Joins an existing game. If the Black slot is empty, the sender becomes Black. Otherwise, the sender joins as a spectator.
- **Payload:**
  ```json
  {
    "type": "join_game",
    "game_id": "uuid-string"
  }
  ```

### 3. `make_move`
Attempts to play a move on the board.
- **Payload:**
  ```json
  {
    "type": "make_move",
    "game_id": "uuid-string",
    "uci": "e2e4"
  }
  ```
- **`uci`**: Standard Universal Chess Interface format (e.g., `e2e4`, `e7e8q`).

### 4. `resign`
Resigns the game for the player sending the command.
- **Payload:**
  ```json
  {
    "type": "resign",
    "game_id": "uuid-string"
  }
  ```

### 5. `get_state`
Requests the current full snapshot of the game.
- **Payload:**
  ```json
  {
    "type": "get_state",
    "game_id": "uuid-string"
  }
  ```

---

## Server Messages (Sent to Client)

### 1. `state`
Broadcasted whenever the game state changes (moves, joins, game end).
- **Payload Schema:**
  ```json
  {
    "type": "state",
    "snapshot": {
      "game_id": "string",
      "fen": "string",
      "move_history": ["string"],
      "last_move": "string | null",
      "white_time_ms": number,
      "black_time_ms": number,
      "status": "ongoing" | "check" | {"checkmate": {"winner": "string"}} | {"draw": {"reason": "string"}} | {"resigned": {"winner": "string"}} | {"time_expired": {"winner": "string"}},
      "events": [
        { "type": "check", "color_in_check": "string" },
        { "type": "checkmate", "winner": "string" },
        ...
      ],
      "side_to_move": "white" | "black"
    }
  }
  ```

### 2. `error`
Sent directly to a client when a command fails or is invalid.
- **Payload:**
  ```json
  {
    "type": "error",
    "message": "Error description string",
    "snapshot": { ... } // Optional: current state if the error happened during a move
  }
  ```

---

## Game Rules & Persistence
1. **Time Control:** The clock starts when the game is created. Time is consumed only when it is a player's turn.
2. **Persistence:** Game states are upserted to a SQLite database (`chess_games.sqlite`) after every move or terminal event.
3. **Terminal States:**
   - **Checkmate**: Winner identified by color.
   - **Stalemate / Draw**: 50-move rule, threefold repetition, and insufficient material are automatically detected.
   - **Time Out**: If a player's clock reaches 0, they lose immediately.

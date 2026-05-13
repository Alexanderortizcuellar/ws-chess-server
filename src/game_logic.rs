use chess::{Board, BoardStatus, ChessMove, Color, MoveGen, Piece};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameEvent {
    Check { color_in_check: String },
    Checkmate { winner: String },
    Draw { reason: String },
    Resignation { winner: String },
    TimeExpiration { winner: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameStatus {
    Ongoing,
    Check,
    Checkmate { winner: String },
    Draw { reason: String },
    Resigned { winner: String },
    TimeExpired { winner: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GameSnapshot {
    pub game_id: String,
    pub fen: String,
    pub move_history: Vec<String>,
    pub last_move: Option<String>,
    pub white_time_ms: i64,
    pub black_time_ms: i64,
    pub status: GameStatus,
    pub events: Vec<GameEvent>,
    pub side_to_move: String,
}

#[derive(Debug, Clone)]
pub struct GameCore {
    pub board: Board,
    pub moves_uci: Vec<String>,
    pub last_move: Option<String>,

    halfmove_clock: u32,
    repetition: HashMap<u64, u32>,
}

impl GameCore {
    pub fn new() -> Self {
        let board = Board::default();
        let mut repetition = HashMap::new();
        repetition.insert(board_key(&board), 1);
        Self {
            board,
            moves_uci: vec![],
            last_move: None,
            halfmove_clock: 0,
            repetition,
        }
    }

    pub fn fen(&self) -> String {
        self.board.to_string()
    }

    pub fn side_to_move(&self) -> Color {
        self.board.side_to_move()
    }

    pub fn apply_move(&mut self, mv: ChessMove, uci: String) -> Result<(), String> {
        if !self.is_legal_move(mv) {
            return Err("Illegal move".into());
        }

        let moved_piece = self.board.piece_on(mv.get_source());
        let dest_piece = self.board.piece_on(mv.get_dest());

        let is_capture = dest_piece.is_some() || is_en_passant(&self.board, mv, moved_piece);
        let is_pawn_move = moved_piece == Some(Piece::Pawn);

        if is_capture || is_pawn_move {
            self.halfmove_clock = 0;
            self.repetition.clear();
        } else {
            self.halfmove_clock += 1;
        }

        self.board = self.board.make_move_new(mv);
        self.last_move = Some(uci.clone());
        self.moves_uci.push(uci);

        let h = board_key(&self.board);
        *self.repetition.entry(h).or_insert(0) += 1;

        Ok(())
    }

    pub fn evaluate_status(&self) -> (GameStatus, Vec<GameEvent>) {
        let mut events = vec![];
        let status = match self.board.status() {
            BoardStatus::Ongoing => {
                if self.halfmove_clock >= 100 {
                    events.push(GameEvent::Draw { reason: "50-move rule".into() });
                    GameStatus::Draw { reason: "50-move rule".into() }
                } else if self.is_threefold() {
                    events.push(GameEvent::Draw { reason: "threefold repetition".into() });
                    GameStatus::Draw { reason: "threefold repetition".into() }
                } else if is_insufficient_material(&self.board) {
                    events.push(GameEvent::Draw { reason: "insufficient material".into() });
                    GameStatus::Draw { reason: "insufficient material".into() }
                } else if self.board.checkers().popcnt() > 0 {
                    events.push(GameEvent::Check { color_in_check: color_to_str(self.board.side_to_move()).into() });
                    GameStatus::Check
                } else {
                    GameStatus::Ongoing
                }
            }
            BoardStatus::Stalemate => {
                events.push(GameEvent::Draw { reason: "stalemate".into() });
                GameStatus::Draw { reason: "stalemate".into() }
            }
            BoardStatus::Checkmate => {
                let winner = match self.board.side_to_move() {
                    Color::White => "black",
                    Color::Black => "white",
                };
                events.push(GameEvent::Checkmate { winner: winner.into() });
                GameStatus::Checkmate { winner: winner.into() }
            }
        };
        (status, events)
    }

    pub fn is_legal_move(&self, mv: ChessMove) -> bool {
        MoveGen::new_legal(&self.board).any(|m| m == mv)
    }

    fn is_threefold(&self) -> bool {
        let h = board_key(&self.board);
        self.repetition.get(&h).copied().unwrap_or(0) >= 3
    }
}

pub fn parse_uci(uci: &str) -> Result<ChessMove, String> {
    ChessMove::from_str(uci).map_err(|e| e.to_string())
}

pub fn color_to_str(c: Color) -> &'static str {
    match c {
        Color::White => "white",
        Color::Black => "black",
    }
}

pub fn opposite_color(c: Color) -> Color {
    match c {
        Color::White => Color::Black,
        Color::Black => Color::White,
    }
}

fn board_key(board: &Board) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    board.hash(&mut hasher);
    hasher.finish()
}

fn is_en_passant(board: &Board, mv: ChessMove, moved_piece: Option<Piece>) -> bool {
    if moved_piece != Some(Piece::Pawn) {
        return false;
    }
    let src = mv.get_source();
    let dst = mv.get_dest();
    if board.piece_on(dst).is_some() {
        return false;
    }
    src.get_file().to_index() != dst.get_file().to_index()
}

fn is_insufficient_material(board: &Board) -> bool {
    use chess::ALL_SQUARES;

    let mut pieces = Vec::new();
    for &sq in ALL_SQUARES.iter() {
        if let Some(p) = board.piece_on(sq) {
            let c = board.color_on(sq).unwrap();
            pieces.push((c, p, sq));
        }
    }

    let non_kings: Vec<_> = pieces.iter().filter(|(_, p, _)| *p != Piece::King).collect();

    if non_kings.is_empty() {
        return true;
    }
    if non_kings.len() == 1 {
        let p = non_kings[0].1;
        return p == Piece::Knight || p == Piece::Bishop;
    }
    if non_kings.len() == 2 {
        let (c1, p1, s1) = *non_kings[0];
        let (c2, p2, s2) = *non_kings[1];
        if p1 == Piece::Bishop && p2 == Piece::Bishop && c1 != c2 {
            let col1 = (s1.get_file().to_index() + s1.get_rank().to_index()) % 2;
            let col2 = (s2.get_file().to_index() + s2.get_rank().to_index()) % 2;
            return col1 == col2;
        }
    }

    false
}

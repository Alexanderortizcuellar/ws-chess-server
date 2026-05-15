use chess::Color;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClockConfig {
    SuddenDeath { time_ms: i64 },
    Increment { time_ms: i64, increment_ms: i64 },
}

impl ClockConfig {
    pub fn base_time_ms(&self) -> i64 {
        match self {
            ClockConfig::SuddenDeath { time_ms } => *time_ms,
            ClockConfig::Increment { time_ms, .. } => *time_ms,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChessClock {
    pub white_ms: i64,
    pub black_ms: i64,
    config: ClockConfig,
    active: Color,
    running: bool,
    last_tick: Instant,
}

impl ChessClock {
    pub fn new(config: ClockConfig) -> Self {
        let base = config.base_time_ms();
        Self {
            white_ms: base,
            black_ms: base,
            config,
            active: Color::White,
            running: false,
            last_tick: Instant::now(),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn set_active(&mut self, color: Color) {
        self.active = color;
    }

    pub fn start(&mut self) {
        self.running = true;
        self.last_tick = Instant::now();
    }

    pub fn stop(&mut self) {
        self.consume_elapsed();
        self.running = false;
    }

    pub fn consume_elapsed(&mut self) {
        if !self.running {
            return;
        }
        let elapsed = Instant::now().duration_since(self.last_tick);
        let delta = elapsed.as_millis() as i64;
        match self.active {
            Color::White => self.white_ms -= delta,
            Color::Black => self.black_ms -= delta,
        }
        self.last_tick = Instant::now();
    }

    pub fn switch_turn(&mut self) {
        self.consume_elapsed();
        
        // Apply increment to the player who just finished their move
        if let ClockConfig::Increment { increment_ms, .. } = self.config {
            match self.active {
                Color::White => self.white_ms += increment_ms,
                Color::Black => self.black_ms += increment_ms,
            }
        }

        self.active = match self.active {
            Color::White => Color::Black,
            Color::Black => Color::White,
        };
        self.last_tick = Instant::now();
    }

    pub fn remaining_for(&mut self, color: Color) -> i64 {
        self.consume_elapsed();
        match color {
            Color::White => self.white_ms,
            Color::Black => self.black_ms,
        }
    }

    pub fn active_deadline_duration(&mut self) -> Duration {
        let rem = self.remaining_for(self.active).max(0) as u64;
        Duration::from_millis(rem)
    }
}

//! Une partie entre deux bots WebAssembly.

use traversal_core::config::{MOVE_LIMIT, SQUARES};
use traversal_core::{apply_move, legal_moves, new_game, Move, Rng, State};
use wasmtime::{Engine, Module};

use crate::sandbox::{Bot, Fault};

pub struct Outcome {
    pub moves: Vec<Move>,
    /// 1 rouge, 2 bleu, None : nulle.
    pub winner: Option<u8>,
    pub reason: String,
    /// Camp fautif et faute (partie perdue sur tapis vert).
    pub fault: Option<(u8, Fault)>,
    /// Fuel moyen par coup, par camp.
    pub fuel: [u64; 2],
    pub logs: [String; 2],
}

/// Graine du coup `ply` d'une partie : chaque coup a la sienne, la partie est reproductible.
fn move_seed(seed: u64, ply: u16) -> u64 {
    Rng::new(seed ^ (ply as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)).next_u64()
}

pub fn play(
    engine: &Engine,
    red: &Module,
    blue: &Module,
    heights: [u8; SQUARES],
    move_limit: u16,
    seed: u64,
    fuel: u64,
    verbose: bool,
) -> Outcome {
    let mut s: State = new_game(heights);
    s.move_limit = if move_limit == 0 { MOVE_LIMIT } else { move_limit };
    let mut out = Outcome { moves: Vec::new(), winner: None, reason: String::new(), fault: None, fuel: [0, 0], logs: Default::default() };
    let mut bots: [Option<Bot>; 2] = [None, None];
    for (i, module) in [red, blue].into_iter().enumerate() {
        match Bot::new(engine, module, verbose) {
            Ok(b) => bots[i] = Some(b),
            Err(f) => {
                out.fault = Some((i as u8 + 1, f));
                break;
            }
        }
    }
    if out.fault.is_none() {
        while !s.over {
            let moves = legal_moves(&s);
            let side = s.to_move;
            let bot = bots[side as usize - 1].as_mut().expect("bot instancié");
            // Un seul coup possible (passe forcée) : inutile de demander.
            let pick = if moves.len() == 1 { Ok(0) } else { bot.choose(&s, &moves, move_seed(seed, s.ply), fuel) };
            match pick {
                Ok(i) => {
                    out.moves.push(moves[i]);
                    s = apply_move(&s, &moves[i]);
                }
                Err(f) => {
                    out.fault = Some((side, f));
                    break;
                }
            }
        }
    }
    for (i, b) in bots.iter_mut().enumerate() {
        if let Some(b) = b {
            out.fuel[i] = b.fuel_used / b.calls.max(1);
            out.logs[i] = b.take_log();
        }
    }
    match &out.fault {
        Some((side, f)) => {
            out.winner = Some(3 - side);
            out.reason = if matches!(f, Fault::OutOfFuel) { "timeout" } else { "forfeit" }.into();
        }
        None => {
            out.winner = s.winner;
            out.reason = s.reason.map_or("?", |r| r.name()).into();
        }
    }
    out
}

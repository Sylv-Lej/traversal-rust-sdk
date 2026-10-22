//! Exemple d'architecture : un réseau de neurones très simple, entraîné en Python
//! (`train.py`), dont les poids (`weights.bin`) sont embarqués dans le module WebAssembly.
//!
//! - `features` : ce que voit le réseau (20 nombres, du point de vue du camp au trait) ;
//!   la même fonction sert à générer les données (`examples/dataset.rs`) et à jouer ;
//! - `Net` : 20 → 16 (ReLU) → 1 (tanh), en f32 ; la sortie estime le résultat de la
//!   partie (+1 gagnée, -1 perdue) ;
//! - la recherche : négamax à 2 demi-coups, toutes les réponses, le réseau aux feuilles.

use std::cell::RefCell;

use traversal_bot::core::bot::{exit_costs, WIN};
use traversal_bot::core::config::{Kind, SQUARES};
use traversal_bot::core::{apply_move, legal_moves, Move, State};
use traversal_bot::{bot, index_of, Turn};

pub const N_IN: usize = 20;

/// Les poids, produits par `train.py`.
static WEIGHTS: &[u8] = include_bytes!("../weights.bin");

/// Pour chaque camp (le camp au trait d'abord) et chaque type de pièce : nombre de pièces,
/// tours (au mieux, sur le relief seul) de la plus proche de la sortie, et somme des
/// `max(0, 7 - tours)` ; puis l'écart de score et le nombre de pièces sorties.
pub fn features(s: &State, costs: &[[u16; SQUARES]; 2]) -> [f32; N_IN] {
    let mut f = [0f32; N_IN];
    for k in 0..6 {
        f[k * 3 + 1] = 10.0; // aucune pièce : « très loin »
    }
    for p in s.pieces() {
        let side = if p.owner == s.to_move { 0 } else { 1 };
        let kind = match p.kind {
            Kind::Explorer => 0,
            Kind::Vanguard => 1,
            Kind::Sentinel => 2,
        };
        let m = p.kind.movement() as f32;
        let turns = (costs[p.owner as usize - 1][p.sq as usize] as f32 / m).ceil().min(10.0);
        let base = (side * 3 + kind) * 3;
        f[base] += 1.0;
        f[base + 1] = f[base + 1].min(turns);
        f[base + 2] += (7.0 - turns).max(0.0);
    }
    let (me, them) = (s.to_move as usize - 1, 2 - s.to_move as usize);
    f[18] = s.scores[me] as f32 - s.scores[them] as f32;
    f[19] = s.finished as f32;
    f
}

/// Réseau à une couche cachée. Format de `weights.bin` (petit-boutiste) : u32 entrées,
/// u32 neurones cachés, puis en f32 : W1 (cachés × entrées, par ligne), b1, W2, b2.
/// La normalisation des entrées est déjà repliée dans W1 et b1 par `train.py`.
pub struct Net {
    hidden: usize,
    w1: Vec<f32>,
    b1: Vec<f32>,
    w2: Vec<f32>,
    b2: f32,
}

impl Net {
    pub fn load(bytes: &[u8]) -> Net {
        let u32_at = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        assert_eq!(u32_at(0), N_IN, "weights.bin : nombre d'entrées");
        let hidden = u32_at(4);
        let floats: Vec<f32> = bytes[8..].chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())).collect();
        let (w1, rest) = floats.split_at(hidden * N_IN);
        let (b1, rest) = rest.split_at(hidden);
        let (w2, rest) = rest.split_at(hidden);
        Net { hidden, w1: w1.to_vec(), b1: b1.to_vec(), w2: w2.to_vec(), b2: rest[0] }
    }

    /// Résultat attendu pour le camp au trait, entre -1 et 1.
    pub fn forward(&self, x: &[f32; N_IN]) -> f32 {
        let mut out = self.b2;
        for h in 0..self.hidden {
            let row = &self.w1[h * N_IN..(h + 1) * N_IN];
            let z = self.b1[h] + row.iter().zip(x).map(|(w, v)| w * v).sum::<f32>();
            out += self.w2[h] * z.max(0.0);
        }
        out.tanh()
    }
}

struct Bot {
    net: Net,
    heights: [u8; SQUARES],
    costs: [[u16; SQUARES]; 2],
}

impl Bot {
    /// Note du point de vue du camp au trait.
    fn eval(&self, s: &State) -> i32 {
        if s.over {
            return match s.winner {
                None => 0,
                Some(w) if w == s.to_move => WIN,
                Some(_) => -WIN,
            };
        }
        (self.net.forward(&features(s, &self.costs)) * 1000.0) as i32
    }

    fn negamax(&self, s: &State, depth: u8) -> i32 {
        if depth == 0 || s.over {
            return self.eval(s);
        }
        legal_moves(s).iter().map(|m| -self.negamax(&apply_move(s, m), depth - 1)).max().unwrap_or(0)
    }

    fn choose(&self, s: &State) -> Move {
        let moves = legal_moves(s);
        *moves.iter().max_by_key(|m| -self.negamax(&apply_move(s, m), 1)).expect("au moins un coup")
    }
}

thread_local! {
    static BOT: RefCell<Option<Bot>> = const { RefCell::new(None) };
}

fn choose(turn: &Turn) -> usize {
    let s = &turn.state;
    BOT.with(|cell| {
        let mut slot = cell.borrow_mut();
        let bot = slot.get_or_insert_with(|| Bot { net: Net::load(WEIGHTS), heights: [u8::MAX; SQUARES], costs: [[0; SQUARES]; 2] });
        if bot.heights != s.heights {
            bot.heights = s.heights;
            bot.costs = [exit_costs(&s.heights, 1), exit_costs(&s.heights, 2)];
        }
        index_of(turn, &bot.choose(s))
    })
}

bot!(choose);

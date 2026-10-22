//! Données d'entraînement : parties du bot du site (niveau Sentier, avec du bruit pour
//! varier les positions) sur les 200 plateaux, deux parties par plateau. Une ligne par
//! position : résultat pour le camp au trait (+1, 0, -1), puis les `N_IN` caractéristiques.
//!
//! ```text
//! cargo run --release --example dataset > data.csv
//! ```

use std::time::Duration;

use traversal_bot::core::bot::{exit_costs, Tuning};
use traversal_bot::core::{apply_move, choose_move, legal_moves, new_game, Level, Rng};
use traversal_neural_bot::features;

fn main() {
    let boards = parse_boards(include_str!("../../../boards.json"));
    let level = Level { depth: 1, beam: 999, noise: 25.0, budget: Duration::from_secs(1), insight: 0, tuning: Tuning::default(), stop: None };
    let mut rng = Rng::new(1);
    for heights in &boards {
        let costs = [exit_costs(heights, 1), exit_costs(heights, 2)];
        for _ in 0..2 {
            let mut s = new_game(*heights);
            let mut rows = Vec::new();
            while !s.over {
                rows.push((s.to_move, features(&s, &costs)));
                // Premiers coups au hasard : des ouvertures variées.
                let mv = if s.ply < 4 {
                    let moves = legal_moves(&s);
                    moves[rng.below(moves.len())]
                } else {
                    choose_move(&s, level, &mut rng).mv
                };
                s = apply_move(&s, &mv);
            }
            for (side, f) in rows {
                let label = match s.winner {
                    None => 0,
                    Some(w) if w == side => 1,
                    Some(_) => -1,
                };
                let cols: Vec<String> = f.iter().map(|v| v.to_string()).collect();
                println!("{label},{}", cols.join(","));
            }
        }
    }
}

/// `boards.json` : une liste de 200 reliefs de 100 cases (sans dépendance JSON).
fn parse_boards(s: &str) -> Vec<[u8; 100]> {
    s.split(']')
        .map(|b| b.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).map(|x| x.parse().unwrap()).collect::<Vec<u8>>())
        .filter(|v| v.len() == 100)
        .map(|v| v.try_into().unwrap())
        .collect()
}

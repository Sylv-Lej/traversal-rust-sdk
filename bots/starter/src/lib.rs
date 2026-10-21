//! Bot de départ de l'arène : le bot du site, tel quel, borné par le fuel au lieu de
//! l'horloge. Point de départ conseillé : copier ce dossier et améliorer `choose`.
//!
//! Le niveau est choisi à la compilation (`TRAVERSAL_LEVEL=1` à `5`, Mont Blanc par
//! défaut) : c'est ainsi que sont construits les 5 bots maison de l'arène.
//!
//! ```text
//! cargo build --release --target wasm32-unknown-unknown -p traversal-starter-bot
//! # -> target/wasm32-unknown-unknown/release/traversal_starter_bot.wasm
//! ```

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use traversal_bot::core::{choose_move, Level, Rng};
use traversal_bot::{bot, fuel_left, index_of, log, Turn};

/// Mêmes réglages que les bots du site : profondeur, largeur, bruit, compréhension.
fn level() -> Level {
    let (depth, beam, noise, insight) = match option_env!("TRAVERSAL_LEVEL").unwrap_or("5") {
        "1" => (1, 999, 90.0, 0),
        "2" => (1, 999, 25.0, 0),
        "3" => (2, 12, 8.0, 1),
        "4" => (3, 9, 2.0, 2),
        _ => (4, 7, 0.0, 3),
    };
    Level { depth, beam, noise, budget: Duration::MAX, insight, tuning: Default::default(), stop: Some(out_of_fuel) }
}

/// En deçà, on arrête de chercher : de quoi finir proprement (le coup est perdu à zéro).
/// La recherche vérifie le fuel tous les 64 nœuds (≈ 20 M de fuel pour Mont Blanc).
static RESERVE: AtomicU64 = AtomicU64::new(0);

fn out_of_fuel() -> bool {
    fuel_left() < RESERVE.load(Ordering::Relaxed)
}

fn choose(turn: &Turn) -> usize {
    RESERVE.store(turn.fuel / 8, Ordering::Relaxed);
    let c = choose_move(&turn.state, level(), &mut Rng::new(turn.seed));
    // Visible avec `traversal-arena duel … --verbose` (ignoré en tournoi).
    log(&format!("demi-coup {} : profondeur {}, {} nœuds, fuel restant {} M", turn.state.ply, c.depth, c.nodes, fuel_left() / 1_000_000));
    index_of(turn, &c.mv)
}

bot!(choose);

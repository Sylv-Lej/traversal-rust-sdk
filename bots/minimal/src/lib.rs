//! Le plus petit bot de l'arène : l'exemple du README et de la page « Créer un bot » du
//! site, gardé ici pour qu'il compile toujours.

use traversal_bot::core::MoveKind;
use traversal_bot::{bot, Turn};

// turn.state : la position, turn.moves : les coups légaux, turn.seed : le hasard.
fn choose(turn: &Turn) -> usize {
    // Une sortie si possible, sinon le premier coup.
    turn.moves.iter().position(|m| m.kind == MoveKind::Exit).unwrap_or(0)
}

bot!(choose);

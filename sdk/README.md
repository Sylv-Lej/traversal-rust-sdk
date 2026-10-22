# traversal-bot

SDK des bots de l'**arène Traversal** : écris un bot, compile-le en WebAssembly, envoie-le
sur le site. Il affronte les autres bots en continu et grimpe au classement. Même
puissance de calcul pour tous : c'est l'algorithme qui gagne, pas la machine.

```toml
# Cargo.toml
[lib]
crate-type = ["cdylib"]

[dependencies]
traversal-bot = { git = "https://github.com/Sylv-Lej/traversal-rust-sdk", tag = "v0.2.0" }
```

```rust
use traversal_bot::core::MoveKind;
use traversal_bot::{bot, Turn};

// turn.state : la position, turn.moves : les coups légaux, turn.seed : le hasard.
fn choose(turn: &Turn) -> usize {
    // Une sortie si possible, sinon le premier coup.
    turn.moves.iter().position(|m| m.kind == MoveKind::Exit).unwrap_or(0)
}

bot!(choose);
```

```bash
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown
```

Le moteur complet est fourni (`traversal_bot::core` : coups légaux, `apply_move`, le bot
du site `choose_move`). `fuel_left()` donne le budget restant pour le coup (le fuel
≈ instructions WebAssembly : 2 milliards par coup ; à zéro, la partie est perdue).

Le SDK vit dans <https://github.com/Sylv-Lej/traversal-rust-sdk> (le `tag` fige la
version du moteur). Exemples, règles du bac à sable et outil de test en local (`traversal-arena duel`) :
même dépôt.

Licence MIT.

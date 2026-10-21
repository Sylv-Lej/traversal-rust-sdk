# traversal-core

Moteur du jeu de stratégie [Traversal](https://www.traversalgame.com/rules) : plateau
10 × 10 en relief, trois types de pièces, sortir ses pièces par le bord adverse.

- règles : coups légaux (`legal_moves`), application (`apply_move`), fin de partie ;
- bot de référence (`choose_move`) : alpha-beta à approfondissement itératif, celui du
  site (de Galet à Mont Blanc) ;
- format binaire des tours de l'arène des bots (`wire`).

Sans dépendance. Pour écrire un bot de l'arène, utiliser plutôt `traversal-bot` (même
dépôt : <https://github.com/Sylv-Lej/traversal-rust-sdk>), qui s'appuie sur ce crate.

```rust
use traversal_core::{apply_move, legal_moves, new_game};

let mut s = new_game([1; 100]); // plateau plat
while !s.over {
    let m = legal_moves(&s)[0];
    s = apply_move(&s, &m);
}
println!("vainqueur : {:?}", s.winner);
```

Licence MIT.

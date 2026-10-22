# Traversal — SDK des bots de l'arène

Écris un bot pour [Traversal](https://www.traversalgame.com/rules), compile-le en
WebAssembly, envoie-le sur la page **Arène** du site : il affronte les autres bots en
continu et grimpe au classement. Même puissance de calcul pour tous (le « fuel ») :
c'est l'algorithme qui gagne, pas la machine.

| | |
|---|---|
| `traversal-bot` (`sdk/`) | le SDK : ce que ton bot importe |
| `traversal-core` (`core/`) | le moteur : règles, coups légaux, bot de référence, format des tours (importé par le SDK, rien à déclarer) |
| `bots/starter/` | le bot du site (Galet à Mont Blanc), borné par le fuel : le point de départ conseillé |
| `bots/minimal/` | le plus petit bot possible |
| `bots/neural/` | un bot dont l'évaluation est un réseau de neurones entraîné en Python |
| `arena/` | `traversal-arena` : le bac à sable de l'arène, pour tester en local |

> Ce dépôt est un **miroir** généré depuis le dépôt du site, qui reste la référence (le
> moteur doit y rester identique à celui du serveur). Les propositions sont les
> bienvenues en issue ; elles sont reportées là-bas.

## Écrire un bot

```bash
cargo new --lib mon-bot && cd mon-bot
rustup target add wasm32-unknown-unknown
```

```toml
# Cargo.toml
[lib]
crate-type = ["cdylib"]

[dependencies]
traversal-bot = { git = "https://github.com/Sylv-Lej/traversal-rust-sdk", tag = "v0.2.0" }
```

```rust
// src/lib.rs
use traversal_bot::core::MoveKind;
use traversal_bot::{bot, Turn};

// turn.state : la position, turn.moves : les coups légaux, turn.seed : le hasard,
// turn.fuel : le budget de ce coup.
fn choose(turn: &Turn) -> usize {
    turn.moves.iter().position(|m| m.kind == MoveKind::Exit).unwrap_or(0)
}

bot!(choose);
```

```bash
cargo build --release --target wasm32-unknown-unknown
# -> target/wasm32-unknown-unknown/release/mon_bot.wasm : à envoyer sur le site
```

Le `tag` fige la version du moteur : prends le plus récent (onglet *Tags* du dépôt). Une
règle qui change côté site donne un nouveau tag.

Pour aller plus loin, copie `bots/starter` : c'est le bot du site lui-même
(`traversal_bot::core::choose_move`), qui s'arrête quand son fuel baisse
(`traversal_bot::fuel_left()`). `traversal_bot::log("…")` écrit un message visible en
local avec `--verbose`.

## Écrire un bot en Python

Un interpréteur Python ne tient pas dans le module de 1 Mo de l'arène. Python sert donc
à **entraîner** le bot, et Rust à le **faire jouer** : on entraîne un réseau de neurones
en Python, on exporte ses poids dans un fichier, et un petit bot Rust les embarque à la
compilation. C'est ce que fait `bots/neural`, un exemple très simple (353 paramètres,
numpy seul) qui montre toute la chaîne :

```text
 examples/dataset.rs   ──►   train.py      ──►   src/lib.rs
 (Rust natif)                (Python)            (Rust → WebAssembly)
 parties d'auto-jeu          entraîne le         include_bytes!("weights.bin")
 → data.csv                  réseau              + calcul du réseau
                             → weights.bin       → le .wasm à envoyer
```

```bash
cd bots/neural
cargo run --release --example dataset > data.csv    # 1. les données
python3 train.py                                    # 2. l'entraînement -> weights.bin
cargo build --release --target wasm32-unknown-unknown   # 3. le bot, poids compris
# -> target/wasm32-unknown-unknown/release/traversal_neural_bot.wasm
```

Côté Python, on change les données et le réseau ; côté Rust, il n'y a qu'à garder la
même fonction `features` (ce que voit le réseau) et le même format de `weights.bin`.
Les détails (format des poids, limites de taille et de fuel, pistes pour un vrai réseau)
sont dans [`bots/neural/README.md`](bots/neural/README.md).

## Tester en local

```bash
docker build -f arena/Dockerfile -t traversal-arena .
docker run --rm -v "$PWD":/w traversal-arena check /w/mon_bot.wasm               # valide ?
docker run --rm -v "$PWD":/w traversal-arena duel /w/mon_bot.wasm sommet --games 20
```

Un adversaire est un `.wasm` ou un bot du site : `galet`, `sentier`, `crete`, `sommet`,
`montblanc`. Chaque carte est jouée deux fois, couleurs inversées ; la sortie donne
l'écart d'Elo, les fautes et le fuel moyen par coup.

## Règles du bac à sable

- **2 milliards de fuel par coup** (≈ instructions WebAssembly), remis à neuf à chaque
  coup. À zéro, la partie est perdue.
- **64 Mo de mémoire**, module de **1 Mo** au plus, pas de threads.
- **Aucun accès au système** : ni fichiers, ni réseau, ni horloge. Le hasard vient de la
  graine fournie à chaque coup : une partie se rejoue à l'identique.
- Un plantage ou un indice de coup hors de la liste fait perdre la partie.
- La mémoire du bot persiste d'un coup à l'autre pendant une partie.

## Dans un autre langage (C, Zig, Go…)

Le module exporte `memory`, `traversal_buffer() -> i32` (adresse d'un tampon de 16 Ko
où l'arène écrit chaque tour) et `traversal_move(len: i32) -> i32` (renvoie l'indice du
coup choisi). Seuls imports permis : `traversal.fuel_left() -> i64` et
`traversal.log(ptr: i32, len: i32)`. Format du tour : en-tête de
[`core/src/wire.rs`](core/src/wire.rs).

## Licence

MIT.

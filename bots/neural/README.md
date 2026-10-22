# Bot « neural » — exemple de neural net très simple

**Exemple d'architecture, pas un bot compétitif** : un réseau minuscule (353 paramètres)
entraîné en Python, dont les poids sont compilés dans le bot Rust. Le but est de montrer
la chaîne complète, à reprendre avec un vrai réseau.

```text
 examples/dataset.rs        train.py              src/lib.rs
 (Rust natif)               (Python, numpy)       (Rust → WebAssembly)
 auto-jeu sur les    ──►    entraîne le     ──►   include_bytes!("weights.bin")
 200 plateaux               réseau                + inférence à la main
        │                       │                        │
        ▼                       ▼                        ▼
    data.csv               weights.bin          traversal_neural_bot.wasm
```

- **`features`** (`src/lib.rs`) : 20 nombres par position, du point de vue du camp au
  trait (par camp et par type de pièce : nombre, tours de la plus proche de la sortie,
  avance cumulée ; puis écart de score, pièces sorties). La **même fonction** sert à
  générer les données et à jouer : c'est ce qui garantit que le réseau voit en jeu ce
  qu'il a vu à l'entraînement.
- **Réseau** : 20 → 16 (ReLU) → 1 (tanh), en f32. Il prédit le résultat de la partie
  pour le camp au trait (+1 gagnée, -1 perdue).
- **`weights.bin`** : u32 entrées, u32 neurones cachés, puis en f32 petit-boutiste W1
  (par ligne), b1, W2, b2. La normalisation des entrées est repliée dans W1 et b1 par
  `train.py` : le Rust n'a qu'un calcul brut à faire.
- **Recherche** : négamax à 2 demi-coups, toutes les réponses, le réseau aux feuilles.

## Refaire toute la chaîne

Depuis la racine du dépôt. `weights.bin` est fourni : l'étape 3 suffit pour compiler le
bot tel quel. Sans Rust en local, tout passe par Docker :

```bash
# image Rust avec la cible WebAssembly (une seule fois)
printf 'FROM rust:1.88-slim\nRUN rustup target add wasm32-unknown-unknown\n' | docker build -t traversal-builder -

# 1. données : ~13 000 positions de parties du bot Sentier (quelques secondes)
docker run --rm -v "$PWD":/src -w /src/bots/neural traversal-builder \
  sh -c 'cargo run --release --example dataset > data.csv'

# 2. entraînement : data.csv -> weights.bin
(cd bots/neural && python3 train.py)

# 3. bot : les poids sont embarqués à la compilation
docker run --rm -v "$PWD":/src -w /src/bots/neural traversal-builder \
  cargo build --release --target wasm32-unknown-unknown
docker run --rm -v "$PWD":/w traversal-arena duel \
  /w/bots/neural/target/wasm32-unknown-unknown/release/traversal_neural_bot.wasm sentier --games 100
```

## Résultat de l'exemple

Sur les données de validation, il désigne le bon vainqueur dans 65 % des parties
décidées. Dans l'arène (100 parties, 26 M de fuel par coup, soit 1 % du budget) :

| adversaire | score | Elo |
|---|---|---|
| galet | 92 % | +424 ± 200 |
| sentier | 78 % | +220 ± 97 |
| crete | 42 % | −56 ± 68 |

## Pour aller plus loin

- **Taille** : module de 1 Mo au plus, poids compris. Cela fait ~225 000 paramètres en
  f32, ~900 000 en int8 (quantification).
- **Coût** : le fuel compte les instructions WebAssembly. Une multiplication-addition
  coûte ~4-5 unités, et le SIMD est refusé par l'arène : garder un petit réseau, ou
  des entrées creuses (ci-dessous).
- **Entrées** : le relief change à chaque partie, il faut donc des entrées qui en
  dépendent (ici, les tours jusqu'à la sortie). Pour un grand réseau, utiliser des
  entrées creuses mises à jour à chaque coup (style NNUE).
- **Étiquettes** : mélanger le résultat final et le score d'une recherche ; générer les
  parties avec un bot plus fort, réentraîner, valider chaque version dans l'arène.

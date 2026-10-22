"""Entraîne le réseau du bot (20 -> 16 ReLU -> 1 tanh) sur data.csv, écrit weights.bin.

    python3 train.py            # data.csv -> weights.bin
    python3 train.py --zero     # réseau nul : de quoi compiler avant les premières données

numpy seul. La normalisation des entrées est repliée dans la première couche : le Rust
n'a qu'à faire le calcul brut. Format : voir `Net` dans src/lib.rs.
"""

import struct
import sys

import numpy as np

N_IN, HIDDEN = 20, 16


def save(path, w1, b1, w2, b2):
    with open(path, "wb") as f:
        f.write(struct.pack("<II", N_IN, HIDDEN))
        for a in (w1, b1, w2, np.array([b2])):
            f.write(np.asarray(a, dtype="<f4").tobytes())


def main():
    if "--zero" in sys.argv:
        save("weights.bin", np.zeros((HIDDEN, N_IN)), np.zeros(HIDDEN), np.zeros(HIDDEN), 0.0)
        return

    # numpy + Accelerate (macOS) signale à tort des débordements dans les produits
    # matriciels ; une vraie divergence est détectée par l'assertion de fin.
    np.seterr(all="ignore")
    data = np.loadtxt("data.csv", delimiter=",", dtype=np.float64)
    y, x = data[:, 0], data[:, 1:]
    mu, sd = x.mean(0), x.std(0) + 1e-6
    xn = (x - mu) / sd

    rng = np.random.default_rng(0)
    idx = rng.permutation(len(y))
    split = int(0.9 * len(y))
    tr, va = idx[:split], idx[split:]

    w1 = rng.normal(0, 1 / np.sqrt(N_IN), (HIDDEN, N_IN))
    b1 = np.zeros(HIDDEN)
    w2 = rng.normal(0, 1 / np.sqrt(HIDDEN), HIDDEN)
    b2 = 0.0

    def forward(xb):
        h = np.maximum(0, xb @ w1.T + b1)
        return h, np.tanh(h @ w2 + b2)

    lr, batch = 0.01, 256
    for epoch in range(30):
        rng.shuffle(tr)
        for i in range(0, len(tr), batch):
            b = tr[i : i + batch]
            h, out = forward(xn[b])
            # Erreur quadratique sur le résultat, rétropropagée à la main.
            g = 2 * (out - y[b]) * (1 - out**2) / len(b)
            gw2, gb2 = h.T @ g, g.sum()
            gh = np.outer(g, w2) * (h > 0)
            gw1, gb1 = gh.T @ xn[b], gh.sum(0)
            w1 -= lr * gw1
            b1 -= lr * gb1
            w2 -= lr * gw2
            b2 -= lr * gb2
        if epoch % 5 == 4:
            _, pv = forward(xn[va])
            decided = y[va] != 0
            acc = (np.sign(pv[decided]) == y[va][decided]).mean()
            print(f"époque {epoch + 1} : erreur {np.mean((pv - y[va]) ** 2):.3f}, bon vainqueur {acc:.1%}")

    # (x - mu) / sd @ w1.T + b1  ==  x @ (w1 / sd).T + (b1 - (w1 / sd) @ mu)
    assert all(np.isfinite(a).all() for a in (w1, b1, w2, [b2])), "l'entraînement a divergé"
    w1f = w1 / sd
    save("weights.bin", w1f, b1 - w1f @ mu, w2, b2)
    print(f"weights.bin : {len(y)} positions, {HIDDEN * N_IN + 2 * HIDDEN + 1} paramètres")


if __name__ == "__main__":
    main()

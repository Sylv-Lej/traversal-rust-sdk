# Arène des bots : `traversal-arena` + les 5 bots du site en WASM. Contexte de build = le
# dossier qui contient arena/, core/, sdk/ et bots/ :
#   docker build -f arena/Dockerfile -t traversal-arena .
#
# Pour les auteurs de bots : `duel`, `check`. Sur le site : `work` (avec ARENA_TOKEN pour
# seul secret).
FROM rust:1.88-slim AS build
RUN rustup target add wasm32-unknown-unknown
WORKDIR /src
COPY . .
# Bots maison : le bot de départ (bots/starter) compilé à chacun des 5 niveaux.
RUN mkdir /house && for l in 1 2 3 4 5; do \
      TRAVERSAL_LEVEL=$l cargo build --release --locked --target wasm32-unknown-unknown -p traversal-starter-bot && \
      cp target/wasm32-unknown-unknown/release/traversal_starter_bot.wasm /house/$l.wasm; \
    done
RUN cd arena && cargo build --release --locked

FROM debian:bookworm-slim
COPY --from=build /house /house
COPY --from=build /src/arena/target/release/traversal-arena /usr/local/bin/traversal-arena
ENV HOUSE_DIR=/house
# Aucun droit : le runner n'écrit rien, ne lit que /house et ce qu'on lui monte.
USER 65534:65534
ENTRYPOINT ["traversal-arena"]
CMD ["work"]

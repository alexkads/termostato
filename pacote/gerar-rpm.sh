#!/usr/bin/env bash
#
# Gera o RPM do termostato. Roda NA fedora (Fedora 44, com rpm-build), de
# dentro do clone:
#
#   pacote/gerar-rpm.sh          # deixa o .rpm em pacote/rpm/
#
# Compila o release esperando a trava da oficina (a mesma da bateria noturna e
# do e2e), com 2 jobs e nice, e empacota com o termostato.spec.
set -euo pipefail
RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ALVO="${CARGO_TARGET_DIR:-$HOME/oficina/target/termostato}"
VERSAO=$(sed -n 's/^version = "\(.*\)"/\1/p' "$RAIZ/Cargo.toml" | head -1)

echo "🔨 compilando a $VERSAO (espera a trava da oficina, se a bateria ou o e2e estiverem rodando)…"
# shellcheck disable=SC1091
source "$HOME/.cargo/env"
(
  exec 9>"$HOME/oficina/.trava-compilacao"
  flock 9
  cd "$RAIZ"
  CARGO_TARGET_DIR="$ALVO" CARGO_BUILD_JOBS=2 nice -n 10 cargo build --release -q
)

TOPO=$(mktemp -d)
trap 'rm -rf "$TOPO"' EXIT
mkdir -p "$TOPO/SOURCES"
cp "$ALVO/release/termostato" "$RAIZ/termostato.service" "$RAIZ/termostato.toml" \
   "$RAIZ/README.md" "$RAIZ/CHANGELOG.md" "$TOPO/SOURCES/"

echo "📦 empacotando…"
rpmbuild -bb --quiet \
  --define "_topdir $TOPO" \
  --define "versao $VERSAO" \
  "$RAIZ/pacote/termostato.spec"

mkdir -p "$RAIZ/pacote/rpm"
cp "$TOPO"/RPMS/x86_64/termostato-"$VERSAO"-*.rpm "$RAIZ/pacote/rpm/"
ls -1 "$RAIZ"/pacote/rpm/termostato-"$VERSAO"-*.rpm

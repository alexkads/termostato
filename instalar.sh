#!/usr/bin/env bash
#
# Instala (ou atualiza) o termostato na fedora. Roda NA fedora, de dentro do
# clone (~/oficina/repos/termostato), com sudo sem senha:
#
#   ./instalar.sh
#
# Compila, instala o binário e o serviço, cria /etc/termostato.toml se ele não
# existir (o que existe fica), tira os três serviços provisórios de 19/11/2026
# e liga o Prometheus e o Grafana em ~/oficina/monitoramento. Reinstalar é
# seguro.
set -euo pipefail
AQUI="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MONITORAMENTO="${MONITORAMENTO:-$HOME/oficina/monitoramento}"
ALVO="${CARGO_TARGET_DIR:-$HOME/oficina/target/termostato}"

echo "🔨 compilando (espera a trava da oficina, se a bateria ou o e2e estiverem rodando)…"
# shellcheck disable=SC1091
source "$HOME/.cargo/env"
(
  exec 9>"$HOME/oficina/.trava-compilacao"
  flock 9
  cd "$AQUI"
  CARGO_TARGET_DIR="$ALVO" CARGO_BUILD_JOBS=2 nice -n 10 cargo build --release -q
)

echo "📦 instalando…"
sudo install -m 755 "$ALVO/release/termostato" /usr/local/bin/termostato
sudo install -m 644 "$AQUI/termostato.service" /etc/systemd/system/termostato.service
# 0.4.0: as chaves do freio antigo viram as da curva (as antigas ainda são aceitas).
if [ -f /etc/termostato.toml ]; then
  sudo sed -i -e 's/^solta_abaixo_c/freio_comeca_c/' -e 's/^desliga_abaixo_c/freio_comeca_c/' \
    -e 's/^leituras_para_frear/janela_leituras/' /etc/termostato.toml
fi
if [ ! -f /etc/termostato.toml ]; then
  sudo install -m 644 "$AQUI/termostato.toml" /etc/termostato.toml
else
  echo "   /etc/termostato.toml já existe: fica como está"
fi

# Os provisórios de 19/11/2026: o termostato faz o trabalho dos três.
for antigo in cpu-folego-longo freio-de-calor ventoinhas-no-maximo; do
  if [ -f "/etc/systemd/system/$antigo.service" ]; then
    sudo systemctl disable --now "$antigo.service" >/dev/null 2>&1 || true
    sudo rm -f "/etc/systemd/system/$antigo.service"
    echo "   🧹 $antigo.service removido"
  fi
done
sudo rm -f /usr/local/bin/freio-de-calor /usr/local/bin/ventoinhas-no-maximo

# O único desvio do padrão do Fedora, e por isso fica aqui (o desinstalar.sh o
# desfaz): o thermald (--adaptive) mexe no mesmo PL1 em degraus de 5 W quando a
# CPU esquenta (visto em 19/11/2026: 95 → 90 → 80 → 70 W a cada 2 s). Dois
# controladores no mesmo registro oscilam, e o thermald sozinho deixou a CPU
# chegar a 99 °C: o termostato fica com o papel.
if systemctl is-enabled --quiet thermald.service 2>/dev/null; then
  sudo systemctl disable --now thermald.service >/dev/null 2>&1
  echo "   🧹 thermald desligado: o termostato é quem controla o PL1"
fi

# Padrão do Fedora no resto: o termostato é o único desvio. O perfil de energia
# volta ao "balanced" (estava em "performance", que fixa o governor no clock
# máximo), pela mesma interface do menu de energia do GNOME. O turbo continua:
# no balanced o governor schedutil sobe o clock sob demanda.
PERFIL=(--system --dest org.freedesktop.UPower.PowerProfiles
        --object-path /org/freedesktop/UPower/PowerProfiles)
if command -v gdbus >/dev/null &&
   ! gdbus call "${PERFIL[@]}" --method org.freedesktop.DBus.Properties.Get \
     org.freedesktop.UPower.PowerProfiles ActiveProfile 2>/dev/null | grep -q "'balanced'"; then
  sudo gdbus call "${PERFIL[@]}" --method org.freedesktop.DBus.Properties.Set \
    org.freedesktop.UPower.PowerProfiles ActiveProfile "<'balanced'>" >/dev/null
  echo "   ⚖️ perfil de energia de volta ao padrão do Fedora: balanced"
fi

# Os valores de fábrica, para o --desinstalar. O termostato os grava sozinho
# na primeira subida; numa máquina em que ele (ou os provisórios) já rodou, o
# PL1 lido seria o nosso, então semeia o do firmware da Apple, 100 W. Os pisos
# das ventoinhas de fábrica não são conhecidos e ficam de fora.
if ! sudo test -f /var/lib/termostato/fabrica.toml; then
  sudo install -d -m 755 /var/lib/termostato
  echo "pl1_uw = 100000000" | sudo tee /var/lib/termostato/fabrica.toml >/dev/null
  echo "   💾 fábrica semeada: PL1 100 W (firmware da Apple)"
fi

sudo systemctl daemon-reload
sudo systemctl enable termostato.service >/dev/null 2>&1
sudo systemctl restart termostato.service
sleep 3
systemctl is-active --quiet termostato || { journalctl -u termostato -n 20 --no-pager; exit 1; }

if [ -d "$MONITORAMENTO" ]; then
  echo "📈 ligando ao Prometheus e ao Grafana…"
  if ! grep -q "job_name: termostato" "$MONITORAMENTO/prometheus.yml"; then
    cat >> "$MONITORAMENTO/prometheus.yml" <<'EOF'
  - job_name: termostato
    scrape_interval: 5s
    static_configs:
      - targets: ["127.0.0.1:9101"]
        labels: { maquina: macpro-i7-alex }
EOF
  fi
  install -m 644 "$AQUI/grafana/termostato.json" "$MONITORAMENTO/grafana/dashboards/termostato.json"
  install -m 644 "$AQUI/grafana/alerta-termostato.yml" "$MONITORAMENTO/grafana/provisioning/alerting/termostato.yml"
  (cd "$MONITORAMENTO" && docker compose restart prometheus grafana >/dev/null)
fi

curl -fsS 127.0.0.1:9101/metrics | grep -E '^termostato_(temperatura|limite_watts\{limite="pl1"\}|freio_acionado)'
echo "✅ termostato instalado: journalctl -u termostato -f para acompanhar"

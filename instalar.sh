#!/usr/bin/env bash
#
# Instala (ou atualiza) o termostato na fedora, pelo pacote RPM. Roda NA fedora,
# de dentro do clone (~/oficina/repos/termostato), com sudo sem senha:
#
#   ./instalar.sh
#
# Gera o RPM (pacote/gerar-rpm.sh) e o instala com o dnf. O que o pacote faz na
# primeira instalação: liga o serviço, desliga o thermald (o único desvio do
# padrão do Fedora) e volta o perfil de energia ao balanced. Na atualização, só
# reinicia o serviço, o que é seguro (parar deixa o limite normal). O
# /etc/termostato.toml que existe fica (%config(noreplace)).
#
# Depois liga o termostato ao Prometheus e ao Grafana de ~/oficina/monitoramento,
# que são desta máquina e por isso ficam fora do pacote. Reinstalar é seguro.
set -euo pipefail
AQUI="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MONITORAMENTO="${MONITORAMENTO:-$HOME/oficina/monitoramento}"
VERSAO=$(sed -n 's/^version = "\(.*\)"/\1/p' "$AQUI/Cargo.toml" | head -1)

"$AQUI/pacote/gerar-rpm.sh"
RPM=$(ls -1t "$AQUI"/pacote/rpm/termostato-"$VERSAO"-*.rpm | head -1)

echo "🧹 o que veio antes do pacote…"
# A instalação à mão das versões 0.1 a 0.6 (binário em /usr/local/bin e unit em
# /etc/systemd/system, que esconderia a do pacote em /usr/lib/systemd/system).
if [ -f /etc/systemd/system/termostato.service ]; then
  # Parar deixa o limite normal (38 W) e o TCC offset no chip: a troca não
  # abre buraco. O pacote liga o serviço logo em seguida.
  sudo systemctl disable --now termostato.service >/dev/null 2>&1 || true
  sudo rm -f /etc/systemd/system/termostato.service /usr/local/bin/termostato
  sudo systemctl daemon-reload
  echo "   termostato instalado à mão: trocado pelo pacote"
fi
# Os provisórios em shell de 19/11/2026.
for antigo in cpu-folego-longo freio-de-calor ventoinhas-no-maximo; do
  if [ -f "/etc/systemd/system/$antigo.service" ]; then
    sudo systemctl disable --now "$antigo.service" >/dev/null 2>&1 || true
    sudo rm -f "/etc/systemd/system/$antigo.service"
    echo "   $antigo.service removido"
  fi
done
sudo rm -f /usr/local/bin/freio-de-calor /usr/local/bin/ventoinhas-no-maximo
# As chaves do freio antigo viram as da curva (as antigas ainda são aceitas).
if [ -f /etc/termostato.toml ]; then
  sudo sed -i -e 's/^solta_abaixo_c/freio_comeca_c/' -e 's/^desliga_abaixo_c/freio_comeca_c/' \
    -e 's/^leituras_para_frear/janela_leituras/' /etc/termostato.toml
fi
# Os valores de fábrica, para a remoção. O termostato os grava sozinho na
# primeira subida; nesta máquina o PL1 já tinha sido mexido antes, então semeia
# o do firmware da Apple (100 W) e o TCC offset de fábrica (0).
if ! sudo test -f /var/lib/termostato/fabrica.toml; then
  sudo install -d -m 755 /var/lib/termostato
  printf "pl1_uw = 100000000\ntcc_offset_c = 0\n" | sudo tee /var/lib/termostato/fabrica.toml >/dev/null
  echo "   💾 fábrica semeada: PL1 100 W (firmware da Apple)"
fi

echo "📦 instalando $(basename "$RPM")…"
if rpm -q termostato >/dev/null 2>&1 && [ "$(rpm -q --qf '%{VERSION}' termostato)" = "$VERSAO" ]; then
  sudo dnf reinstall -y -q "$RPM"
else
  sudo dnf install -y -q "$RPM"
fi
# A primeira instalação do pacote liga o serviço; aqui garante o estado final
# (ele sobe no sysinit.target, antes do Docker, do k3s e da VM).
sudo systemctl enable --now termostato.service >/dev/null 2>&1
sudo systemctl restart termostato.service
sleep 3
systemctl is-active --quiet termostato || { journalctl -u termostato -n 20 --no-pager; exit 1; }

if [ -d "$MONITORAMENTO" ]; then
  echo "📈 ligando ao Prometheus e ao Grafana…"
  # O Grafana desta máquina: o padrão do código é sem Grafana (o projeto é
  # público), e o endereço da tailnet fica só na configuração daqui.
  if ! sudo grep -q '^\[grafana\]' /etc/termostato.toml; then
    printf '\n[grafana]\nurl = "http://100.110.180.47:3000"\ntoken_arquivo = "/etc/termostato/grafana.token"\n' | sudo tee -a /etc/termostato.toml >/dev/null
    sudo systemctl restart termostato.service
  fi
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
  (cd "$MONITORAMENTO" && docker compose restart prometheus grafana >/dev/null 2>&1)
fi

rpm -q termostato
curl -fsS 127.0.0.1:9101/metrics | grep -E '^termostato_(info|temperatura_celsius|limite_watts\{limite="pl1"\}|tcc_corte_celsius)'
echo "✅ termostato instalado: journalctl -u termostato -f para acompanhar"

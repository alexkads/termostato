#!/usr/bin/env bash
#
# Tira o termostato da fedora e devolve o hardware à fábrica. Roda NA fedora,
# com sudo sem senha:
#
#   ./desinstalar.sh
#
# A remoção do pacote (dnf remove) roda o `termostato --desinstalar`, que volta
# o PL1 aos 100 W do firmware, e religa o thermald. É o único caminho que faz
# isso: parar o serviço, atualizar ou um crash deixam o limite normal, de
# propósito. Depois, tira o termostato do Prometheus e do Grafana desta máquina.
set -euo pipefail
MONITORAMENTO="${MONITORAMENTO:-$HOME/oficina/monitoramento}"

if rpm -q termostato >/dev/null 2>&1; then
  sudo dnf remove -y -q termostato
else
  # Uma instalação à mão anterior ao pacote.
  sudo systemctl disable --now termostato.service >/dev/null 2>&1 || true
  if [ -x /usr/local/bin/termostato ]; then
    sudo /usr/local/bin/termostato --desinstalar /etc/termostato.toml
  fi
  sudo rm -f /etc/systemd/system/termostato.service /usr/local/bin/termostato
  sudo systemctl daemon-reload
  sudo systemctl enable --now thermald.service >/dev/null 2>&1 || true
fi
# O que sobra: a configuração ajustada (o rpm a guarda como .rpmsave) e o estado.
sudo rm -rf /etc/termostato.toml /etc/termostato.toml.rpmsave /var/lib/termostato

if [ -d "$MONITORAMENTO" ]; then
  rm -f "$MONITORAMENTO/grafana/dashboards/termostato.json" \
        "$MONITORAMENTO/grafana/provisioning/alerting/termostato.yml"
  python3 - "$MONITORAMENTO/prometheus.yml" <<'PY'
import sys
p = sys.argv[1]
linhas = open(p).read().split("\n")
saida, pulando = [], False
for l in linhas:
    if l.strip() == "- job_name: termostato":
        pulando = True
        continue
    if pulando and l.startswith("  - job_name:"):
        pulando = False
    if not pulando:
        saida.append(l)
open(p, "w").write("\n".join(saida))
PY
  (cd "$MONITORAMENTO" && docker compose restart prometheus grafana >/dev/null 2>&1)
fi
echo "✅ termostato desinstalado; PL1 agora: $(( $(cat /sys/class/powercap/intel-rapl:0/constraint_0_power_limit_uw) / 1000000 )) W"

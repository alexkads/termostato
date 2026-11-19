# Histórico de versões

Todas de 19/11/2026. O porquê de cada mudança está em [docs/decisoes.md](docs/decisoes.md) e a narrativa em [docs/historia.md](docs/historia.md).

## 0.4.0: curva de freio com previsão do pico

- O freio liga-desliga (5 leituras acima de 92 °C) virou uma **curva**: o PL1 desce em linha reta de 38 W em 80 °C até 25 W em 92 °C.
- **Previsão:** reta de mínimos quadrados nas últimas 5 leituras, 10 s à frente. A curva usa a maior entre a temperatura de agora e a prevista, e freia antes do pico.
- **Freia na hora, solta a 0,5 W/s**, no lugar da histerese.
- Configuração: `freio_comeca_c`, `janela_leituras`, `previsao_s`, `subida_w_por_s`. Os nomes antigos continuam aceitos, e o instalador migra o arquivo.
- Métricas novas: `termostato_temperatura_prevista_celsius`, `termostato_tendencia_celsius_por_segundo`, `termostato_freio_intensidade`.
- Painel: "Temperatura, previsão e freio" e "Controle da CPU" em porcentagem do freio.

## 0.3.0: a CPU nunca fica solta

- Vocabulário: **limite normal** e **freio de calor acionado/desligado**; nada de "solto" ou "puxado".
- Métricas renomeadas: `termostato_freio_acionado`, `termostato_freio_acionamentos_total`, `termostato_freio_acionado_segundos_total`.
- O `instalar.sh` volta o perfil de energia ao **balanced**, o padrão do Fedora.

## 0.2.0: parar sem abrir buraco

- SIGTERM, SIGINT e crash deixam o PL1 no limite normal, e não nos 100 W de fábrica.
- Valores de fábrica em `/var/lib/termostato/fabrica.toml`, gravados só na primeira subida.
- `termostato --desinstalar` e o `desinstalar.sh`.
- O `instalar.sh` desliga o `thermald`, que disputava o PL1.

## 0.1.0: o termostato

- Limite de potência (PL1 38 W) regravado a cada leitura, com o turbo curto (PL2 125 W) livre.
- Freio de calor: PL1 25 W com 5 leituras acima de 92 °C, voltando abaixo de 80 °C.
- Piso das ventoinhas no máximo.
- `/metrics` e `/saude` em `127.0.0.1:9101`; painel e alertas do Grafana.
- `instalar.sh`, que substitui os três serviços provisórios em shell.

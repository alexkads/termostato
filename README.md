# termostato

Um serviço em Rust, dono de toda a parte térmica da **fedora** (`macpro-i7-alex`, um MacBookPro11,2 de 2014 com i7-4770HQ e Fedora). Ele limita a potência sustentada da CPU, puxa um freio quando ela esquenta demais, deixa as ventoinhas no máximo e publica tudo para o Prometheus e o Grafana.

A ideia é não precisar mexer no sistema operacional: tudo o que se ajusta está em `/etc/termostato.toml`.

## Por que existe

- **O firmware da Apple deixa o limite de potência de longo prazo (RAPL PL1) em 100 W**, mais que o dobro dos 47 W de projeto do i7-4770HQ. Na prática não há limite: numa compilação longa a CPU fica em turbo nos 4 núcleos até chegar a 99 °C (crítico = 100 °C). No macOS, quem segurava isso era o próprio sistema; no Linux, ninguém.
- **O turbo curto é o que faz dele um foguetinho.** O termostato não mexe no PL2 (125 W) nem no turbo. Uma arrancada (abrir um programa, compilar um arquivo) sai em até 3,4 GHz como sempre. Só a carga sustentada, depois de ~28 s com os 4 núcleos ocupados, passa a caber em 38 W, e a CPU acha sozinha o clock que cabe nisso.
- **O `thermald` brigava pelo mesmo registro.** Com `--adaptive`, ele mexe no PL1 em degraus de 5 W quando a CPU esquenta (visto em 19/11/2026: 95 → 90 → 80 → 70 W a cada 2 s, contra os 38 W do termostato). O `instalar.sh` o desliga. O termostato ainda confere e regrava o PL1 a cada leitura, e registra no journal com 🔧 quando precisou: se essas linhas voltarem, há outro programa mexendo no PL1.

## O que ele faz, a cada 2 segundos

1. Lê a temperatura do pacote (`coretemp`, `temp1`).
2. Passa pelo **freio de calor**: com 5 leituras seguidas em 92 °C ou mais, o PL1 cai para 25 W (🔥 no journal). Ele só solta abaixo de 80 °C (❄️), e a distância evita ligar e desligar a cada leitura.
3. Garante o PL1 alvo (38 W, ou 25 W com o freio puxado) e o piso das ventoinhas no máximo.
4. Mede a potência real (RAPL `energy_uj`) e a frequência média, e atualiza `/metrics`.

Ao parar (SIGTERM, SIGINT ou crash), o PL1 fica no valor seguro (38 W) e as ventoinhas ficam como estão: um reinício nunca deixa a máquina sem limite no meio de uma compilação. O systemd o reinicia sozinho (`Restart=always`).

Os valores de fábrica (o PL1 de 100 W do firmware) ficam guardados em `/var/lib/termostato/fabrica.toml` e só voltam com o `desinstalar.sh`, que roda `termostato --desinstalar`, remove o serviço, religa o `thermald` e tira o termostato do Prometheus e do Grafana.

O número de cada `hwmon` muda entre boots, então os chips são achados pelo nome (`coretemp`, `applesmc`), nunca por caminho fixo.

## Ajustar

Edite `/etc/termostato.toml` e rode `sudo systemctl restart termostato`. Os campos, com os padrões, estão em [`termostato.toml`](termostato.toml):

| Campo | Padrão | O que é |
|---|---|---|
| `pl1_normal_w` | 38 | Potência sustentada. 32 se ainda esquentar, 42 se sobrar folga |
| `pl1_freio_w` | 25 | O PL1 com o freio puxado |
| `freio_acima_c` | 92 | Temperatura que conta como quente |
| `leituras_para_frear` | 5 | Leituras quentes seguidas para puxar |
| `solta_abaixo_c` | 80 | Abaixo disso, o freio solta |
| `intervalo_s` | 2 | Segundos entre leituras |
| `ventoinhas` | `"maximo"` | `"maximo"` ou `"automatico"` (o `applesmc` decide) |
| `escuta` | `127.0.0.1:9101` | Onde o Prometheus lê; só no loopback, porque o serviço roda como root |

Uma configuração inválida (campo com erro de digitação, freio acima do normal, histerese invertida, freio acima de 99 °C) é recusada ao subir, com a razão no journal.

## Instalar ou atualizar

O `gh` da fedora não tem token, então o código vai do Mac para o clone `~/oficina/repos/termostato`, e o instalador roda lá:

```bash
rsync -a --delete --exclude target ~/Projects/termostato/ fedora:oficina/repos/termostato/
ssh fedora 'cd ~/oficina/repos/termostato && ./instalar.sh'
```

O `instalar.sh`:

- compila esperando a trava da oficina (`~/oficina/.trava-compilacao`), com 2 jobs e `nice`;
- instala `/usr/local/bin/termostato` e o `termostato.service`;
- cria `/etc/termostato.toml` só se ele não existir;
- remove os três serviços provisórios de 19/11/2026 (`cpu-folego-longo`, `freio-de-calor`, `ventoinhas-no-maximo`) e desliga o `thermald`;
- liga o termostato ao Prometheus e ao Grafana de `~/oficina/monitoramento`.

O Rust é o 1.100.0, fixado em [`rust-toolchain.toml`](rust-toolchain.toml) só para este projeto. O padrão da máquina não muda.

## Métricas e Grafana

`curl 127.0.0.1:9101/metrics` mostra as métricas; `/saude` responde 200 com leitura recente e 503 sem.

| Métrica | O que é |
|---|---|
| `termostato_temperatura_celsius` | Pacote da CPU |
| `termostato_potencia_watts` | Potência média medida |
| `termostato_limite_watts{limite="pl1"\|"pl2"\|"pl1_alvo"}` | Os limites no hardware e o que o termostato quer |
| `termostato_freio_puxado` | 1 enquanto o freio segura |
| `termostato_freadas_total`, `termostato_freado_segundos_total` | Quantas vezes e por quanto tempo |
| `termostato_frequencia_mhz` | Média dos núcleos |
| `termostato_ventoinha_rpm{ventoinha, medida="atual"\|"piso"\|"maximo"}` | As ventoinhas |
| `termostato_leituras_total`, `termostato_falhas_de_leitura_total` | A saúde do próprio termostato |

Em [`grafana/`](grafana/) ficam o painel **Termostato** (`termostato.json`) e dois alertas (`alerta-termostato.yml`), que vão pelo e-mail do monitoramento:

- **termostato parado** por 2 min;
- **freio puxado há mais de 10 min**, sinal de ventoinha parada ou poeira nas aletas.

O painel está em http://macpro-i7-alex:3000, só pela tailnet.

## Desenvolver

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

Os testes montam um sysfs de mentira numa pasta temporária (`hardware::sysfs_de_teste`), então rodam em qualquer máquina, sem root e sem MacBook.

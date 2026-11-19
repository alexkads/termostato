# Operação

Tudo o que se faz no dia a dia com o termostato, sem mexer no sistema operacional.

## Instalar ou atualizar

O `gh` da fedora não tem token, então o código vai do Mac para o clone da fedora, e o instalador roda lá:

```bash
rsync -a --delete --exclude target ~/Projects/termostato/ fedora:oficina/repos/termostato/
```

```bash
ssh fedora 'cd ~/oficina/repos/termostato && ./instalar.sh'
```

O [`instalar.sh`](../instalar.sh), na ordem:

1. Compila esperando a trava da oficina (`~/oficina/.trava-compilacao`, a mesma da bateria noturna e do e2e), com 2 jobs e `nice`.
2. Instala `/usr/local/bin/termostato` e o `termostato.service`.
3. Migra as chaves antigas do `/etc/termostato.toml` e só cria o arquivo se ele não existir. O que você ajustou fica.
4. Remove os serviços provisórios de 19/11/2026, se ainda existirem.
5. Volta o perfil de energia ao **balanced**, o padrão do Fedora.
6. Desliga o `thermald`, o único desvio do padrão (ver [decisões](decisoes.md)).
7. Semeia os valores de fábrica, se ainda não existirem.
8. Reinicia o serviço (é seguro: parar deixa o limite normal, nunca os 100 W de fábrica).
9. Liga o Prometheus e o Grafana de `~/oficina/monitoramento`: o job `termostato`, o painel e os alertas.
10. Mostra as primeiras métricas.

Reinstalar é seguro, e é assim que se atualiza.

## Desinstalar

```bash
ssh fedora 'cd ~/oficina/repos/termostato && ./desinstalar.sh'
```

Devolve o PL1 de fábrica (`termostato --desinstalar`), remove o serviço, o binário, a configuração e o estado, religa o `thermald` e tira o termostato do Prometheus e do Grafana. É o único caminho que volta o PL1 aos 100 W do firmware.

## Ajustar

Edite o `/etc/termostato.toml` e reinicie:

```bash
ssh fedora 'sudoedit /etc/termostato.toml && sudo systemctl restart termostato'
```

| Campo | Padrão | O que é |
|---|---|---|
| `pl1_normal_w` | 38 | O limite normal: potência sustentada no dia a dia |
| `pl1_freio_w` | 25 | O fim da curva: o PL1 com o freio no máximo |
| `freio_comeca_c` | 80 | Temperatura (de agora ou prevista) em que a curva começa a frear |
| `freio_acima_c` | 92 | Temperatura em que o freio chega ao máximo |
| `janela_leituras` | 5 | Leituras na reta da tendência |
| `previsao_s` | 10 | Quantos segundos à frente a previsão olha |
| `subida_w_por_s` | 0,5 | Velocidade com que o PL1 volta ao normal; frear é na hora |
| `intervalo_s` | 2 | Segundos entre leituras |
| `ventoinhas` | `"maximo"` | `"maximo"` ou `"automatico"` (o `applesmc` decide) |
| `escuta` | `127.0.0.1:9101` | Onde o Prometheus lê |

A configuração é validada ao subir. Um campo com erro de digitação, freio acima do normal, curva invertida, freio acima de 99 °C ou valores fora da faixa fazem o serviço recusar, com a razão no journal. Os nomes da 0.3 (`leituras_para_frear`, `solta_abaixo_c`, `desliga_abaixo_c`) continuam aceitos.

### Receitas

| Quero | Mude |
|---|---|
| Menos calor em carga longa | `pl1_normal_w = 30` (a medição mostrou o mesmo desempenho sustentado: ver [a máquina](maquina.md)) |
| Frear mais cedo | `freio_comeca_c = 75` ou `previsao_s = 15` |
| Previsão menos nervosa | `janela_leituras = 8` (reta por 16 s) |
| Voltar ao normal mais rápido | `subida_w_por_s = 1` |
| Ventoinhas silenciosas | `ventoinhas = "automatico"` (o pico de temperatura sobe) |

## Acompanhar

- **Painel:** http://macpro-i7-alex:3000/d/termostato, pela tailnet.
- **Ao vivo, no terminal:** `ssh fedora 'journalctl -u termostato -f'`.
- **Números crus:** `ssh fedora 'curl -s 127.0.0.1:9101/metrics'`.
- **Saúde:** `ssh fedora 'curl -s 127.0.0.1:9101/saude'` (200 `ok`; 503 sem leitura recente).

## O journal

| Linha | Quer dizer |
|---|---|
| 🌡️ `termostato 0.4.0 · limite normal PL1 38 W …` | Subiu, com a configuração em vigor |
| 💾 `valores de fábrica guardados …` | Primeira subida: guardou o que o hardware tinha |
| 🔥 `… °C agora, … °C previstos …: freio de calor acionado, PL1 em … W` | A curva começou a frear |
| ❄️ `…: freio de calor desligado, de volta ao limite normal` | Voltou ao normal |
| 🔧 `PL1 estava em … W: reaplicado …` | Outro programa mudou o PL1 por fora |
| 🌀 `ventoinha N: piso … → … RPM` | O piso da ventoinha foi corrigido |
| 👋 `termostato parado com o PL1 em 38 W …` | Parou, deixando o limite normal |
| ↩️ `PL1 devolvido a 100 W` | Só no `--desinstalar` |
| ⚠️ … | Leitura ou escrita que falhou, com o motivo |

## As métricas

Em `127.0.0.1:9101/metrics`, lidas pelo Prometheus a cada 5 s:

| Métrica | O que é |
|---|---|
| `termostato_temperatura_celsius` | O pacote da CPU agora |
| `termostato_temperatura_prevista_celsius` | A prevista para daqui a `previsao_s` |
| `termostato_tendencia_celsius_por_segundo` | A inclinação da reta (negativa esfriando) |
| `termostato_freio_intensidade` | 0 = limite normal; 1 = PL1 no `pl1_freio_w` |
| `termostato_freio_acionado` | 1 enquanto o PL1 está abaixo do normal |
| `termostato_freio_acionamentos_total` | Quantas vezes a curva começou a frear |
| `termostato_freio_acionado_segundos_total` | Tempo total com o freio agindo |
| `termostato_potencia_watts` | Potência real do pacote |
| `termostato_limite_watts{limite="pl1"\|"pl2"\|"pl1_alvo"}` | Os limites no hardware e o que o termostato quer |
| `termostato_frequencia_mhz` | Média dos núcleos |
| `termostato_ventoinha_rpm{ventoinha, medida="atual"\|"piso"\|"maximo"}` | As ventoinhas |
| `termostato_leituras_total`, `termostato_falhas_de_leitura_total` | A saúde do próprio termostato |
| `termostato_info{versao}` | A versão em execução |

## Os alertas

Pelo e-mail do monitoramento (contato `email-pelo-carteiro`):

| Alerta | Quando | O que fazer |
|---|---|---|
| **Termostato parado** | Sem métricas por 2 min | `systemctl status termostato` e `journalctl -u termostato -n 50`. O systemd o reinicia sozinho; parado 2 min quer dizer que ele cai ao subir, geralmente por configuração inválida |
| **Freio de calor acionado há mais de 10 min** | O freio segura há 10 min | Com o limite normal no lugar, sinal de ventoinha parada ou poeira nas aletas. Confira as ventoinhas no painel. Se for carga normal, baixe o `pl1_normal_w` |

Os alertas que já existiam no monitoramento continuam: CPU acima de 90 °C por 5 min e acima de 95 °C por 1 min.

## Problemas conhecidos

| Sintoma | Causa provável | O que fazer |
|---|---|---|
| Muitas linhas 🔧 | Outro programa disputa o PL1 | Ver quem: `systemctl is-active thermald tuned`. O `thermald` deve estar desligado |
| `⚠️ … Permission denied` | O binário rodou fora do serviço, sem root | Rodar pelo `systemctl` |
| Freio agindo o tempo todo em carga longa | O resfriamento só aguenta ~25 W com os 8 threads (ver [a máquina](maquina.md)) | Esperado. Para menos picos, `pl1_normal_w = 30` |
| Temperatura subindo sem mudança de carga | Poeira nas aletas | Limpar as ventoinhas. A pasta térmica já foi trocada |
| Painel em branco no navegador estreito | O Grafana precisa de largura para a grade | Abrir numa janela larga |

## Desenvolver

```bash
cd ~/Projects/termostato
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

Os testes rodam no Mac e na fedora, sem root e sem MacBook (o sysfs de teste fica numa pasta temporária). O Rust é o 1.100.0, fixado só neste projeto em [`rust-toolchain.toml`](../rust-toolchain.toml). Compile e instale na fedora, que é onde o hardware está.

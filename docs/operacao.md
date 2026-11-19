# Operação

Tudo o que se faz no dia a dia com o termostato, sem mexer no sistema operacional.

## Instalar ou atualizar

### Pelo Release (o pacote pronto)

Cada versão tem um [Release](https://github.com/alexkads/termostato/releases) com o RPM anexado, e o `dnf` instala direto pela URL:

```bash
ssh fedora 'sudo dnf install -y https://github.com/alexkads/termostato/releases/download/v0.6.1/termostato-0.6.1-1.fc45.x86_64.rpm'
```

### Pelo código, na fedora

```bash
rsync -a --delete --exclude target ~/Projects/termostato/ fedora:oficina/repos/termostato/
```

```bash
ssh fedora 'cd ~/oficina/repos/termostato && ./instalar.sh'
```

O [`instalar.sh`](../instalar.sh), na ordem:

1. Gera o RPM com o [`pacote/gerar-rpm.sh`](../pacote/gerar-rpm.sh): compila esperando a trava da oficina (`~/oficina/.trava-compilacao`, a mesma da bateria noturna e do e2e), com 2 jobs e `nice`, e roda o `rpmbuild` com o [`pacote/termostato.spec`](../pacote/termostato.spec).
2. Tira o que veio antes do pacote: a instalação à mão (`/usr/local/bin`, unit em `/etc/systemd/system`) e os serviços provisórios de 19/11/2026. Parar o termostato deixa o limite normal e o TCC no chip, então a troca não abre buraco (medido: o PL1 não passou de 38 W).
3. Migra as chaves antigas do `/etc/termostato.toml` e semeia os valores de fábrica, se faltarem.
4. Instala o RPM com o `dnf` (ou reinstala, na mesma versão).
5. Liga o Prometheus e o Grafana de `~/oficina/monitoramento`: o job `termostato`, o painel e os alertas. Eles são desta máquina e por isso ficam fora do pacote.

### O que o pacote faz

| Momento | O que acontece |
|---|---|
| Primeira instalação | liga o serviço (no `sysinit.target`), desliga o `thermald` (o único desvio do padrão do Fedora) e volta o perfil de energia ao `balanced` |
| Atualização | reinicia o serviço; parar deixa o limite normal, então não há janela sem limite. O `/etc/termostato.toml` ajustado fica (`%config(noreplace)`; o novo vira `.rpmnew`) |
| Remoção (`dnf remove`) | para o serviço, roda `termostato --desinstalar` (PL1 100 W, TCC offset 0, pisos das ventoinhas) e religa o `thermald` |

Os arquivos: `/usr/bin/termostato`, `/usr/lib/systemd/system/termostato.service`, `/etc/termostato.toml`, `/var/lib/termostato/` e a documentação em `/usr/share/doc/termostato/`. O token do Grafana (`/etc/termostato/grafana.token`) não vem no pacote: é desta máquina.

### Publicar uma versão nova

```bash
ssh fedora 'cd ~/oficina/repos/termostato && pacote/gerar-rpm.sh'
```

```bash
scp 'fedora:oficina/repos/termostato/pacote/rpm/termostato-*.rpm' /tmp/ && gh release create vX.Y.Z /tmp/termostato-X.Y.Z-*.rpm -R alexkads/termostato --title "termostato X.Y.Z" --notes-file CHANGELOG.md
```

## Desinstalar

```bash
ssh fedora 'cd ~/oficina/repos/termostato && ./desinstalar.sh'
```

Remove o pacote (que devolve a fábrica e religa o `thermald`), apaga a configuração e o estado e tira o termostato do Prometheus e do Grafana. É o único caminho que volta o PL1 aos 100 W do firmware.

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

### Os superpoderes

| Campo | Padrão | O que é |
|---|---|---|
| `tcc_offset_c` | 8 | O chip se freia sozinho a TjMax − offset (92 °C). 0 deixa o de fábrica; máximo 15 |
| `psi_parado_ms` / `psi_janela_ms` | 150 / 1000 | O gatilho de pressão de memória no kernel. 0 desliga |
| `[processos] aprender` / `antecipar` | `true` / `true` | Aprender quem esquenta e frear antes quando ele aparece |
| `[processos] suspeito_c_s` | 1,0 | Nota de calor a partir da qual o processo é suspeito |
| `[processos] minimo_de_observacoes` | 3 | Observações de calor antes de virar suspeito |
| `[maos] modo` | `"observar"` | `"observar"` (só mostra o que faria), `"agir"` ou `"desligado"` |
| `[maos] cpu_a_partir_de` | 0,3 | Intensidade do freio que começa a segurar a CPU de alguém |
| `[maos] memoria_aperto_pct` / `memoria_folga_pct` | 10 / 2 | Pressão de memória que segura e que solta |
| `[maos] nunca` | sistema, SSH, Tailscale, Docker, monitoramento | Quem nunca é segurado |
| `[grafana] url` / `token_arquivo` | tailnet / `/etc/termostato/grafana.token` | As anotações |

**Passar as mãos para agir**, depois de ver o modo observar no painel ("O que o termostato fez", os tipos `_observado`): `[maos] modo = "agir"` no `/etc/termostato.toml` e reiniciar. Para voltar, `"observar"` e reiniciar: ao parar, ele devolve tudo o que estava segurando.

**Zerar o histórico negativo:** `sudo systemctl stop termostato && sudo rm /var/lib/termostato/historico.json && sudo systemctl start termostato`.

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
| 🧠 `rustc (8 cópias, 90 % da CPU) tem histórico de +4.2 °C/s: freio antecipado…` | O freio veio antes, pelo histórico do processo |
| 📒 `qemu-system-x86 entrou no histórico negativo…` | Um processo passou a ser suspeito |
| 👀 `(observar) seguraria …` | O que as mãos fariam no modo observar |
| 🤚 `…: CPU de 6.4 para 2.0 núcleos (cpu.max em 2.0 núcleos)` | As mãos seguraram alguém (modo agir) |
| ↩️ `…: devolvido: …` | Um limite voltou ao original |
| 🛡️ `TCC offset 0 → 8 °C: o chip se freia sozinho a 92 °C` | A proteção do chip foi gravada |
| ⚡ `gatilho de pressão armado…` / `o kernel avisou…` | O kernel acorda o termostato quando a memória aperta |
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
| `termostato_pressao_pct{recurso, tipo}` | Pressão (PSI avg10) de memória, CPU e disco |
| `termostato_memoria_disponivel_pct`, `termostato_swap_usado_mb` | A memória que sobra |
| `termostato_processo_fatia{processo}`, `_copias`, `_memoria_mb` | Os 5 nomes que mais gastam CPU |
| `termostato_processo_nota_c_s{processo}` | O histórico negativo |
| `termostato_freio_antecipado`, `termostato_segurados` | Suspeito ativo agora; cgroups segurados agora |
| `termostato_acoes_total{acao}` | Cada ação, por tipo (`_observado` = só no modo observar) |
| `termostato_tcc_corte_celsius`, `termostato_tcc_offset_celsius` | Onde o próprio chip se freia |
| `termostato_kernel_avisos_total` | Vezes que o kernel acordou o termostato |
| `termostato_estrangulamento_total{onde}` | Estrangulamentos térmicos do chip desde o boot |
| `termostato_dominio_watts{dominio}` | Potência dos núcleos, do gráfico e da memória |
| `termostato_sensor_celsius{sensor}` | Os sensores do `applesmc` |

## Os alertas

Pelo e-mail do monitoramento (contato `email-pelo-carteiro`):

| Alerta | Quando | O que fazer |
|---|---|---|
| **Termostato parado** | Sem métricas por 2 min | `systemctl status termostato` e `journalctl -u termostato -n 50`. O systemd o reinicia sozinho; parado 2 min quer dizer que ele cai ao subir, geralmente por configuração inválida |
| **Freio de calor acionado há mais de 10 min** | O freio segura há 10 min | Com o limite normal no lugar, sinal de ventoinha parada ou poeira nas aletas. Confira as ventoinhas no painel. Se for carga normal, baixe o `pl1_normal_w` |

| **O termostato segurou um processo ou contêiner** | As mãos agiram (só no modo agir) | Ver as marcas no painel e `journalctl -u termostato \| grep 🤚`. Para desfazer: modo observar e reiniciar |
| **Pressão de memória alta** | Mais de 20 % por 2 min | Ver no painel quem está gastando. Compilação com muitos jobs, a VM e os contêineres do e2e juntos não cabem em 16 GB |

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

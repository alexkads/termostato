# Operação

Tudo o que se faz no dia a dia com o termostato, sem mexer no sistema operacional.

## Instalar ou atualizar

### Pelo Release (o pacote pronto)

Cada versão tem um [Release](https://github.com/alexkads/termostato/releases) com o RPM anexado, e o `dnf` instala direto pela URL:

```bash
ssh fedora 'sudo dnf install -y https://github.com/alexkads/termostato/releases/download/v0.8.0/termostato-0.8.0-1.fc45.x86_64.rpm'
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

## Ajustar na hora (`termostato chave=valor`)

Qualquer parâmetro da tabela abaixo muda **com o serviço rodando**, sem reiniciar e sem editar o arquivo do pacote. Ele vale em até um intervalo (2 s):

```bash
ssh fedora 'sudo termostato CPU50C'                               # segura a CPU em 50 °C
ssh fedora 'sudo termostato pl1_normal_w=30 previsao_s=15 maos.modo=agir'
ssh fedora 'sudo termostato pl1_normal_w=padrao'                  # tira um ajuste
ssh fedora 'sudo termostato CPULIVRE'                             # sem alvo (alvo_c=0)
ssh fedora 'sudo termostato padrao'                               # tira todos
ssh fedora 'termostato ajustes'                                   # o que está em vigor
```

- **`CPU<n>C`** é o atalho de `alvo_c=<n>`, aceito em maiúsculas ou minúsculas e com vírgula ou ponto (`CPU62,5C`), de 40 a 95 °C. O termostato baixa o PL1 o quanto precisar, até o `pl1_minimo_w` (8 W), para a CPU ficar no alvo. Depois volta a subir devagar (0,5 W/s) quando há folga.
- O pedido é **conferido antes de gravar**, com as mesmas regras do toml. Chave que não existe, valor fora da faixa ou freio acima do normal são recusados, e nada é gravado.
- Os ajustes moram em `/var/lib/termostato/ajustes.toml`, por cima do `/etc/termostato.toml`. Sobrevivem a um reinício e somem com `termostato padrao` ou `--desinstalar`.
- Cada mudança aparece no journal (🎛️), vira anotação no Grafana (tag `ajuste`) e conta em `termostato_acoes_total{tipo="ajuste"}`. O alvo aparece em `termostato_alvo_celsius`.
- **Só mudam reiniciando:** `escuta`, `sysfs`, `estado`, `proc`, `docker_socket`, `dev_cpu`, `psi_*`, `tcc_offset_c` e `[grafana]`. A linha de comando os recusa.
- Gravar exige root. Sem `sudo`, a resposta é "sem permissão, rode com sudo".

## Ajustar de vez (o toml)

Para valer sempre, edite o `/etc/termostato.toml` e reinicie:

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
| `alvo_c` | 0 | Alvo de temperatura (40 a 95 °C); 0 = sem alvo, só a curva. Atalho: `termostato CPU50C` |
| `pl1_minimo_w` | 8 | O PL1 mais baixo a que o alvo pode levar |
| `alvo_ganho_w_por_c_s` | 0,5 | Quanto o PL1 desce por segundo para cada °C acima do alvo |
| `intervalo_s` | 2 | Segundos entre leituras |
| `ventoinhas` | `"curva"` | `"curva"` (o piso segue a temperatura), `"maximo"` ou `"automatico"` (o `applesmc` decide) |
| `ventoinhas_calma_ate_c` | 60 | Curva: até aqui, o piso fica no mínimo |
| `ventoinhas_maximo_em_c` | 80 | Curva: com a média daqui em diante, o piso vai ao máximo |
| `ventoinhas_minimo_rpm` | 2000 | Curva: o piso mínimo |
| `ventoinhas_descida_rpm_por_s` | 50 | Curva: quão rápido o piso desce ao esfriar |
| `ventoinhas_suavizacao_s` | 20 | Curva: a média que a curva olha; um pico mais curto que isso mexe pouco na ventoinha (0 = a temperatura de agora) |
| `escuta` | `127.0.0.1:9101` | Onde o Prometheus lê |

### Os superpoderes

| Campo | Padrão | O que é |
|---|---|---|
| `tcc_offset_c` | 8 | O chip se freia sozinho a TjMax − offset (92 °C). 0 deixa o de fábrica; máximo 15. **Neste Haswell, ele é gravado mas não segura** (ver [a máquina](maquina.md)) |
| `[caixa_preta] horas` | 24 | Quantas horas a caixa-preta guarda (1 a 168). Só muda reiniciando |
| `[caixa_preta] mb_max` | 50 | O teto de espaço da caixa-preta, em MB (1 a 2000). Só muda reiniciando |
| `psi_parado_ms` / `psi_janela_ms` | 150 / 1000 | O gatilho de pressão de memória no kernel. 0 desliga |
| `[processos] aprender` / `antecipar` | `true` / `true` | Aprender quem esquenta e frear antes quando ele aparece |
| `[processos] suspeito_c_s` | 1,0 | Nota de calor a partir da qual o processo é suspeito |
| `[processos] minimo_de_observacoes` | 3 | Observações de calor antes de virar suspeito |
| `[maos] modo` | `"observar"` | `"observar"` (só mostra o que faria), `"agir"` ou `"desligado"` |
| `[maos] cpu_a_partir_de` | 0,3 | Intensidade do freio que começa a segurar a CPU de alguém |
| `[maos] memoria_aperto_pct` / `memoria_folga_pct` | 10 / 2 | Pressão de memória que segura e que solta |
| `[maos] nunca` | sistema, SSH, Tailscale, Docker, monitoramento | Quem nunca é segurado |
| `[grafana] url` / `token_arquivo` | tailnet / `/etc/termostato/grafana.token` | As anotações |

**Passar as mãos para agir**, depois de ver o modo observar no painel ("O que o termostato fez", os tipos `_observado`): `sudo termostato maos.modo=agir`. Para voltar, `sudo termostato maos.modo=padrao`: ao sair do agir, ele devolve tudo o que estava segurando.

**Zerar o histórico negativo:** `sudo systemctl stop termostato && sudo rm /var/lib/termostato/historico.json && sudo systemctl start termostato`.

A configuração é validada ao subir. Um campo com erro de digitação, freio acima do normal, curva invertida, freio acima de 99 °C ou valores fora da faixa fazem o serviço recusar, com a razão no journal. Os nomes da 0.3 (`leituras_para_frear`, `solta_abaixo_c`, `desliga_abaixo_c`) continuam aceitos.

### Receitas

| Quero | Mude |
|---|---|
| A máquina fria agora, custe o desempenho | `sudo termostato CPU55C` |
| Menos calor em carga longa | `pl1_normal_w = 30` (a medição mostrou o mesmo desempenho sustentado: ver [a máquina](maquina.md)) |
| Frear mais cedo | `freio_comeca_c = 75` ou `previsao_s = 15` |
| Previsão menos nervosa | `janela_leituras = 8` (reta por 16 s) |
| Voltar ao normal mais rápido | `subida_w_por_s = 1` |
| Ventoinhas mais silenciosas | `ventoinhas_calma_ate_c=65` (começam a subir mais tarde) ou `ventoinhas_suavizacao_s=40` (ignoram picos mais longos) ou `ventoinhas = "automatico"` (o pico de temperatura sobe) |
| Ventoinhas sempre no máximo | `ventoinhas = "maximo"` |

## Estressar a máquina (`termostato estresse`)

Leva a CPU, a memória e o disco ao limite e mostra todos os sensores lado a lado, a cada 2 s. No fim, diz onde a máquina parou. O comando só gera carga e lê: o freio continua com o serviço e com os ajustes em vigor, e assim o teste mede a máquina como ela está.

```bash
ssh -t fedora 'sudo termostato estresse'
```

```bash
ssh -t fedora 'sudo termostato estresse cpu segundos=120'
```

| Opção | Padrão | O que é |
|---|---|---|
| `cpu`, `memoria`, `disco` | os três | O que estressar. Sem nenhum, vão os três |
| `segundos=` | 60 | De 5 a 1800 |
| `threads=` | todas (8) | Threads de CPU |
| `memoria_pct=` | 50 | Quanto da memória **disponível** ocupar, de 1 a 90 |
| `disco_mb=` | 1024 | O tamanho do arquivo gravado e relido |
| `pasta=` | `/var/tmp` | Onde fica o arquivo |

**O que cada carga faz:**

- **CPU:** uma thread por núcleo lógico, num laço de inteiros.
- **Memória:** escreve um padrão diferente a cada volta e relê conferindo, como um memtest simples. Um erro de conferência aparece no resumo.
- **Disco:** grava o arquivo com `fsync`, tira do cache, relê e confere. Mede os MB/s de gravação e de leitura.

**As colunas:**

| Coluna | O que é |
|---|---|
| `chip` | O pacote da CPU (`coretemp`) |
| `núcleos` | Cada núcleo físico (`coretemp`, "Core N") |
| `perto` | `TC0P`, o sensor na placa ao lado da CPU |
| `dissip` | A média dos `TH0*`, o dissipador |
| `W` | O pacote, pelo RAPL. Sem `sudo`, sai "—" |
| `mem%` | A memória disponível (`MemAvailable`) |
| `estrang` | Os estrangulamentos do chip desde a largada |

**As proteções:**

- A memória nunca deixa menos de 10 % do total livre. Se o disponível cair abaixo de 5 % durante o teste, ela é devolvida na hora.
- O disco recusa começar sem `disco_mb` + 2 GiB livres. O arquivo `termostato-estresse-<pid>` é apagado no fim, também no Ctrl+C.
- Com o chip em 99 °C ou mais por 30 s seguidos, a carga para: o limite foi achado.

**O diagnóstico do fim:**

| Linha | Quer dizer |
|---|---|
| ✅ Aguentou | Sem estrangulamento: há folga |
| 🧱 O calor não sai do chip | Estrangulou com as ventoinhas no máximo e o dissipador subiu menos de 5 °C. O gargalo é entre o chip e o dissipador: pasta térmica ou contato |
| 🌀 As ventoinhas não acompanharam | Estrangulou com as ventoinhas abaixo de 90 % do máximo |
| 🔥 O resfriamento chegou ao limite | O dissipador esquentou junto: o chassi não dá conta de mais calor |

O resumo também traz a média e o pico de cada núcleo, os estrangulamentos por núcleo e o tempo total estrangulado, com mais uma linha:

| Linha | Quer dizer |
|---|---|
| ⚖️ Os núcleos estão parelhos | Até 8 °C entre o mais quente e o mais frio, com a mesma carga |
| ⚖️ Os núcleos estão desiguais | Mais de 8 °C: a pasta não cobre o chip por igual |

## A caixa-preta

A cada leitura (2 s), o termostato grava uma linha JSON com o que explica um travamento:
- cada núcleo, o PL1, os watts, os MHz e as ventoinhas;
- os motivos do freio que o chip registra;
- a bateria;
- o PSI, a memória disponível, o swap por segundo e as mortes do OOM;
- os 3 que mais gastam CPU e os 3 que mais seguram memória (RAM + swap, por nome).

As linhas vão para `/var/lib/termostato/caixa-preta/<hora unix>.jsonl`, com `fsync` a cada 10 s. Ficam as últimas `[caixa_preta] horas` (24), até `mb_max` (50 MB).

```bash
ssh fedora 'sudo tail -1 /var/lib/termostato/caixa-preta/$(sudo ls /var/lib/termostato/caixa-preta | tail -1) | jq .'
```

**Depois de uma queda.** Ao parar limpo, o serviço grava o marcador `parada-limpa`. Se ele não está lá na subida seguinte, o journal e o Grafana recebem:

```
💥 a fedora parou às 2026-11-19 18:32:16 sem desligar (travou ou perdeu energia). A última leitura: chip 97 °C, memória disponível 0 %, PSI de memória 88 %, swap entrando 41 MB/s e saindo 63 MB/s. Quem segurava a memória: rustc ×8 (12.0 GB + 11.0 GB de swap), …
```

O último minuto vai para `/var/lib/termostato/quedas/<hora>.json`. Pelo `boot_id` de cada linha, o 💥 separa os dois casos:
- **a máquina travou ou perdeu energia:** a última linha é de outro boot;
- **o serviço foi morto e a máquina seguiu ligada:** a última linha é do mesmo boot.

```bash
ssh fedora 'sudo ls /var/lib/termostato/quedas && journalctl -u termostato | grep 💥'
```

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
| 🌀 `ventoinha N: piso … → … RPM` | O piso da ventoinha foi corrigido (no modo curva, só ao chegar ao mínimo ou ao máximo) |
| 👋 `termostato parado com o PL1 em 38 W …` | Parou, deixando o limite normal |
| ↩️ `PL1 devolvido a 100 W` | Só no `--desinstalar` |
| 🧠 `rustc (8 cópias, 90 % da CPU) tem histórico de +4.2 °C/s: freio antecipado…` | O freio veio antes, pelo histórico do processo |
| 📒 `qemu-system-x86 entrou no histórico negativo…` | Um processo passou a ser suspeito |
| 👀 `(observar) seguraria …` | O que as mãos fariam no modo observar |
| 🤚 `…: CPU de 6.4 para 2.0 núcleos (cpu.max em 2.0 núcleos)` | As mãos seguraram alguém (modo agir) |
| ↩️ `…: devolvido: …` | Um limite voltou ao original |
| 🛡️ `TCC offset 0 → 8 °C: o chip se freia sozinho a 92 °C` | O offset foi gravado. Neste Haswell ele não segura: ver a linha abaixo |
| 🛡️ `o chip chegou a … °C, acima do corte de 92 °C do TCC offset…` | O chip passou do corte: quem o protege é o TjMax (100 °C). O painel e a métrica passam a mostrar 100 |
| 📼 `caixa-preta em …` | A caixa-preta está gravando |
| 💥 `a fedora parou às … sem desligar…` | Houve queda desde a última parada, com a última leitura |
| 🔋 `a bateria descarrega com o carregador ligado há … min…` | O carregador não dá conta da carga (5 min seguidos) |
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
| `termostato_tcc_corte_celsius`, `termostato_tcc_offset_celsius` | Onde o próprio chip se freia: TjMax − offset, ou o TjMax depois que o chip passou do corte |
| `termostato_tcc_respeitado` | 0 se o chip já passou do corte do offset |
| `termostato_nucleo_celsius{nucleo}` | Cada núcleo físico |
| `termostato_nucleo_estrangulamento_total{nucleo}` | Estrangulamentos de cada núcleo desde o boot |
| `termostato_estrangulado_segundos_total` | Tempo com o pacote estrangulado desde o boot |
| `termostato_freio_motivo{motivo, quando="agora"\|"desde_o_boot"}` | Por que o chip está freado (`MSR 0x690`): `termico`, `pl1`, `pl2`, `corrente`, `prochot` (o SMC ou o carregador, por fora), `vr`, `turbo_maximo` |
| `termostato_bateria_amperes`, `termostato_bateria_carga_pct`, `termostato_carregador_ligado` | A bateria (negativa descarregando) e o carregador |
| `termostato_swap_mb_por_segundo{sentido="entra"\|"sai"}` | Swap trocado por segundo |
| `termostato_oom_mortes_total` | Mortes do OOM killer desde o boot |
| `termostato_memoria_mb{processo, tipo="ram"\|"swap"}` | Os 5 nomes que mais seguram memória |
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
| **A bateria descarrega com o carregador ligado há mais de 10 min** | A corrente da bateria abaixo de −0,1 A com o carregador ligado | Conferir se o carregador é o MagSafe 2 de 85 W. Com a bateria vazia, o SMC pode frear a CPU por fora: ver o painel "Por que o chip está freado" |

Os alertas que já existiam no monitoramento continuam: CPU acima de 90 °C por 5 min e acima de 95 °C por 1 min.

## Problemas conhecidos

| Sintoma | Causa provável | O que fazer |
|---|---|---|
| Muitas linhas 🔧 | Outro programa disputa o PL1 | Ver quem: `systemctl is-active thermald tuned`. O `thermald` deve estar desligado |
| `⚠️ … Permission denied` | O binário rodou fora do serviço, sem root | Rodar pelo `systemctl` |
| Freio agindo o tempo todo em carga longa | O resfriamento só aguenta ~25 W com os 8 threads (ver [a máquina](maquina.md)) | Esperado. Para menos picos, `pl1_normal_w = 30` |
| Temperatura subindo sem mudança de carga | Poeira nas aletas | Limpar as ventoinhas. A pasta térmica já foi trocada |
| 🧱 no `termostato estresse`: chip a 100 °C, ventoinhas no máximo e dissipador frio | O calor não passa do chip para o dissipador. Com a pasta já trocada, o suspeito é o contato: parafusos do dissipador, pasta demais ou mal espalhada | Abrir e conferir o assentamento do dissipador |
| 💥 no journal ao subir | A parada anterior não foi limpa | Ler a frase e o `/var/lib/termostato/quedas/`: memória, swap, OOM ou calor |
| Painel em branco no navegador estreito | O Grafana precisa de largura para a grade | Abrir numa janela larga |

## Desenvolver

```bash
cd ~/Projects/termostato
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

Os testes rodam no Mac e na fedora, sem root e sem MacBook (o sysfs de teste fica numa pasta temporária). O Rust é o 1.100.0, fixado só neste projeto em [`rust-toolchain.toml`](../rust-toolchain.toml). Compile e instale na fedora, que é onde o hardware está.

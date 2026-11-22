# Histórico de versões

Todas de 19/11/2026. O porquê de cada mudança está em [docs/decisoes.md](docs/decisoes.md) e a narrativa em [docs/historia.md](docs/historia.md).

## Ainda não lançado: ventoinhas pela curva

- **Painel com a energia (#26):**
  - três cartões novos: "Cada núcleo", "Por que o chip freia" (os motivos do `MSR 0x690` e o PROCHOT) e "Bateria e carregador" (corrente, tensão, carga e os Wh tirados da bateria com o carregador ligado);
  - o `GET /api/estado` passa a mandar `nucleos`, `motivos`, `prochot` e `bateria`;
  - o cartão das ventoinhas deixa de dizer "No máximo desde o boot".
- **Registro de cada PROCHOT e de cada descarga da bateria no carregador (#24):**
  - o termostato limpa o log do `MSR 0x690` a cada leitura, e assim nenhum freio de fora curto se perde;
  - ⛔ e ✅ no journal, anotação no Grafana (no máximo uma por minuto), `termostato_prochot_eventos_total` e `_segundos_total`;
  - a bateria ganha a tensão e os acumulados com o carregador ligado (segundos, Wh, episódios);
  - 🔌 quando o carregador sai ou volta, e 🔋 quando a carga cai abaixo de 80 % no carregador;
  - dois motivos novos: `utilizacao_autonoma` e `atenuacao_turbo`.

  Grafana: o painel "PROCHOT e bateria no carregador" e o alerta "A CPU foi freada por fora (PROCHOT)". O 💥 da caixa-preta diz quantas leituras com PROCHOT houve no último minuto.
- **Caixa-preta (#21):** a cada leitura, uma linha JSON em `/var/lib/termostato/caixa-preta/` com:
  - cada núcleo e os motivos do freio do chip (`MSR 0x690`);
  - a bateria, o PSI, a memória, o swap por segundo e as mortes do OOM;
  - quem mais gasta CPU e quem mais segura memória.

  Fica em anel (`[caixa_preta] horas = 24`, `mb_max = 50`), com `fsync` a cada 10 s. Depois de uma queda, o boot seguinte escreve 💥 no journal e no Grafana com a hora e a última leitura, e guarda o último minuto em `/var/lib/termostato/quedas/`.
- **Métricas novas:** `termostato_nucleo_celsius`, `termostato_nucleo_estrangulamento_total`, `termostato_estrangulado_segundos_total`, `termostato_freio_motivo`, `termostato_bateria_amperes`, `termostato_bateria_carga_pct`, `termostato_carregador_ligado`, `termostato_swap_mb_por_segundo`, `termostato_oom_mortes_total`, `termostato_memoria_mb` e `termostato_tcc_respeitado`.
- **O corte do TCC diz a verdade:** quando o chip passa do corte do offset, o termostato avisa (🛡️), e o `termostato_tcc_corte_celsius` e o painel passam a mostrar o TjMax (100 °C).
- **🔋 Aviso da bateria:** descarregando com o carregador ligado por 5 min seguidos.
- **Grafana:** faixa nova "O que realmente acontece" (cada núcleo, estrangulamentos por núcleo, por que o chip está freado, bateria e carregador, swap e OOM, quem segura a memória) e o alerta "A bateria descarrega com o carregador ligado há mais de 10 min".
- **`termostato estresse`** leva a CPU, a memória e o disco ao limite por 60 s (ou `segundos=`). A cada 2 s mostra o chip, o sensor perto da CPU, o dissipador, os watts, os MHz, as ventoinhas, a memória disponível, os MB/s do disco e os estrangulamentos. No fim, o resumo diz onde a máquina parou: 🧱 o calor não sai do chip, 🌀 as ventoinhas não acompanharam, 🔥 o resfriamento chegou ao limite ou ✅ aguentou. A memória é conferida a cada volta e o disco é relido fora do cache. Proteções: 10 % da memória sempre livre, devolução abaixo de 5 %, espaço em disco conferido antes, arquivo apagado no Ctrl+C e parada com o chip 30 s em 99 °C (#19).
- **Cada núcleo no estresse:** a temperatura de cada núcleo físico a cada linha. No resumo, a média e o pico por núcleo, os estrangulamentos por núcleo, o tempo total estrangulado e se os núcleos estão parelhos ou desiguais (⚖️, mais de 8 °C de diferença).
- **Novo modo `ventoinhas = "curva"`, agora o padrão.** O piso das ventoinhas acompanha a temperatura média dos últimos ~20 s: fica no mínimo (2000 RPM) até 60 °C, sobe em linha reta e chega ao máximo em 80 °C, onde o freio começa. Uma arrancada curta do turbo quase não mexe nele; sem leitura de temperatura, vai ao máximo. Desce devagar (50 RPM/s). O SMC continua livre para soprar acima do piso.
- Novos parâmetros: `ventoinhas_calma_ate_c`, `ventoinhas_maximo_em_c`, `ventoinhas_minimo_rpm` e `ventoinhas_descida_rpm_por_s` e `ventoinhas_suavizacao_s`, todos ajustáveis na hora (`sudo termostato ventoinhas_calma_ate_c=65`).
- `"maximo"` continua existindo. Um `/etc/termostato.toml` já instalado com `ventoinhas = "maximo"` não muda sozinho: troque com `sudo termostato ventoinhas=curva`.

## 0.8.0: o painel

- **Painel próprio em `/`**, feito em RavenRS e compilado para o navegador (WebGL2), embutido no binário: números ao vivo com tendência, temperatura com alvo e corte do chip, ajustes com um clique, o fluxo da decisão (FlowCanvas), potência, ventoinhas, quem gasta a CPU, freio por hora, sensores do Mac, processos, o que ele fez e o histórico dos mal-educados. Temas Escuro, Claro e Vidro. Desenho aprovado em [Termostato · painel](https://claude.ai/artifact/WZ8myAZ9MhV63rVUQxrSiR), com o design system do RCB (RavenRS).
- **API:** `GET /api/estado` (JSON) e `POST /api/ajustes`, que passa pelas mesmas regras do `sudo termostato chave=valor`.
- **Quem pode ajustar:** só quem chega pelo `tailscale serve` com um login de `[painel] quem_pode_ajustar`; corpo JSON e `Origin` do próprio host, contra outra página.
- O servidor HTTP atende cada conexão numa thread: o painel baixando o wasm não atrasa o Prometheus.
- O instalador publica o painel na tailnet (`http://macpro-i7-alex:9101`) e põe o dono da máquina como quem pode ajustar.

## 0.7.0: ajustes na hora e o alvo de temperatura

- **`sudo termostato chave=valor …`** muda qualquer parâmetro com o serviço rodando, sem reiniciar: vale em até 2 s, sobrevive a um reinício e passa pela mesma validação do toml. `termostato ajustes` mostra o que está em vigor, e `termostato padrao` tira tudo.
- **`sudo termostato CPU50C`**: alvo de temperatura. Um controle integral baixa o PL1 até 8 W se precisar, para segurar a CPU no alvo; `CPULIVRE` tira.
- Novos parâmetros: `alvo_c`, `pl1_minimo_w` e `alvo_ganho_w_por_c_s`. Novas métricas: `termostato_alvo_celsius` e `termostato_ajustes`.
- Trocar as mãos de agir para outro modo devolve na hora o que estava segurado.

## 0.6.2: o observar sem repetir

- No modo observar, as mãos simulam o estado: cada cgroup é anunciado **uma vez** ("seguraria") e de novo só ao se acalmar ("devolveria"), como o modo agir faria. Antes, cada leitura decidia de novo segurar o mesmo cgroup: foram 931 anotações iguais no Grafana em 19/11/2026.

## 0.6.1: o Fedora 45

- O termostato roda no **Fedora 45** (kernel 7.3.4-300.fc45), pelo pacote gerado lá.
- Os avisos ao Grafana esperam a rede subir no boot (5, 15, 30 e 60 s) em vez de se perder: o serviço sobe antes da rede.

## 0.6.0: MegaBlaster, a conversa com o kernel e com o chip

- **Pacote RPM** (`pacote/termostato.spec`), publicado no Release `v0.6.0`: primeira instalação liga o serviço, desliga o `thermald` e volta o perfil ao `balanced`; atualização só reinicia; remoção devolve a fábrica. O `instalar.sh` e o `desinstalar.sh` passam a usar o `dnf`.

- **TCC offset:** o chip se freia sozinho a 92 °C (`MSR 0x1A2`, offset 8), gravado pelo driver `msr` do kernel, porque o `intel_tcc_cooling` não atende o Haswell. Mexe só nos 6 bits, recusa acima de 15 °C, confere lendo de novo, regrava a cada 30 s e volta a 0 no `--desinstalar`.
- **Gatilho de pressão:** o kernel acorda o termostato quando a memória para 150 ms em 1 s (PSI trigger com `poll`).
- Métricas do kernel: estrangulamento térmico, domínios do RAPL (`core`, `uncore`, `dram`) e os sensores do `applesmc`.
- O serviço carrega o `msr` ao subir.

## 0.5.0: os superpoderes

- **Processos:** CPU e memória por nome e por cgroup, lidos do `/proc`.
- **Histórico negativo:** nota de calor por processo, aprendida observando; o falso suspeito se corrige sozinho; em `historico.json`.
- **Freio antecipado:** a previsão pelos suspeitos ativos entra na curva de freio.
- **Pressão de memória** (PSI), memória disponível e swap.
- **Mãos:** segurar CPU (`cpu.max`, ou o Docker pelos `NanoCpus`) e memória (`memory.high`) no cgroup de quem abusa, e devolver quando a máquina se acalma. Modo **observar** por padrão.
- **Avisos:** cada ação vira anotação no Grafana, contador por tipo e e-mail quando as mãos agem.
- Painel: Pressão de memória, Quem está gastando a CPU, Histórico negativo, O que o termostato fez e as marcas das anotações. Alertas: O termostato segurou um processo; Pressão de memória alta.
- **Sobe cedo no boot**, antes do `sysinit.target`, do Docker, do k3s e da VM, e espera o sensor e as ventoinhas.

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

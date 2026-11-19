# Decisões

Cada decisão com o contexto em que foi tomada e o que a sustenta. Todas de 19/11/2026.

## 1. Um serviço só, e nada solto no sistema

**Contexto.** O calor foi tratado primeiro com três serviços em shell (`cpu-folego-longo`, `freio-de-calor`, `ventoinhas-no-maximo`) e ajustes avulsos.
**Decisão.** Um serviço em Rust, dono de toda a parte térmica, com a configuração num arquivo só.
**Por quê.** O dono: *"não gosto de ficar mexendo na configuração do sistema operacional"*. Tudo o que mexe no calor fica num lugar com instalador, desinstalador, testes e histórico.

## 2. Rust, e não Go

**Decisão.** Rust, Edition 2024, toolchain 1.100.0 fixado só neste projeto.
**Por quê.** A casa já é Rust (backend, VintageLightbox), o toolchain estava na fedora, e o serviço fica em ~1–2 MB de memória, sem coletor de lixo, rodando o tempo todo ao lado da VM e do Docker. Go compilaria mais rápido, mas seria a única peça numa linguagem diferente. O toolchain fixado não troca o padrão da máquina (1.99.1), que o VintageLightbox e o backend usam: trocar ali obrigaria a recompilar os `target` quentes.

## 3. Limitar potência, não desligar o turbo

**Alternativas.** Desligar o turbo (`no_turbo`) ou travar o clock (`max_perf_pct`).
**Decisão.** Limitar só a potência sustentada (PL1) e deixar o PL2 e o turbo livres.
**Por quê.** O dono: *"esse processador é um foguetinho quando recebe demanda"*. Sem turbo, a CPU ficaria presa em 2,2 GHz até para abrir um programa. Com o PL1, a arrancada sai em 3,2–3,4 GHz; só a carga longa é contida, e a CPU acha sozinha o clock que cabe no orçamento.

## 4. Nada de overclock

**Decisão.** O termostato só **limita** abaixo do que a fábrica permite. Nunca sobe clock, tensão ou limite acima do de fábrica. A varredura de medição foi até 45 W, abaixo dos 47 W de projeto do i7-4770HQ.
**Por quê.** Pedido do dono.

## 5. PL1 de 38 W no limite normal

**Contexto.** O firmware da Apple deixa o PL1 em 100 W, sem efeito nenhum; com 2 jobs de compilação a CPU chegou a 99 °C.
**Decisão.** 38 W, abaixo dos 47 W de projeto.
**Revisão pendente.** A medição mostrou que, com os 8 threads em carga contínua, o resfriamento deste MacBook só sustenta ~25 W, com o mesmo desempenho de 25 a 45 W (ver [a máquina](maquina.md)). Os 38 W continuam úteis para cargas de 1 a 4 threads, que não esquentam tanto. Baixar para ~30 W reduziria os picos sem perder nada sustentado.

## 6. Pelo driver, e não direto no registro

**Decisão.** Gravar no `/sys` do `intel_rapl` e do `applesmc`, e não nos MSRs pelo `/dev/cpu/*/msr`.
**Por quê.** O driver valida unidade, faixa e trava do registro; funciona com Secure Boot (lockdown); o efeito no MSR 0x610 é o mesmo bit a bit (conferido no chip, ver [como funciona](como-funciona.md)).

## 7. Regravar a cada leitura

**Decisão.** A cada 2 s o termostato confere o PL1 e o piso das ventoinhas e regrava o que estiver diferente, registrando 🔧 ou 🌀.
**Por quê.** Algo voltava o PL1 a 100 W de tempos em tempos. Gravar uma vez e confiar não basta, e o registro no journal revelou quem era (decisão 8).

## 8. O `thermald` desligado: o único desvio do padrão do Fedora

**Contexto.** Com `--adaptive`, o `thermald` mexia no mesmo PL1 em degraus de 5 W (95 → 90 → 80 → 70 W a cada 2 s), brigando com o termostato. Sozinho, antes, ele deixou a CPU chegar a 99 °C.
**Decisão.** O `instalar.sh` desliga o `thermald`; o `desinstalar.sh` o religa.
**Por quê.** Dois controladores no mesmo registro, com metas diferentes, oscilam para sempre. É o único ajuste fora do padrão do Fedora, e fica no projeto, com o caminho de volta.

## 9. O resto no padrão do Fedora

**Decisão.** O `instalar.sh` volta o perfil de energia ao **balanced** (estava em "performance" desde 18/11, com o governor preso no clock máximo), pela mesma interface do menu de energia do GNOME. Nenhum outro ajuste de CPU, ventoinha ou temperatura à mão no sistema.
**Por quê.** O dono: *"aquelas configurações… terá que ser desfeita e deixar padrão original do Fedora. Quem deve fazer isso é o projeto termostato"*. No balanced, o governor `schedutil` ainda leva um núcleo a ~3,3 GHz.

## 10. Parar deixa o limite normal, nunca a fábrica

**Contexto.** Na 0.1, parar devolvia o PL1 de fábrica (100 W). Numa reinstalação, com um clippy de 2 jobs rodando, a CPU chegou a 96 °C.
**Decisão.** SIGTERM, SIGINT e crash deixam o PL1 em 38 W. A fábrica só volta pelo `--desinstalar`.
**Por quê.** Um serviço de proteção, ao falhar, precisa deixar o estado seguro, e não o original. Com `Restart=always`, cada reinício abria uma janela sem limite.

## 11. Fábrica gravada uma vez só

**Decisão.** `/var/lib/termostato/fabrica.toml` é escrito só na primeira subida; o `instalar.sh` o semeia com 100 W onde o PL1 já tinha sido mexido.
**Por quê.** Lido a cada subida, o "original" seria o valor que o próprio termostato acabou de gravar, e o `--desinstalar` "devolveria" 38 W.

## 12. A CPU nunca fica solta

**Contexto.** O painel mostrava "Freio: solto" quando o freio de calor não agia, passando a ideia de CPU livre.
**Decisão.** O vocabulário: **limite normal** (sempre valendo) e **freio de calor** (a camada que aperta mais no calor). Nada de "solto". As métricas viraram `termostato_freio_acionado` e `termostato_freio_intensidade`.
**Por quê.** O dono: *"Freio puxado é não deixar o processador solto pra fazer o que quiser"*.

## 13. Curva com previsão, no lugar do liga-desliga

**Contexto.** A varredura mostrou picos de 99–100 °C na arrancada, antes de o freio antigo (5 leituras acima de 92 °C) reagir.
**Decisão.** Previsão por reta de mínimos quadrados nas últimas 5 leituras, 10 s à frente; o PL1 sai de uma curva linear de 80 °C (38 W) a 92 °C (25 W), aplicada à maior entre a temperatura de agora e a prevista; freia na hora e solta a 0,5 W/s.
**Por quê.** O dono: *"Crie um sistema curva para prever um pico e puxar o freio"*. A reta é simples, explicável e robusta a ruído com 5 pontos; frear rápido e soltar devagar dispensa a histerese e evita a sanfona.

## 14. Ventoinhas no máximo

**Decisão.** O piso de cada ventoinha vai ao máximo dela (`ventoinhas = "maximo"`).
**Por quê.** A saída de ar do MacBook é pela dobradiça e o chassi esquenta rápido; soprar antes de esquentar reduz os picos. O custo é o barulho e a poeira, que volta mais rápido. `"automatico"` devolve a decisão ao SMC.

## 15. HTTP e Prometheus à mão

**Decisão.** `std::net::TcpListener` e o formato de texto do Prometheus escritos à mão, sem framework.
**Por quê.** Um pedido a cada 5 s, do loopback. Um runtime assíncrono e um cliente de métricas seriam a maior parte do binário (hoje ~570 KB) para poucas linhas.

## 16. Só no loopback, como root, mas trancado

**Decisão.** `/metrics` em `127.0.0.1:9101`; o serviço roda como root com `ProtectSystem=strict`, gravando só em `/sys` e no próprio estado.
**Por quê.** Escrever no RAPL e no SMC exige root. O resto do sistema fica só para leitura, e ninguém de fora lê as métricas.

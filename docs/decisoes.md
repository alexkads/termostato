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

**Revista em 20/11/2026 (decisão 14b).**

## 14b. Ventoinhas pela curva

**Contexto.** Com o piso sempre no máximo, a fedora ficava com as ventoinhas a ~6000 RPM mesmo parada, a 45–55 °C.
**Decisão.** O padrão passa a ser `ventoinhas = "curva"`: mínimo até 60 °C, máximo a partir de 80 °C (onde o freio começa), em linha reta entre os dois, sobre a **média móvel de 20 s** da temperatura; máximo sem leitura de temperatura. Desce a 50 RPM/s, gravando em degraus de 100 RPM.
**A primeira versão seguia o freio** (freio acionado → máximo) e a temperatura de agora. Instalada na fedora, cada arrancada do turbo (68–77 °C por poucos segundos, previsão de 88–101 °C) acionava o freio pela previsão e punha as ventoinhas no máximo, com a CPU de volta a 47–55 °C logo depois. A média separa o pico, que o freio segura cortando a potência na hora, do calor que fica, que é o trabalho da ventoinha.
**Por quê.** O dono: *"Eu gostaria que o cooler não ficasse acelerado sem necessidade"*. O motivo da decisão 14 (soprar antes de esquentar) continua de pé onde importa: a curva chega ao máximo **antes** do freio terminar de apertar, e uma carga longa leva a média a 80 °C em menos de um minuto. Abaixo de 60 °C não há pico a evitar, só barulho e poeira. Descer devagar é o mesmo truque do freio contra a sanfona.

## 15. HTTP e Prometheus à mão

**Decisão.** `std::net::TcpListener` e o formato de texto do Prometheus escritos à mão, sem framework.
**Por quê.** Um pedido a cada 5 s, do loopback. Um runtime assíncrono e um cliente de métricas seriam a maior parte do binário (hoje ~570 KB) para poucas linhas.

## 16. Só no loopback, como root, mas trancado

**Decisão.** `/metrics` em `127.0.0.1:9101`; o serviço roda como root com `ProtectSystem=strict`, gravando só em `/sys` e no próprio estado.
**Por quê.** Escrever no RAPL e no SMC exige root. O resto do sistema fica só para leitura, e ninguém de fora lê as métricas.

## 17. Os superpoderes começam observando

**Decisão.** As mãos (segurar CPU e memória de processos e contêineres) vêm com o modo `observar` como padrão; agir é decisão do dono, depois de ver.
**Por quê.** O dono: *"deve notificar o grafana o que ele está fazendo, pois pode ser nocivo para o ambiente"*. Segurar o Postgres de um e2e, por exemplo, pode estourar os prazos dos testes. O histórico do modo observar mostra o que aconteceria, sem risco.

## 18. Segurar no cgroup em que o processo mora, e nunca matar

**Decisão.** `cpu.max` e `memory.high` no cgroup do próprio processo; para contêiner, o pedido de CPU vai ao Docker. Nada de mover processos de cgroup, nada de `memory.max` nem de matar.
**Por quê.** Mover processos confunde o systemd. O `memory.high` desacelera e força o processo a devolver memória, sem o OOM; matar é o último recurso do kernel e do `systemd-oomd`.

## 19. Aprender por média móvel, com a nota que cai

**Decisão.** A nota de calor é uma média móvel com peso pela fatia, e observações frias puxam a nota para baixo.
**Por quê.** Uma regra fixa não se adapta. Se as aletas forem limpas e a mesma build passar a esquentar menos, as notas descem sozinhas, e o falso suspeito sai da lista sem intervenção.

## 20. O TCC offset pelo driver `msr`

**Contexto.** O driver próprio do kernel (`intel_tcc_cooling`) não atende o Haswell. O dono pediu para forçar o kernel a atendê-lo.
**Decisão.** Gravar o `MSR 0x1A2` pelo driver genérico `msr`, com as conferências no termostato (programável, só os 6 bits, até 15 °C, leitura de volta). É uma exceção à decisão 6.
**Por quê.** É a mesma gravação que o driver de TCC faria, sem um módulo próprio para recompilar a cada atualização de kernel. O custo é o kernel marcar a sessão como "tainted", um rótulo para relatórios de bug.

## 21. Gatilho do kernel, e não só leitura

**Decisão.** Inscrever um gatilho de pressão no PSI e esperar num `poll`, além da leitura a cada 2 s.
**Por quê.** O sufoco de memória pode travar a máquina em segundos; o kernel acordando o termostato reage em milissegundos, sem gastar CPU na espera.

## 22. Subir antes de todo mundo

**Decisão.** O serviço sobe antes do `sysinit.target`, logo depois do udev, e antes do Docker, do k3s e da VM.
**Por quê.** O firmware volta o PL1 aos 100 W a cada boot; em 19/11/2026 a CPU tocou 100 °C nos primeiros segundos, antes de o termostato assumir.

## 23. Ajustes na hora, por um arquivo do estado

**Contexto.** O dono pediu para passar parâmetros ao termostato, como `CPU50C`, e mudá-los para testar sem reiniciar o serviço.
**Decisão.** `sudo termostato chave=valor …` grava os ajustes em `/var/lib/termostato/ajustes.toml`, que o serviço relê a cada volta e põe por cima do `/etc/termostato.toml`. `CPU50C` é o atalho de `alvo_c=50`.
**Por quê.** Um arquivo só do root dispensa abrir porta de escrita no `/metrics`, que qualquer usuário local alcança. O pacote nunca toca o estado, então o ajuste sobrevive à atualização; e a mesma validação do toml roda antes de gravar e de novo ao ler.

## 24. O alvo é integral, e não uma curva

**Decisão.** O alvo de temperatura desce o PL1 em proporção ao erro e acumula, entre 8 W e o normal; vale o menor entre ele e a curva.
**Por quê.** O PL1 que segura uma temperatura muda com a carga. O controle integral acha esse valor sem tabela, e a curva continua sendo a rede de segurança acima dele.

## 25. O painel em RavenRS, embutido no binário

**Contexto.** O dono pediu uma interface própria, "linda", desenhada com o design system do RCB (RavenComponentBook, o do RavenRS) e feita com os componentes do RavenRS em WebGL2.
**Decisão.** Uma crate à parte (`painel/`, workspace próprio, RavenRS fixado num commit) compilada para wasm por `painel/compilar.sh`; o resultado (`painel/dist/`) fica no repositório e o termostato o embute com `include_bytes!`. O app não fala HTTP: a página busca `/api/estado` a cada segundo e entrega o JSON ao wasm, e os ajustes voltam por `window.termostatoAjustar`.
**Por quê.** O RPM continua compilando em segundos, sem toolchain de wasm na fedora, e o painel nunca fica de versão diferente do serviço. O custo são 6 MB de wasm no git a cada mudança do painel.

## 26. Ajustar pelo painel só pela tailnet

**Decisão.** O `POST /api/ajustes` exige o cabeçalho `Tailscale-User-Login` de um login listado em `[painel] quem_pode_ajustar` (que só muda no toml, reiniciando), corpo JSON e `Origin` do próprio host.
**Por quê.** O `tailscale serve` identifica quem pede sem senha nova. O JSON obriga o navegador a um preflight de CORS que o servidor não responde, e o `Origin` barra outra página aberta no navegador do dono. Um processo local poderia forjar o cabeçalho no loopback, mas na fedora o único usuário já tem `sudo` sem senha: não há ganho de privilégio.

## 27. O estresse dentro do termostato, sem `stress-ng`

**Contexto.** Em 22/11/2026, para descobrir onde a fedora para, o teste foi feito à mão: `openssl speed` para a carga e um laço de shell lendo o sysfs. O `openssl speed` terminou em ~20 s em vez dos 90 s pedidos, e a potência saiu vazia na primeira volta, porque só o root lê o `energy_uj`.
**Decisão.** Um subcomando, `termostato estresse`, que gera a carga em Rust (CPU, memória com conferência, disco com `fsync` e leitura fora do cache) e lê os sensores com o mesmo código do serviço (`hardware.rs`). Ele não mexe em limite, ventoinha nem ajuste.
**Por quê.** Os sensores que dizem onde está o gargalo (`TC0P`, os `TH0*` do dissipador, as ventoinhas em % do máximo, os estrangulamentos) já são lidos pelo termostato, e o `stress-ng` não os conhece. Um pacote a menos na fedora, e o teste fica repetível: o mesmo comando antes e depois de trocar a pasta térmica. Deixar o freio com o serviço mede a máquina com a configuração real.


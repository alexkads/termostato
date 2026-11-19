# A história: 19/11/2026

Como o termostato nasceu, no mesmo dia, do primeiro susto à curva com previsão.

## Antes

A fedora (`macpro-i7-alex`) é um MacBookPro11,2 de 2014 que travava no macOS e ganhou vida nova com o Linux e uma manutenção geral, com a pasta térmica trocada. Ela virou a máquina de testes da casa: Docker, VMs, a bateria noturna no lugar do CI e, desde 19/11, o único lugar com Docker, porque o Mac do dono não tem mais o Docker Desktop.

## O susto

- **O e2e do VintageLightbox**, compilando com 4 jobs, levou a CPU a **100 °C**, o ponto em que ela se desliga, com as ventoinhas no máximo.
- Com 2 jobs ela ainda chegava a **99 °C**, e a trava de calor do script do e2e (96 °C) encerrou a rodada no meio.

## A causa

O registro de limite de potência mostrou o motivo: **o firmware da Apple deixa o PL1 em 100 W**, mais que o dobro dos 47 W de projeto do i7-4770HQ. No macOS, o próprio sistema segurava a CPU por software; no Linux, ninguém segurava. O PL2 (125 W) e a janela de 28 s eram os de fábrica.

## Os provisórios

No mesmo dia, ainda em shell:

1. `cpu-folego-longo.service`: PL1 em 38 W, mantendo o turbo curto. O dono tinha descartado desligar o turbo: *"esse processador é um foguetinho quando recebe demanda"*.
2. `ventoinhas-no-maximo.service`: piso das ventoinhas no máximo a cada boot.
3. `freio-de-calor.service`: PL1 em 25 W acima de 92 °C, voltando a 38 W abaixo de 80 °C.

## O termostato

O dono pediu um serviço em Rust, num projeto privado, pronto para o Grafana, porque não gosta de mexer na configuração do sistema operacional. Nasceu o `alexkads/termostato`:

- **0.1.0:** os três provisórios num serviço só, com métricas, painel e alertas. O teste com o hardware real mostrou **o PL1 de novo em 100 W**, e o termostato regravou 38 W na largada.
- **O culpado:** em 10 minutos, 20 regravações, em degraus de 5 W (95 → 90 → 80 → 70 W). Era o `thermald --adaptive`, mexendo no mesmo registro. Ele foi desligado pelo instalador; no minuto seguinte, nenhuma regravação.
- **0.2.0:** uma sessão vizinha, compilando o VintageLightbox na fedora, viu a CPU em 96 °C durante uma reinstalação do termostato. A 0.1 devolvia os 100 W de fábrica ao parar. Desde então, parar deixa o limite normal, e a fábrica só volta com o `--desinstalar`. A troca da 0.1 para a 0.2 foi feita com SIGKILL, para a parada antiga não rodar.
- **O padrão do Fedora:** o dono pediu que tudo voltasse ao padrão, com o termostato como responsável. O perfil de energia, em "performance" desde 18/11, voltou ao balanced. O `thermald` desligado ficou como o único desvio, aplicado e desfeito pelo projeto.
- **A prova no chip:** a leitura direta do `MSR 0x610` mostrou os 38 W gravados no registro da CPU.
- **0.3.0:** o vocabulário. *"Freio puxado é não deixar o processador solto pra fazer o que quiser."* O painel deixou de dizer "solto": há o **limite normal**, sempre valendo, e o **freio de calor**, que aperta mais.
- **A varredura:** de 25 a 45 W, sem overclock, mesmo desempenho sustentado (~1.270 MB/s, ~2,43 GHz, ~25 W) e picos de 99–100 °C na arrancada, antes de o freio reagir.
- **0.4.0:** *"Crie um sistema curva para prever um pico e puxar o freio."* Previsão pela tendência das últimas 5 leituras, curva de 80 °C (38 W) a 92 °C (25 W), freia na hora e solta devagar.
- **O rojão:** uma build de release do VintageLightbox do zero, com os 8 threads, para ver se a 0.4 segura (resultado em [a máquina](maquina.md)).

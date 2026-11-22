# A máquina

A fedora (`macpro-i7-alex`), lida em 19/11/2026.

## Hardware

| Peça | O que é |
|---|---|
| Modelo | MacBookPro11,2 (Retina 15", 2014) |
| CPU | Intel Core i7-4770HQ (Haswell / Crystal Well): 4 núcleos, 8 threads, base 2,2 GHz, turbo até 3,4 GHz, 47 W de projeto |
| Caches | L1d 128 KiB, L2 1 MiB, L3 6 MiB e **L4 de 128 MiB (eDRAM)**, raro, só da família Crystal Well |
| Instruções | AVX2, FMA, BMI2, AES em hardware |
| GPU | Intel Iris Pro 5200 (integrada, até 1.200 MHz), que usa a mesma eDRAM |
| Memória | 16 GB DDR3-1600 (2 × 8 GB) |
| Disco | SSD Apple SM0256F, 256 GB |
| Bateria | 126 ciclos |
| Corte de emergência do hardware (TjMax) | 100 °C (MSR 0x1A2) |

## Limites de potência (RAPL)

| | Fábrica (firmware da Apple) | Com o termostato |
|---|---|---|
| PL1 (sustentado, janela de ~28 s) | **100 W**, na prática sem limite | 38 W, descendo até 25 W pela curva de freio |
| PL2 (curto, ~2 ms) | 125 W | 125 W, sem mudança |

O RAPL tem quatro domínios: o pacote (`package-0`, o que o termostato controla), os núcleos (`core`), o gráfico e o resto do chip (`uncore`) e a memória (`dram`).

## Sensores

O `coretemp` dá a temperatura do pacote, que é a que o termostato usa. O controlador da Apple (`applesmc`) expõe mais **35 sensores**: núcleos, GPU, memória, chipset, aletas, bateria, apoio de mãos e Wi-Fi. Por enquanto eles não entram no termostato (ver "Próximos passos").

As duas ventoinhas vão a 6.156 e 5.700 RPM no máximo.

## Quanto ela aguenta: a varredura do limite

Carga de 90 s nos 8 threads (`openssl speed -multi 8 sha256`), com o limite normal fixado em cada valor pelo `termostato.toml`, de 25 a 45 W. Sem overclock: 45 W ainda fica abaixo dos 47 W de projeto. Médias depois dos primeiros 30 s, com a versão 0.3 (freio liga-desliga):

| Limite (PL1) | Vazão | Temp. média | Pico | Frequência | Potência real | Estrangulamento do hardware |
|---|---|---|---|---|---|---|
| 25 W | 1.288 MB/s | 80,8 °C | 99 °C | 2.434 MHz | 25,2 W | 4.548 eventos |
| 30 W | 1.263 MB/s | 83,3 °C | 89 °C | 2.486 MHz | 26,8 W | 2.688 eventos |
| 38 W | 1.267 MB/s | 82,7 °C | 85 °C | 2.415 MHz | 24,9 W | 5.135 eventos |
| 45 W | 1.282 MB/s | 84,4 °C | 100 °C | 2.433 MHz | 25,7 W | 6.560 eventos |

O que isso mostra:

- **O desempenho sustentado é o mesmo em todos os limites:** ~1.270 MB/s, ~2,43 GHz e ~25 W. É o teto real de resfriamento deste chassi com os 8 threads em carga contínua.
- **Subir o limite só aumenta os picos**, sem ganhar nada.
- **Os picos de 99–100 °C acontecem na arrancada**, quando o turbo (PL2) queima tudo nos primeiros segundos, antes de o freio liga-desliga reagir. Foi isso que motivou a curva com previsão da 0.4.
- O contador de estrangulamento do pacote (`thermal_throttle/package_throttle_count`) registrou **342 mil eventos** desde o boot até 19/11, a maior parte antes do termostato.

## O rojão: a build do VintageLightbox

Em 19/11/2026, com a 0.4.0 (curva com previsão) instalada: uma build de release do VintageLightbox **do zero**, com os **8 threads** (`CARGO_BUILD_JOBS=8`), num `target` descartável. É a carga mais pesada que esta máquina recebe; com a regra antiga, compilações assim levavam a CPU a 99–100 °C.

O que o termostato registrou:

| Momento | CPU | Previsão | Freio | PL1 | Potência real |
|---|---|---|---|---|---|
| Arrancada | 87 °C | 129 °C em 10 s (+4,2 °C/s) | acionado na hora | 25,0 W | — |
| 6,5 min | 83 °C | 85 °C | 59 % | 30,3 W | 29,1 W |
| 10 min | 88 °C | 90 °C | 85 % | 27,0 W | — |
| 13,7 min (fase final, 1 `rustc`) | 80 °C | 80 °C | 88 % | 26,5 W | — |

A linha do journal na arrancada:

```
🔥 87 °C agora, 129 °C previstos em 10 s (+4.2 °C/s): freio de calor acionado, PL1 em 25.0 W
```

- **A previsão funcionou:** o freio agiu aos **87 °C**, ao ver a temperatura subindo +4,2 °C/s. Na varredura com a regra antiga, o freio só agia depois de a CPU já estar em 99–100 °C.
- Durante a build, a CPU ficou entre **80 e 88 °C**, com a frequência média em ~2,4 GHz. Os "129 °C previstos" não são uma temperatura real: é para onde ela iria naquele ritmo, o bastante para a curva saber que precisava frear forte.
- **A medição não chegou ao fim.** Por volta dos 14 min, já depois do último dado (80 °C), a fedora parou de responder por completo (SSH, Tailscale e Grafana), com o e2e de outra sessão na fila para começar. A causa ainda não foi investigada; o último dado de temperatura antes disso estava normal.

## O diagnóstico de 22/11/2026: o calor não sai do chip

Com o freio do termostato desligado (`pl1_normal_w=100 pl1_freio_w=100`, por ajuste), as ventoinhas pela curva e o Jenkins rodando ao lado, o comando novo `sudo termostato estresse segundos=60` (CPU nos 8 threads, 3 GB de memória e 1 GB de disco):

| Medida | Resultado |
|---|---|
| Chip | 95 °C em 2 s; **99–100 °C** do 14º segundo até o fim |
| Núcleos (média da segunda metade) | 97 / 97 / 99 / 92 °C. O núcleo 2 é sempre o mais quente, e o 3 o mais frio |
| Potência | 41 W de média, 46 W de pico |
| Frequência | ~3.090 MHz, caindo para 2.900 nos estrangulamentos |
| Ventoinhas | 100 % do máximo (6.157 / 5.700 RPM) do começo ao fim |
| Perto da CPU (`TC0P`) | 56 → 66 °C |
| **Dissipador (`TH0*`)** | **44 → 47 °C**: quase não esquenta |
| Estrangulamentos | 10.289 no pacote, **16,4 s** dos 61 s estrangulado. Por núcleo: 765 / 163 / **9.719** / 0 |
| Memória | 26 voltas de escrita e conferência, 0 erros |
| Disco | Grava 113 MB/s e lê 92 MB/s (fora do cache), 0 erros |

O diagnóstico do comando: 🧱 **o calor não sai do chip**. As ventoinhas já estão no máximo e o ar interno está frio (39 °C), mas o dissipador não esquenta: o calor fica preso entre o chip e o dissipador. A pasta térmica **já foi trocada** (ver [Problemas conhecidos](operacao.md#problemas-conhecidos)). Então o suspeito é o **contato** ou o **heat pipe**: ver a medição abaixo. Os núcleos estão parelhos (7 °C entre o mais quente e o mais frio, abaixo dos 8 °C do critério), mas o núcleo 2 sozinho fez 94 % dos estrangulamentos.

Uma carga curta antes disso (`openssl speed`, ~20 s) chegou a puxar **57–72 W**, com o chip de 64 a 98 °C em 2 s.

### O heat pipe: o calor não chega às aletas

Um comentário antigo dizia que **a solda do heat pipe com a placa de cobre estava quebrada**. Para conferir, os 31 sensores do `applesmc` foram lidos antes e no fim de 45 s de carga (`sudo termostato estresse cpu segundos=45`, ~38 W de média, ventoinhas no máximo):

| Onde | Sensores | Subiu |
|---|---|---|
| **O chip** (núcleos, pacote, GPU, system agent) | `TC0E`, `TC0F`, `TC1C`–`TC4C`, `TCXC`, `TCGC`, `TCSA` | **+12 a +24 °C** (até 97–99 °C) |
| A placa ao lado da CPU | `TC0P` | +11 °C |
| **O dissipador e os heat pipes** | `TH0A`, `TH0B`, `TH0V`, `Th1H`, `Th2H` | **−1 a +2 °C** |
| O ar dentro e o de saída | `Ta0P`, `TaSP` | 0 °C |
| Memória, chipset, bateria, apoio de mãos | `TM0P`, `TPCD`, `TB*T`, `Ts*` | 0 a +1 °C |

E 20 s **depois** de parar a carga, com a CPU quase parada, o chip ainda estava em **84–86 °C**.

O que isso quer dizer:

- **38 W por 45 s são ~1,7 kJ.** Com o dissipador funcionando, esse calor passa pelo heat pipe até as aletas, e as aletas e o ar de saída esquentam vários graus. Aqui eles **não se mexeram**, com as ventoinhas no máximo.
- **Pasta ruim não explicaria isso sozinha.** Com pasta ruim, o chip fica mais quente, mas o calor que ele dissipa continua indo para o dissipador, e as aletas esquentam. E a pasta já foi trocada.
- **O chip demora a esfriar** mesmo sem carga: o calor em volta dele não tem para onde ir.
- O que o calor esquenta é **a placa** (`TC0P`, +11 °C). É o caminho de quem não tem dissipador.

É o retrato de um **heat pipe que não transporta calor**: a solda com a placa de cobre trincada, ou o tubo furado (sem o fluido, ele vira um tubo de cobre comum). O contato do dissipador solto daria um quadro parecido, e é mais barato de conferir.

**Mas outros donos veem o mesmo quadro, e trocar o dissipador não resolveu para eles** (pesquisado em 22/11/2026):

- [Num MacBook Pro Retina 15" Late 2013](https://forums.macrumors.com/threads/hypothesis-re-high-temp-overheating-on-late-2013-macbook-pro-retina-due-to-high-vapor-point-in-heatpipe.2238074/), o dono via a CPU em 80 °C ou mais com o heat pipe bem mais frio. Outro dono trocou o heat pipe **três vezes**, comprando pelo AliExpress, e nada mudou.
- [No iFixit](https://ts.ifixit.com/Answers/View/538140/Is+my+heat+sink+broken), um dono trocou a pasta e depois **dois dissipadores usados**. A carga seguiu em 95–98 °C, com o calor chegando só ao começo das aletas.
- [Num Mid 2014 15"](https://forums.macrumors.com/threads/cannot-get-a-good-repaste-on-2014-15-macbook-pro.2437146/), o dono passou por PTM7950 duas vezes, MX-6 duas vezes e trocou o dissipador. O que ajudou foi aplicar a pasta **o mais fina possível** e apertar o dissipador **por igual**: a diferença entre núcleos caiu de 15 °C para 2–3 °C. Mesmo assim, com undervolt, pads térmicos até a carcaça e uma base com ventoinha, ele segura 3,5 GHz com o pacote em **96–97 °C**. Nesses modelos, 95–100 °C em carga cheia é comum.

Então o dissipador frio com o chip quente pode ser **o jeito deste projeto** (os heat pipes só levam bem o calor com o chip já muito quente), e não um defeito desta máquina. Comprar um dissipador novo **pode não mudar nada**.

**O que fazer:**
1. Fazer primeiro o teste da mão na saída de ar (abaixo). Se o ar sair quente, o dissipador está trabalhando, e não há o que trocar.
2. Abrir e conferir os parafusos do dissipador (os de mola, em volta da CPU). Solto, basta apertar.
2. Olhar a junta entre o heat pipe e a placa de cobre sobre o chip. Trinca ou folga ali confirma a quebra.
3. Trocar o dissipador **só se a junta estiver visivelmente quebrada**: o **dissipador inteiro** (o heat pipe vem soldado nele), com pasta nova. Atenção ao modelo: o MacBookPro11,2 (A1398, só Iris Pro) usa o dissipador **sem a placa da GPU dedicada**. O do MacBookPro11,3 (com GeForce GT 750M) é outro.
4. Depois, repetir `sudo termostato estresse segundos=60`. Com o dissipador bom, os `TH0*` e o ar de saída sobem junto com o chip, e o 🧱 vira 🔥 ou ✅.

**Um teste sem abrir:** com o `termostato estresse` rodando, ponha a mão na saída de ar da dobradiça. Com o chip a 99 °C e as ventoinhas no máximo, o ar deveria sair quente. Saindo morno, o calor não está chegando às aletas.

### O que o chip diz que o freou

Os registros de motivo do próprio processador (`MSR 0x690`, `MSR_CORE_PERF_LIMIT_REASONS`, e `0x1B1`), lidos desde o boot de 19/11:

| Motivo | Aconteceu? |
|---|---|
| Térmico (o chip no limite) | **sim** |
| Limite de potência PL1 e PL2 | **sim** |
| Corrente (EDP) | **sim** |
| Turbo máximo | sim (é o teto normal) |
| **PROCHOT externo** (o SMC ou o carregador freando a CPU por fora) | **nunca** |
| Regulador de tensão quente (VR) | nunca |

O processador continua agindo como um de notebook: os estados de repouso fundos funcionam (a CPU 0 passou 57 % do tempo em C7s) e ninguém de fora o freou. O que o limita é o próprio calor.

### O TCC de 92 °C não segura

O termostato grava o TCC offset de 8 °C (`MSR 0x1A2`), para o chip se frear a 92 °C. No teste, o chip ficou em **99–100 °C** mesmo assim. Neste Haswell, o offset é aceito e lido de volta, mas não muda onde o chip se freia. Quem protege o chip é o corte de fábrica (TjMax de 100 °C).

### A bateria descarrega com o carregador ligado

O carregador está ligado (`ADP1 online = 1`), mas a bateria passa a **descarregar** sempre que a CPU acelera:

| CPU | Bateria |
|---|---|
| 11–12 W | carregando, ~600 mA |
| 17 W | descarregando, 1.312 mA |
| ~40 W (no estresse) | descarregando, 1.000–1.360 mA (~15 W saindo da bateria) |

Os limites da Apple (PL1 de 100 W) deixam o chip pedir mais do que o carregador dá. Até agora o SMC não freou a CPU por isso (sem PROCHOT), mas uma carga longa pode esvaziar a bateria. O MacBookPro11,2 pede o carregador MagSafe 2 de **85 W**. Vale conferir a etiqueta do que está ligado. Bateria: 127 ciclos, 7,77 Ah de 8,6 Ah de fábrica (90 %).

## O travamento de 19/11/2026: falta de memória, e não calor

A fedora parou às 18:32 (o boot −4 do journal termina sem desligamento). O dump do OOM no `journalctl -k` mostra:

- **O swap de 16 GB acabou:** "Free swap = 208kB".
- **O OOM killer disparou 4 vezes e matou os pequenos:** `traefik` (18 MB), `coredns`, `ibus-engine-tb` e `systemd-coredump`.
- **Quem segurava a memória eram 8 `rustc`**, cada um com 1,3–1,6 GB de RAM e ~1,4 GB de swap. Era a build do VintageLightbox com `CARGO_BUILD_JOBS=8` (ver "O rojão" acima).
- **Antes de parar,** o Docker, o Tailscale e o polkit reclamavam de timeout.
- **Às 18:29 o chip chegou a 97 °C,** e o freio segurou.

O OOM killer escolhe pela pontuação de cada processo (`oom_score_adj`), e não pelo tamanho. Por isso os `rustc` sobreviveram.

No boot seguinte (desde 19/11, 19:55), não houve OOM, mas o swap trocou muito em rajadas: ~56 GB entraram e ~78 GB saíram em 2,4 dias, mais nas madrugadas.

Desde então, a caixa-preta grava quem segura a memória, o swap por segundo e as mortes do OOM a cada leitura. No boot seguinte a uma queda, o journal diz 💥 com a última leitura (ver [Operação](operacao.md#a-caixa-preta)).

## Próximos passos

O inventário mostrou o que o hardware oferece e o termostato ainda não usa:

- **O contador de estrangulamento térmico** como métrica (`termostato_estrangulamento_total`): é o melhor termômetro do sucesso do termostato. Com ele bem ajustado, o contador para de subir.
- **Os 35 sensores do `applesmc`** no painel, para ver aletas, GPU e chipset junto com a CPU, e o apoio de mãos, que é o que se sente na pele.
- **Os domínios `core`, `uncore` e `dram`** do RAPL, para saber se o calor vem dos núcleos ou da GPU.
- **Revisar o limite normal** para ~30 W, já que o desempenho sustentado não muda e os picos caem.

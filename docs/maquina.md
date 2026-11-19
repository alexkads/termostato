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

## Próximos passos

O inventário mostrou o que o hardware oferece e o termostato ainda não usa:

- **O contador de estrangulamento térmico** como métrica (`termostato_estrangulamento_total`): é o melhor termômetro do sucesso do termostato. Com ele bem ajustado, o contador para de subir.
- **Os 35 sensores do `applesmc`** no painel, para ver aletas, GPU e chipset junto com a CPU, e o apoio de mãos, que é o que se sente na pele.
- **Os domínios `core`, `uncore` e `dram`** do RAPL, para saber se o calor vem dos núcleos ou da GPU.
- **Revisar o limite normal** para ~30 W, já que o desempenho sustentado não muda e os picos caem.

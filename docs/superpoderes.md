# Os superpoderes

Da 0.5 em diante, o termostato não olha só a temperatura. Ele vê os processos, aprende quem esquenta, sente a pressão de memória, segura quem abusa, conversa com o kernel e grava uma proteção no próprio chip. E avisa o Grafana de tudo o que faz.

> *"o projeto termostato precisa ser mais inteligente vendo o que está acontecendo no sistema operacional e prever processos que possam subir a temperatura já puxando o freio antecipadamente"*, *"pode verificar os processos e dá uma segurada nos mal educados"*, *"deve notificar o grafana o que ele está fazendo, pois pode ser nocivo para o ambiente"*, *"faça o termostato conversar com o Kernel do Fedora e deixa ele MegaBlaster"*. Pedidos do dono, 19/11/2026.

## O mapa

| | Sentido (lê) | Mão (age) |
|---|---|---|
| 🌡️ Calor | temperatura, tendência, potência, 35 sensores do Mac | curva de freio no PL1 (0.4) |
| 🧠 Processos | CPU e memória por nome e por cgroup, a cada 2 s | freio **antecipado** pelo histórico |
| 💾 Memória | pressão (PSI), memória disponível, swap | `memory.high` no cgroup de quem mais ocupa |
| 🐳 Contêineres | o nome de cada um, pela API do Docker | `NanoCpus` pela API do Docker |
| ⚡ Kernel | gatilho de pressão, estrangulamento, domínios do RAPL | o laço acorda na hora |
| 🛡️ Chip | o registro de temperatura (`MSR 0x1A2`) | TCC offset: o chip se freia sozinho a 92 °C |
| 📣 Grafana | — | anotação a cada ação, contador por tipo, e-mail quando age |

## O guardião ([`guardiao.rs`](../src/guardiao.rs))

A peça que junta os superpoderes. A cada leitura:

1. **Antes da curva**, vê os processos e diz quanto os suspeitos ativos costumam esquentar (a terceira fonte da previsão).
2. A curva decide o PL1, com a temperatura de agora, a tendência e a previsão por processos.
3. **Depois da curva**, aprende com o que aconteceu, lê a pressão de memória, decide o que as mãos fazem e avisa o Grafana.

## Os processos ([`processos.rs`](../src/processos.rs))

O `/proc/[pid]/stat` dá o nome, o tempo de CPU (`utime` + `stime`, em ticks) e a memória residente; o `/proc/[pid]/cgroup` dá o cgroup (`0::/caminho`). O uso de CPU vem do **delta** de ticks entre duas leituras, somado por nome e por cgroup.

A **fatia** é da máquina inteira: 1,0 = os 8 threads ocupados. Um processo que ocupa um núcleo inteiro tem fatia 0,125. Um processo que aparece pela primeira vez só conta da segunda leitura em diante, porque antes não há delta.

O custo, medido na fedora: **0,5 % de um núcleo e 1,5 MB de memória**.

## O histórico negativo ([`historico.rs`](../src/historico.rs))

A cada leitura, os nomes com fatia de pelo menos 5 % recebem uma **observação** do calor daquele momento, numa média móvel exponencial (`nota_c_s`, em °C/s), com peso pela fatia (raiz quadrada, para quem tem 80 % levar quase todo o mérito e quem tem 5 %, pouco):

| Momento | Valor da observação |
|---|---|
| Subindo mais que 0,3 °C/s | a própria tendência |
| Parada, mas acima de 80 °C | 0,1 °C/s por grau acima: o processo está mantendo a máquina quente |
| Fria e sem subir | zero: a nota **cai** |

Com nota de pelo menos 1,0 °C/s e 3 observações de calor, o nome entra no **histórico negativo** (📒 no journal). A nota que cai é o que corrige o falso suspeito: um processo que gasta CPU sem esquentar perde a nota e sai da lista sozinho.

O histórico fica em `/var/lib/termostato/historico.json`, gravado a cada 60 s e ao parar. Nomes não vistos há 30 dias são esquecidos. A primeira suspeita que ele aprendeu sozinho, minutos depois de instalado, foi a VM Windows (`qemu-system-x86`, +1,1 °C/s em 13 observações).

### O freio antecipado

Quando um suspeito volta a gastar CPU, a previsão ganha a terceira fonte:

```
prevista_por_processos = agora + maior_nota × √(fatia_dos_suspeitos) × previsao_s
```

A curva usa a maior das três (agora, tendência, processos). Com o `rustc` do VintageLightbox (+4,2 °C/s, medido no rojão) aparecendo com 80 % da CPU numa máquina parada a 60 °C, a previsão passa de 90 °C, e a curva freia **antes** de o sensor reagir (🧠 no journal). Um processo que esquenta pouco (+2 °C/s) prevê ~78 °C e não freia uma máquina fria, e há teste para os dois casos.

## A memória ([`pressao.rs`](../src/pressao.rs))

O PSI (`/proc/pressure/memory`) mede **tempo perdido**: `some avg10=12.5` quer dizer que, nos últimos 10 s, em 12,5 % do tempo algum processo ficou parado esperando memória. A memória livre pode parecer razoável enquanto a máquina já sufoca; o PSI mostra o sufoco de verdade.

Em 19/11/2026 a fedora travou por falta de memória: uma build com 8 jobs, a VM Windows, o k3s e os contêineres do e2e em 16 GB. O kernel chamou o OOM com 16 GB de swap trocando sem parar, e a máquina só voltou reiniciando no botão. Este é o sentido que teria visto isso chegando.

## As mãos ([`maos.rs`](../src/maos.rs))

O freio do RAPL aperta a CPU **inteira**. As mãos apertam **só o culpado**, no cgroup em que ele já mora (o serviço da bateria noturna, a sessão do SSH, o escopo do contêiner), sem movê-lo de lá. Mover processo de cgroup confunde o systemd.

| Situação | O que a mão faz |
|---|---|
| Freio de calor acima de 30 % | o cgroup que mais gasta CPU fica com no máximo `fatia × núcleos × (1 − intensidade)` núcleos, nunca menos que 1, pelo `cpu.max`. Num contêiner, o pedido vai ao Docker (`POST /containers/{id}/update` com `NanoCpus`), o mesmo controle do `docker update --cpus` |
| Pressão de memória acima de 10 %, ou menos de 10 % disponível | o cgroup que mais ocupa memória fica com o `memory.high` em 90 % do que usa agora: o kernel o obriga a devolver memória e ele anda mais devagar |
| 60 s sem freio e sem pressão | os limites voltam ao original, um por leitura |

As regras de segurança:

- **Observar antes de agir.** O padrão é `modo = "observar"`: o journal (👀), as métricas (`_observado`) e o Grafana mostram o que ele **faria**, e nada é gravado. Passar para `"agir"` é decisão do dono, depois de ver o modo observar rodando.
- **Tudo é temporário e reversível.** Os originais ficam em `/var/lib/termostato/segurados.json`. Ao parar e ao subir depois de um crash, tudo volta.
- **Nunca toca** a raiz, o `init.scope`, o próprio termostato, o SSH, os serviços do systemd, o Tailscale, o Docker e o containerd, o monitoramento e o ryuk do testcontainers (a lista `nunca` é configurável).
- **Nunca mata processo.** Matar é o último recurso do kernel (OOM) e do `systemd-oomd`; as mãos agem antes, segurando.

## Os avisos ([`avisos.rs`](../src/avisos.rs))

Cada coisa que o termostato faz (freio acionado e desligado, freio antecipado, processo no histórico negativo, PL1 regravado por fora, TCC regravado, aviso do kernel, mão que segura ou devolve, e o que faria no modo observar) vira:

- uma **anotação no Grafana**: uma marca laranja no painel Termostato, com o texto do journal e tags (`termostato` e o tipo). Vai por `POST /api/annotations`, com o token de uma conta de serviço (`termostato`, papel Editor) em `/etc/termostato/grafana.token`, modo 600. O envio fica numa thread com fila: um Grafana fora do ar nunca atrasa o freio;
- um **contador**, `termostato_acoes_total{acao}`, que o Prometheus guarda mesmo com o Grafana fora;
- um **e-mail**, pelo alerta "O termostato segurou um processo ou contêiner", quando as mãos agem de verdade.

## O kernel ([`kernel.rs`](../src/kernel.rs), [`gatilhos.rs`](../src/gatilhos.rs))

### O gatilho de pressão

Em vez de olhar a pressão a cada 2 s, o termostato se inscreve no kernel: escreve `some 150000 1000000\0` no `/proc/pressure/memory` ("me avise quando, em qualquer janela de 1 s, algum processo ficar 150 ms parado esperando memória") e espera num `poll`. A thread fica parada, sem gastar CPU, até o kernel chamar, e então o laço principal roda na hora.

O `\0` no fim é obrigatório: o kernel trata o último byte recebido como fim de texto, e sem o terminador ele come o último dígito (`1000000` vira `100000`) e recusa com `EINVAL`. Foi o que aconteceu na primeira instalação.

Testado na fedora com 6 GB de memória pedidos em 3 s: `⚡ o kernel avisou: a memória está apertando`.

### Estrangulamento, domínios e sensores

- `termostato_estrangulamento_total{onde}`: quantas vezes o chip se estrangulou por calor desde o boot, pelos contadores do kernel. Com o termostato bem ajustado, eles param de subir.
- `termostato_dominio_watts{dominio}`: a potência dos núcleos (`core`), do gráfico e do resto do chip (`uncore`) e da memória (`dram`).
- `termostato_sensor_celsius{sensor}`: os sensores do `applesmc`, pela chave do SMC (`TC0P` CPU, `TG0P` GPU, `Ts0P` apoio de mãos, `TH0*` aletas, `TB*T` bateria).

## O chip: o TCC offset ([`tcc.rs`](../src/tcc.rs))

O `MSR 0x1A2` (`MSR_TEMPERATURE_TARGET`) guarda o TjMax (100 °C) e o **TCC offset**: o circuito térmico da própria CPU passa a reduzir o clock `offset` graus antes do TjMax. Com 8, **o chip se freia sozinho a 92 °C**, sem software no meio e em microssegundos. Vale mesmo se o termostato travar ou o sistema sufocar, como em 19/11/2026.

| Camada | Age em | Depende de software? |
|---|---|---|
| Curva de freio com previsão | 80–92 °C, antes do pico | sim (termostato) |
| **TCC offset** | **92 °C, no chip** | **não**, depois de gravado |
| Corte de fábrica | 100 °C | não |

**Por que pelo driver `msr`, e não pelo driver de TCC.** O kernel tem um driver próprio para isso (`intel_tcc_cooling`), mas ele não atende o Haswell: o modelo não está na lista dele (`modprobe` responde `No such device`). Forçá-lo exigiria recompilar o módulo com o Haswell na lista, e o módulo quebraria a cada atualização de kernel do Fedora. O driver genérico `msr` do kernel faz a mesma gravação, e as conferências que o driver de TCC faria ficam com o termostato:

- só grava se o `MSR_PLATFORM_INFO` (0xCE, bit 30) diz que o offset é programável (na fedora: sim);
- lê antes e mexe **só nos 6 bits** do offset (24–29), mantendo o resto do registro;
- recusa offsets acima de 15 °C;
- lê de novo, em cada CPU, para confirmar.

O chip zera o offset a cada boot, então o termostato o grava ao subir (cedo, antes dos pesados) e o confere a cada 30 s. O `--desinstalar` o devolve a 0.

O kernel registra cada gravação direta em MSR (`msr: Write to unrecognized MSR 0x1a2 by termostato … tainting CPU_OUT_OF_SPEC`) e marca a sessão como "tainted". É um rótulo para relatórios de bug ao kernel, sem efeito no funcionamento.

Conferido na fedora: `MSR 0x1A2 = 0x8641000` nas 8 CPUs, ou seja, TjMax 100 °C, offset 8 °C, corte em 92 °C.

## Subir cedo no boot

O firmware volta o PL1 aos 100 W a cada inicialização. Em 19/11/2026, depois de um reinício, a CPU tocou 100 °C nos primeiros segundos, antes de o termostato assumir: ele subia aos 8,8 s, depois do `tuned`, e a VM e o k3s já estavam subindo.

Desde a 0.5, o serviço sobe **antes do `sysinit.target`**, logo depois de o udev carregar os drivers (o `applesmc` sai aos ~6 s), e antes do Docker, do containerd, do k3s e da VM. Se ele chegar antes do sensor ou das ventoinhas, espera por eles (até 30 s pelo sensor; as ventoinhas são procuradas de novo a cada leitura). Com `Type=simple`, essa espera nunca atrasa o boot.

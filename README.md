# termostato

Um serviço em Rust que cuida de todo o calor da **fedora** (`macpro-i7-alex`), um MacBookPro11,2 de 2014 com i7-4770HQ rodando Fedora.

**A CPU nunca fica solta.** No dia a dia ela trabalha sob o **limite normal** de potência. Quando a temperatura sobe, ou quando a **previsão** mostra que vai subir, uma **curva de freio** aperta o limite antes do pico. As ventoinhas acompanham a temperatura (quietas com a CPU fria, no máximo antes do freio), e tudo aparece no Grafana. A arrancada curta (o turbo até 3,4 GHz) continua livre: é ela que faz desse processador um foguetinho.

Desde a 0.5, ele tem **superpoderes**: vê os processos e aprende quem costuma esquentar a máquina, freando **antes** quando eles aparecem; sente a pressão de memória (com o próprio kernel o acordando); segura no cgroup quem abusa (por padrão, só observando); grava no chip uma proteção que faz a **própria CPU se frear a 92 °C**; e avisa o Grafana de tudo o que faz. Ver [os superpoderes](docs/superpoderes.md).

Desde a 0.8, tem **painel próprio** em `/`, feito em RavenRS e desenhado pela GPU do navegador (WebGL2): números ao vivo, o fluxo de cada decisão, gráficos, o histórico dos mal-educados e os ajustes com um clique. Na fedora: http://macpro-i7-alex:9101. Ver [o painel](docs/painel.md).

Desde a 0.7, **recebe parâmetros na hora**, sem reiniciar: `sudo termostato CPU50C` segura a CPU em 50 °C, e `sudo termostato chave=valor` muda qualquer ajuste (`pl1_normal_w=30`, `maos.modo=agir`…). Ver [ajustar na hora](docs/operacao.md#ajustar-na-hora-termostato-chavevalor).

> ⚠️ **Testado num processador só.** O termostato foi feito e testado em **uma máquina**: um MacBookPro11,2 (Retina 15", 2014) com **Intel Core i7-4770HQ (Haswell)** com **Fedora 44 e 45**. Várias partes dependem desse hardware: as ventoinhas pelo `applesmc` (só Macs), o limite de potência pelo RAPL (Intel), o **TCC offset gravado direto no `MSR 0x1A2`** (o chip precisa dizer que é programável, e a gravação marca o kernel como "tainted") e os valores de fábrica que o instalador semeia (PL1 de 100 W, do firmware da Apple). Em outro processador ou outro Mac, revise o [`termostato.toml`](termostato.toml) antes, comece com `tcc_offset_c = 0` e as mãos em `"observar"`, e acompanhe o journal. Use por sua conta e risco.

Não é preciso mexer no sistema operacional: tudo se ajusta num arquivo, o `/etc/termostato.toml`.

## Por que existe

- **O firmware da Apple deixa o limite de potência sustentada (RAPL PL1) em 100 W**, mais que o dobro dos 47 W de projeto do i7-4770HQ. No macOS, o próprio sistema segurava a CPU; no Linux, ninguém. Compilando, ela chegava a 99–100 °C, o ponto em que se desliga.
- **O chassi só dissipa ~25 W em carga contínua com os 8 threads.** Medido em 19/11/2026: de 25 a 45 W de limite, o desempenho sustentado é o mesmo, e os picos de 99–100 °C acontecem na arrancada. Ver [a máquina](docs/maquina.md).

## Como funciona, em resumo

A cada 2 s:

1. **Lê** a temperatura do pacote da CPU.
2. **Prevê** onde ela estará daqui a 10 s, pela reta das últimas 5 leituras.
3. **Aplica a curva de freio** à maior entre a temperatura de agora e a prevista: 38 W até 80 °C, descendo em linha reta até 25 W em 92 °C.
4. **Freia na hora e solta devagar:** o limite cai imediatamente e volta a subir no máximo 0,5 W/s.
5. **Garante o limite e as ventoinhas no hardware**, regravando se outro programa os mudou.
6. **Publica** as métricas para o Prometheus.

O termostato grava nos drivers do kernel (`intel_rapl`, `applesmc`), que escrevem no registro de limite de potência da CPU (MSR 0x610) e no controlador de ventoinhas da Apple (SMC). O valor foi conferido lido direto do chip. Os detalhes estão em [como funciona](docs/como-funciona.md).

Ao parar ou cair, ele deixa o limite normal, nunca os 100 W de fábrica; o systemd o sobe de volta em 3 s.

## O padrão do Fedora, e o único desvio

Fora o que o termostato faz em tempo de execução, a fedora fica no padrão do Fedora, e quem garante isso é o instalador: o perfil de energia fica em **balanced**. O único desvio é o **`thermald` desligado**, porque ele disputava o mesmo limite de potência; o `desinstalar.sh` o religa. Nenhum ajuste de CPU, ventoinha ou temperatura deve ser feito à mão no sistema: se precisar, ele entra neste projeto.

## Começo rápido

O termostato é instalado por um **pacote RPM** (Fedora 45, x86_64), anexado a cada [Release](https://github.com/alexkads/termostato/releases):

```bash
sudo dnf install https://github.com/alexkads/termostato/releases/download/v0.8.0/termostato-0.8.0-1.fc45.x86_64.rpm
```

Na primeira instalação, o pacote liga o serviço (no `sysinit`, antes do Docker, do k3s e da VM), desliga o `thermald` e volta o perfil de energia ao `balanced`. Numa atualização, só reinicia o serviço; o `/etc/termostato.toml` ajustado nunca é trocado. Remover (`sudo dnf remove termostato`) devolve a fábrica: PL1 100 W, TCC offset 0, pisos das ventoinhas, e religa o `thermald`.

Na fedora, o `instalar.sh` faz tudo de uma vez a partir do código (gera o RPM, instala e liga o Prometheus e o Grafana desta máquina), e o `desinstalar.sh` desfaz:

```bash
rsync -a --delete --exclude target ~/Projects/termostato/ fedora:oficina/repos/termostato/
```

```bash
ssh fedora 'cd ~/oficina/repos/termostato && ./instalar.sh'
```

- **Painel:** http://macpro-i7-alex:3000/d/termostato (pela tailnet)
- **Ao vivo:** `ssh fedora 'journalctl -u termostato -f'`
- **Ajustar:** `ssh fedora 'sudoedit /etc/termostato.toml && sudo systemctl restart termostato'`

## A configuração

O arquivo comentado está em [`termostato.toml`](termostato.toml). Os principais campos:

| Campo | Padrão | O que é |
|---|---|---|
| `pl1_normal_w` | 38 | O limite normal de potência sustentada |
| `pl1_freio_w` | 25 | O fim da curva: o limite com o freio no máximo |
| `freio_comeca_c` / `freio_acima_c` | 80 / 92 | Onde a curva começa e onde chega ao máximo |
| `previsao_s` / `janela_leituras` | 10 / 5 | Quanto à frente a previsão olha, e com quantas leituras |
| `subida_w_por_s` | 0,5 | Quão rápido o limite volta ao normal |
| `ventoinhas` | `"curva"` | `"curva"` (mínimo até 60 °C, máximo a partir de 80 °C), `"maximo"` ou `"automatico"` |

A tabela completa, as receitas ("quero menos calor", "quero frear mais cedo") e as mensagens do journal estão em [operação](docs/operacao.md).

## Documentação

| Documento | Para quê |
|---|---|
| [Os superpoderes](docs/superpoderes.md) | Processos, histórico negativo, freio antecipado, memória, mãos, avisos ao Grafana, o kernel e o chip |
| [O painel](docs/painel.md) | A interface em `/`, a API e como mudar o painel |
| [Como funciona](docs/como-funciona.md) | O laço, a previsão, a curva, o caminho até o registro da CPU, a parada segura, os testes |
| [Operação](docs/operacao.md) | Instalar, atualizar, ajustar, acompanhar, métricas, alertas, problemas conhecidos |
| [A máquina](docs/maquina.md) | O hardware, os sensores, as medições e o teste com a build do VintageLightbox |
| [Decisões](docs/decisoes.md) | Cada escolha, com o contexto e o porquê |
| [História](docs/historia.md) | Como o projeto nasceu, do primeiro susto à curva com previsão |
| [Histórico de versões](CHANGELOG.md) | O que mudou em cada versão |

## Desenvolver

```bash
cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test
```

Rust 1.100.0 (Edition 2024), fixado só neste projeto em [`rust-toolchain.toml`](rust-toolchain.toml). Os testes montam um sysfs de mentira numa pasta temporária e rodam em qualquer máquina, sem root. A compilação e a instalação de verdade são feitas na fedora, onde está o hardware.

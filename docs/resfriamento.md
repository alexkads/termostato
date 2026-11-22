# O sistema de resfriamento do MacBookPro11,2

O que a Apple, a Intel e o iFixit dizem do resfriamento deste modelo, o MacBook Pro (Retina, 15", Mid 2014, A1398, só Iris Pro). Ao lado de cada dado está o que as medições da fedora mostraram. Fontes lidas em 22/11/2026.

## O conjunto

```
  entrada de ar                                                  entrada de ar
  (lateral esq.)                                                 (lateral dir.)
        │                                                              │
   ┌────▼─────┐      ┌───────────────────────────────┐      ┌─────────▼┐
   │ventoinha │      │ placa de cobre sobre o chip   │      │ventoinha │
   │esquerda  │══════│ (CPU + eDRAM, uma peça só com │══════│direita   │
   │ + aletas │ heat │  os heat pipes e as aletas)   │ heat │ + aletas │
   └────┬─────┘ pipe └───────────────────────────────┘ pipe └────┬─────┘
        └──────────────► saída de ar pela dobradiça ◄────────────┘
```

- **Duas ventoinhas, com as pás em espaçamento assimétrico:** o barulho se espalha por várias frequências e incomoda menos. O ar entra pelas laterais. O "dissipador e saída de ar" é uma peça só, para a CPU e o gráfico. [iFixit, desmonte do Mid 2012](https://www.ifixit.com/Teardown/MacBook+Pro+15-Inch+Retina+Display+Mid+2012+Teardown/9462), o mesmo projeto de chassi de 2012 a 2015.
- **A saída de ar pela dobradiça tem um estreitamento antes das grelhas,** que acelera o ar e o empurra para fora mais rápido ([iFixit, mesmo desmonte](https://www.ifixit.com/Teardown/MacBook+Pro+15-Inch+Retina+Display+Mid+2012+Teardown/9462)). É onde pôr a mão no teste da issue #23.
- **O dissipador desta versão é diferente do da versão com GeForce.** A peça de reposição é a "MacBook Pro 15" Retina (Late 2013/Mid 2014, **Integrated Graphics**) Heat Sink". Ela é presa por **6 parafusos Torx T5**: um prateado de cabeça redonda de 5,5 mm, quatro de 3,5 mm e um prateado de 3,8 mm. A versão com GeForce leva mais parafusos ([iFixit, troca do dissipador do Mid 2014](https://www.ifixit.com/Guide/MacBook+Pro+15-Inch+Retina+Display+Mid+2014+Heat+Sink+Replacement/27659)).
- **As ventoinhas têm guias de troca próprios.** A esquerda sai com 3 parafusos. A direita exige tirar antes a placa AirPort ([esquerda](https://www.ifixit.com/Guide/MacBook+Pro+15-Inch+Retina+Display+Mid+2014+Left+Fan+Replacement/27661), [direita](https://www.ifixit.com/Guide/MacBook+Pro+15-Inch+Retina+Display+Mid+2014+Right+Fan+Replacement/27690)).
- **Medido na fedora:** as ventoinhas vão a **6.156 e 5.700 RPM** no máximo (`applesmc`, `fan*_max`).

## O processador

Pela ficha da Intel ([Intel ARK, i7-4770HQ](https://www.intel.com/content/www/us/en/products/sku/83505/intel-core-i74770hq-processor-6m-cache-up-to-3-40-ghz/specifications.html)):

| Dado | Intel | O que vimos na fedora |
|---|---|---|
| Família | Crystal Well, 22 nm, lançado no 3º tri de 2014 | — |
| Base / turbo | 2,2 GHz / 3,4 GHz | ~3,19 GHz com os 4 núcleos (`termostato estresse`) |
| **TDP** | **47 W** | O firmware da Apple deixa o PL1 em **100 W**: mais do dobro do projeto da Intel. O termostato o põe em 38 W |
| **TJUNCTION** | **100 °C** | O `MSR 0x1A2` diz o mesmo TjMax, e o chip se freia ali. O TCC offset de 8 °C não segura |
| Pacote | FCBGA1364, 37,5 × 32 mm | Duas pastilhas sob o dissipador: a CPU e a eDRAM de 128 MB |
| Monitoramento térmico | Sim | Os motivos de freio no `MSR 0x690`: térmico, PL1, PL2 e corrente; PROCHOT nunca |

A folha de dados completa, com o controle térmico (TCC, PROCHOT#), é a [Mobile 4th Generation Intel Core M and H Processor Lines, Datasheet Volume 1](https://intel.com/content/www/us/en/content-details/328901/mobile-4th-generation-intel-core-m-and-h-processor-lines-mobile-pentium-and-mobile-celeron-processor-families-datasheet-volume-1.html). O i7-4770HQ é da linha H. Ainda não foi lida a fundo.

## O que a Apple especifica

Pelas [especificações técnicas](https://support.apple.com/en-us/111935):

| Dado | Apple | O que vimos na fedora |
|---|---|---|
| Processador | i7 quad-core de 2,2 GHz (Turbo até 3,4 GHz), Intel Iris Pro | O mesmo (MacBookPro11,2) |
| **Carregador** | **MagSafe 2 de 85 W** (a Apple também vende um de 60 W) | A bateria descarrega com o carregador ligado a partir de ~17 W de CPU. **O da fedora não é o de 85 W:** o original foi danificado pelo técnico e trocado por um mais fraco. Com a CPU acima de 30 W, a bateria descarrega em 71 % das leituras |
| Bateria | 95 Wh, lítio-polímero | 127 ciclos, 90 % da carga de fábrica |
| **Temperatura do ambiente para uso** | **10 a 35 °C** | A fedora roda 24 h como servidor: num quarto quente, o limite chega antes |
| Umidade | 0 a 90 %, sem condensação | — |
| Altitude de uso | testada até ~3.000 m (10.000 pés) | — |

E pelos artigos de suporte:

- **[Manter o notebook dentro da temperatura de uso](https://support.apple.com/en-us/102336):**
  - é normal o notebook esquentar, e os sensores ligam as ventoinhas sozinhos;
  - a faixa do ambiente é de 10 a 35 °C;
  - os aplicativos de temperatura leem os sensores internos, e não a carcaça, que fica bem mais fria;
  - nos Macs Intel, a Apple sugere zerar o SMC quando as ventoinhas ficam altas sem motivo.
- **[As ventoinhas e o barulho delas](https://support.apple.com/en-us/101576):**
  - as ventoinhas aceleram em tarefas pesadas;
  - com o ambiente quente, começam antes e giram mais rápido;
  - saídas de ar tapadas ou superfícies macias (cama, colo) fazem a ventoinha trabalhar mais.

## Como isso se encaixa no que medimos

- **A Apple projetou o resfriamento para os 47 W da Intel.** O firmware, porém, deixa o chip pedir até 100 W no PL1. No macOS, a Apple segura o chip com o gerenciamento dela. No Linux, isso não existe: é o papel do termostato.
- **O chip a 100 °C é o limite de projeto da Intel (TJUNCTION),** e não um defeito por si só. O que destoa na fedora é o **dissipador e o ar de saída frios** com o chip nesse limite (ver [a máquina](maquina.md#o-heat-pipe-o-calor-não-chega-às-aletas)). Junto com o pino de fixação ressoldado, isso aponta para a pressão de montagem (issue [#23](https://github.com/alexkads/termostato/issues/23)).
- **O carregador de 85 W é o certo para este modelo.** Com ele, a bateria não deveria descarregar com a CPU em ~17–40 W. Se o carregador ligado for o de 60 W, isso explica a descarga.

## Outras leituras (não conferidas a fundo)

- [Notebookcheck, ficha do Mid 2014](https://www.notebookcheck.net/Apple-MacBook-Pro-Retina-15-inch-Mid-2014.124945.0.html) e [EveryMac, i7 2,2 GHz Iris](https://everymac.com/systems/apple/macbook_pro/specs/macbook-pro-core-i7-2.2-15-iris-only-mid-2014-retina-display-specs.html): fichas de terceiros.
- [Marco Arment, 2012](https://marco.org/2012/06/14/retina-macbook-pro-heat-fan-noise): as pás assimétricas mudam o tom do barulho, não o calor.
- [MacRumors, repasta num Mid 2014](https://forums.macrumors.com/threads/cannot-get-a-good-repaste-on-2014-15-macbook-pro.2437146/) e [MacRumors, a hipótese do heat pipe](https://forums.macrumors.com/threads/hypothesis-re-high-temp-overheating-on-late-2013-macbook-pro-retina-due-to-high-vapor-point-in-heatpipe.2238074/): relatos de donos, resumidos em [a máquina](maquina.md#o-heat-pipe-o-calor-não-chega-às-aletas).

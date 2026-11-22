# O painel

O termostato serve o próprio painel em `/`, na porta das métricas. Na fedora, pela tailnet: **http://macpro-i7-alex:9101**.

O painel foi desenhado com o design system do RCB (o RavenRS, gerado pelo RavenComponentBook) e é feito em RavenRS, desenhado pela GPU do navegador em **WebGL2**. O desenho aprovado está em [Termostato · painel](https://claude.ai/artifact/WZ8myAZ9MhV63rVUQxrSiR).

## O que ele mostra

| Parte | O que é | De onde vem |
|---|---|---|
| Cabeçalho | Versão, máquina, quem está freando e a troca de tema (Escuro, Claro, Vidro) | `decisao.vence` |
| Quatro números | CPU agora, prevista em 10 s, limite PL1 e potência, cada um com a tendência dos últimos 4 min | `agora`, `serie` |
| Temperatura | Os últimos 10 min, com o alvo (tracejado) e o corte do chip (TCC) | `serie`, `ajustes.alvo_c`, `agora.tcc_corte` |
| Ajustes | Alvo (sem, 55, 60, 65, 70 °C), mãos (observar, agir, desligado), limite normal (25–45 W) e voltar ao padrão | `POST /api/ajustes` |
| Como ele decidiu o limite | O fluxo de cada volta (FlowCanvas só leitura): leituras, regras e ações; o caminho que venceu fica em execução, o que não contou fica desligado | `decisao` |
| Potência e limite, Ventoinhas, Quem gasta a CPU | Linha, radial (a rotação pela curva) e rosca | `serie`, `ventoinhas`, `processos` |
| Cada núcleo | Uma barra por núcleo físico (em alerta a partir de 90 °C), com os estrangulamentos dele, e a diferença entre o mais quente e o mais frio (desiguais a partir de 8 °C) | `nucleos` |
| Por que o chip freia | Os motivos do `MSR 0x690` (agora, desde o boot, leituras) e o resumo do PROCHOT (vezes e segundos com a CPU freada por fora) | `motivos`, `prochot` |
| Bateria e carregador | O carregador ligado ou não, a corrente com sinal e a tensão, a carga e o que a bateria já deu com o carregador ligado (Wh, minutos, vezes) | `bateria` |
| Freando por hora | Minutos com o PL1 abaixo do normal, nas últimas 12 h | `freio_por_hora` |
| Sensores do Mac | Os seis mais quentes do `applesmc`, com o nome em português quando o código é conhecido | `sensores` |
| Processos e O que ele fez | Quem gasta a CPU agora, com a nota de calor, e os últimos avisos do journal | `processos`, `feitos` |
| Histórico dos mal-educados | A nota de calor dos 8 mais quentes, com a linha de suspeito, e a tabela com a maior subida, as vezes visto e quando | `historico` |
| Barra de status | A versão, se leu agora, o TCC, a pressão de memória e as ventoinhas | |

A série, o freio por hora e "o que ele fez" ficam em memória: começam vazios a cada reinício do serviço. O histórico longo é o do Grafana.

## Ajustar pelo painel

Os ajustes do painel são os mesmos do `sudo termostato chave=valor` ([operação](operacao.md#ajustar-na-hora-termostato-chavevalor)): conferidos pelas mesmas regras e gravados no mesmo `/var/lib/termostato/ajustes.toml`. Cada um aparece no journal como `🎛️ painel (<login>): …`.

Só ajusta quem chega **pela tailnet**, com um login listado em:

```toml
[painel]
quem_pode_ajustar = ["login@da.tailnet"]
```

O `instalar.sh` põe ali o dono da máquina na tailnet e publica o painel com `tailscale serve --bg --http=9101 http://127.0.0.1:9101`. Quem abre sem estar na lista vê o painel com o selo "Só leitura". Esta seção só muda no toml, reiniciando: a linha de comando e a API recusam `painel.*`.

## A API

| Pedido | Resposta |
|---|---|
| `GET /api/estado` | Tudo o que o painel desenha (ver `painel/exemplo.json`) |
| `POST /api/ajustes` com `{"ajustes": {"alvo_c": 65, "maos.modo": "agir"}}` | `200 {"mudou": […]}` |
| `POST /api/ajustes` com `{"ajustes": {"alvo_c": null}}` | tira um ajuste (volta ao toml) |
| `POST /api/ajustes` com `{"padrao": true}` | tira todos |
| Sem `Tailscale-User-Login` da lista, ou de outra página | `403` |
| Corpo que não é JSON | `415` |
| Ajuste fora das regras | `400 {"erro": "…"}`, nada gravado |

## Mudar o painel

O código está em `painel/` (uma crate RavenRS à parte). Depois de mudar:

```bash
painel/compilar.sh
```

Ele compila para `wasm32-unknown-unknown`, passa pelo `wasm-bindgen` 0.2.130 (igual ao do RavenRS) e pelo `wasm-opt`, e deixa `index.html`, `painel.js` e `painel_bg.wasm` em `painel/dist/`, que o termostato embute. Commit o `dist/` junto: o RPM não compila wasm.

Para ver sem a fedora, sirva `painel/dist/` com o `exemplo.json` em `api/estado`:

```bash
mkdir -p /tmp/painel/api && cp painel/dist/* /tmp/painel/ && cp painel/exemplo.json /tmp/painel/api/estado && python3 -m http.server -d /tmp/painel 8765
```

Os testes do painel (`cd painel && cargo test`) desenham a página inteira sem janela, nos temas Escuro e Claro, com o exemplo e sem estado.

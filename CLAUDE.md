# termostato

## 📋 Toda mudança tem issue e PR

> O dono, 20/11/2026: *"Deixe documentado que o padrão é sempre criar issue e PR para esse projeto."*

Nada vai direto para o `main`. Toda mudança, por menor que seja (código, configuração, documentação, versão), segue este caminho:

1. **Issue primeiro**, em `alexkads/termostato`, antes de commitar. Se o trabalho já começou sem ela, crie-a assim que perceber, antes do PR.
2. **Branch** `claude/<assunto-curto>` a partir do `main` (ex.: `claude/ventoinhas-curva`).
3. **Commits** no padrão do Alex, com o tempo real no início (skill `commit-com-tempo`).
4. **PR** para o `main`, pronto para revisão, com `Closes #<issue>` na primeira linha. Um PR empilhado sobre outro diz isso logo abaixo.
5. Juntar o PR é decisão do dono.

### A issue

Quem pegar a issue precisa conseguir trabalhar **sem perguntar nada**. O modelo é a #12. Ela leva, nesta ordem:

- **História:** "como dono da fedora, quero <ação>, para <benefício>".
- **Contexto:** o que existe hoje, com os arquivos e as funções reais (`src/main.rs`, `servir`…). Confira no código antes de citar.
- **O que fazer:** passos numerados, incluindo a configuração (`termostato.toml`, `config.rs` com a validação), os testes, o `instalar.sh` se mudar, e a documentação (README, `docs/`, CHANGELOG, decisão nova em `docs/decisoes.md`).
- **Telas:** o link do Artifact com o nome da prancheta. Sem tela, diga isso.
- **Dependências:** outras issues e PRs, com link.
- **Referências:** arquivos e docs do repositório.
- **Critérios de aceite:** verificáveis na fedora, no formato entrada → resultado (o que o journal mostra, que métrica muda), incluindo o caso de recusa e o `cargo xtask pr` limpo.
- **Label:** `enhancement` para recurso novo e `bug` para defeito.

### O PR

O modelo é o #13. A descrição leva:

- `Closes #<issue>` (e "Empilhado sobre o PR #…", se for o caso).
- **Por quê** e **O que entra**, em bullets.
- **Para quem revisa:** o que não é óbvio, como mudanças de padrão que não chegam ao `/etc/termostato.toml` já instalado.
- **Provado na fedora (<data>):** o que foi visto de verdade no journal, nas métricas ou no painel depois do `instalar.sh`.
- **Testes:** quantos e o que cobrem, e a tabela de tempos do `cargo xtask pr`.

## Desenvolver

- Enquanto trabalha, `cargo xtask dev`. Antes do PR, `cargo xtask pr`, e antes de publicar uma versão, `cargo xtask release`. Nenhum deles recebe argumentos. Depois de mudar o painel, `cargo xtask painel`, e o `painel/dist/` vai no mesmo commit. Não se cria `.sh` novo.
- As checagens rodam na fedora, sob a trava da oficina (veja o README), e não no Mac do Alex. Os testes não pedem root.
- A instalação de verdade é na fedora (`macpro-i7-alex`), pelo `rsync` e o `./instalar.sh` do README.
- O RPM nunca troca o `/etc/termostato.toml` instalado. Um padrão novo precisa ser levado à mão, ou pelo `sudo termostato chave=valor`, e o PR diz isso.

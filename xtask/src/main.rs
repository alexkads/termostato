//! `cargo xtask <tarefa>`: as checagens e o build do painel do termostato, um
//! subcomando por finalidade (#28, decisão 27 de `docs/decisoes.md`).
//!
//! É Rust, então roda igual no macOS, no Linux e no Windows. Usa só a biblioteca
//! padrão, então compila em segundos antes de qualquer checagem.

mod cache;
mod checagens;
mod painel;
mod run;
mod selecao;

use std::path::{Path, PathBuf};

const AJUDA: &str = "\
cargo xtask <tarefa>: as checagens do termostato, uma por finalidade.

  dev       enquanto se trabalha, antes do commit (minutos): formata, confere o
            painel/dist/ e passa o clippy e os testes só no que a mudança toca
  pr        o gate do PR: formatação conferida, clippy e testes em tudo o que a
            mudança alcança, o build de release e o painel para wasm32 quando
            alcançados
  release   antes de publicar: as três unidades inteiras, o painel para o
            navegador, a documentação e o cargo deny
  painel    compila o painel para o navegador e regrava o painel/dist/

As tarefas não recebem argumentos: cada uma faz uma coisa. A comparação é com
origin/main, ou com CHECK_BASE. Os testes rodam no cargo-nextest.
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(despachar(&args));
}

type Tarefa = fn(&mut run::Checagem) -> Result<String, run::Falha>;

fn despachar(args: &[String]) -> i32 {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map_or_default(PathBuf::from);
    let (nome, resto) = match args.split_first() {
        Some((nome, resto)) => (nome.as_str(), resto),
        None => ("ajuda", &[][..]),
    };
    let tarefa: Tarefa = match nome {
        "dev" => checagens::dev,
        "pr" => checagens::pr,
        "release" => checagens::release,
        "painel" => painel::compilar,
        "ajuda" | "help" | "--help" | "-h" => {
            print!("{AJUDA}");
            return 0;
        }
        outro => {
            eprintln!("erro: não há tarefa chamada {outro}\n\n{AJUDA}");
            return 2;
        }
    };
    if let Some(recusa) = recusar_argumentos(nome, resto) {
        eprintln!("{recusa}");
        return 2;
    }
    let mut c = run::Checagem::nova(nome, &raiz);
    if nome != "painel"
        && let Err(f) = nextest_instalado(&c)
    {
        eprintln!("erro: {f}");
        return 1;
    }
    let cache = cache::Cache::ligar(&mut c);
    let feito = tarefa(&mut c);
    c.terminar(feito, &cache)
}

/// A recusa de uma tarefa chamada com argumentos, ou nada.
fn recusar_argumentos(nome: &str, resto: &[String]) -> Option<String> {
    if resto.is_empty() {
        return None;
    }
    let qual = if resto.iter().any(|a| a == "--full" || a == "--tudo") {
        "Para conferir tudo, use cargo xtask release."
    } else {
        "Cada tarefa faz uma coisa: dev enquanto se trabalha, pr para o PR, release antes de publicar."
    };
    Some(format!(
        "erro: cargo xtask {nome} não recebe argumentos, e recebeu: {}\n{qual}",
        resto.join(" ")
    ))
}

/// Os testes rodam no cargo-nextest, que se instala uma vez.
fn nextest_instalado(c: &run::Checagem) -> run::Feito {
    c.saida("cargo", &["nextest", "--version"]).map(drop).map_err(|_| {
        run::falha("as checagens rodam os testes no cargo-nextest; instale uma vez com: cargo install cargo-nextest --locked")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uma_checagem_com_argumentos_e_recusada_dizendo_qual_usar() {
        assert_eq!(recusar_argumentos("dev", &[]), None);
        let full = recusar_argumentos("pr", &["--full".to_owned()]).unwrap();
        assert!(
            full.contains("não recebe argumentos, e recebeu: --full"),
            "{full}"
        );
        assert!(full.contains("cargo xtask release"), "{full}");
        let outro = recusar_argumentos("dev", &["-p".to_owned(), "painel".to_owned()]).unwrap();
        assert!(outro.contains("pr para o PR"), "{outro}");
    }
}

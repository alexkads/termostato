//! As três camadas. O que sai do `dev` continua no `pr`, e o que sai do `pr`
//! continua no `release`.
//!
//! - `dev`: enquanto se trabalha. Formata, confere o `dist/` e passa o clippy e
//!   os testes só nas unidades tocadas.
//! - `pr`: o gate do PR. Formatação conferida, e clippy e testes em tudo o que a
//!   mudança alcança, mais o build de release (o do RPM) e o painel para wasm32
//!   quando são alcançados.
//! - `release`: as três unidades inteiras, o build do painel para o navegador,
//!   a documentação e o `cargo deny`.

use std::collections::BTreeSet;

use crate::painel;
use crate::run::{self, Checagem, Falha, Feito, falha};
use crate::selecao::{self, Mudanca, TODAS, Unidade};

const PAINEL: &str = "painel/Cargo.toml";

/// O que se confere: as unidades e se o `dist/` está atrasado.
struct Alcance {
    unidades: BTreeSet<Unidade>,
    dist_atrasado: bool,
}

/// O que a mudança toca (no `dev`) ou alcança (no `pr`), e o que se conferiu.
fn alcance(c: &mut Checagem, alcancadas: bool) -> Alcance {
    c.etapa("o que a mudança alcança");
    match Mudanca::ler(c) {
        Ok(m) => {
            let tocadas = selecao::tocadas(m.nomes());
            let unidades = if alcancadas {
                selecao::alcancadas(&tocadas)
            } else {
                tocadas.clone()
            };
            println!("base: {}; {} arquivos mudados", m.base, m.arquivos.len());
            println!("tocadas: {}", selecao::lista(&tocadas));
            if alcancadas {
                println!("alcançadas: {}", selecao::lista(&unidades));
            }
            Alcance {
                unidades,
                dist_atrasado: selecao::dist_atrasado(m.nomes()),
            }
        }
        Err(motivo) => {
            println!("\x1b[1;33m{motivo}: confiro tudo.\x1b[0m");
            Alcance {
                unidades: TODAS.into_iter().collect(),
                dist_atrasado: false,
            }
        }
    }
}

/// Os pacotes do workspace da raiz entre as unidades, como `-p` do cargo.
fn pacotes_da_raiz(unidades: &BTreeSet<Unidade>) -> Vec<&'static str> {
    let mut args = Vec::new();
    for u in unidades {
        match u {
            Unidade::Termostato => args.extend(["-p", "termostato"]),
            Unidade::Xtask => args.extend(["-p", "xtask"]),
            Unidade::Painel => {}
        }
    }
    args
}

fn com<'a>(base: &[&'a str], meio: &[&'a str], fim: &[&'a str]) -> Vec<&'a str> {
    base.iter().chain(meio).chain(fim).copied().collect()
}

fn formatar(c: &mut Checagem, conferir: bool) -> Feito {
    c.etapa(if conferir {
        "rustfmt --check"
    } else {
        "rustfmt"
    });
    let check: &[&str] = if conferir { &["--check"] } else { &[] };
    c.rodar("cargo", &com(&["fmt", "--all"], check, &[]))?;
    c.rodar(
        "cargo",
        &com(&["fmt", "--manifest-path", PAINEL], check, &[]),
    )
}

fn conferir_dist(c: &mut Checagem, a: &Alcance) -> Feito {
    c.etapa("o painel/dist/ acompanha o painel");
    if a.dist_atrasado {
        return Err(falha(
            "o código do painel mudou e o painel/dist/ não; rode `cargo xtask painel` e commite o painel/dist/",
        ));
    }
    Ok(())
}

fn clippy(c: &mut Checagem, unidades: &BTreeSet<Unidade>) -> Feito {
    let raiz = pacotes_da_raiz(unidades);
    let fim = ["--all-targets", "--locked", "--", "-D", "warnings"];
    if !raiz.is_empty() {
        c.etapa(&format!(
            "clippy: {}",
            selecao::lista(
                &unidades
                    .iter()
                    .filter(|u| **u != Unidade::Painel)
                    .copied()
                    .collect()
            )
        ));
        c.rodar("cargo", &com(&["clippy"], &raiz, &fim))?;
    }
    if unidades.contains(&Unidade::Painel) {
        c.etapa("clippy: painel");
        c.rodar(
            "cargo",
            &com(&["clippy", "--manifest-path", PAINEL], &[], &fim),
        )?;
    }
    Ok(())
}

/// Os testes no nextest, com a compilação numa etapa e a execução na outra,
/// para a tabela dizer onde está o tempo.
fn testes(c: &mut Checagem, unidades: &BTreeSet<Unidade>) -> Feito {
    let raiz = pacotes_da_raiz(unidades);
    let grupos: Vec<(String, Vec<&str>)> = [
        (!raiz.is_empty()).then(|| {
            let nomes = unidades
                .iter()
                .filter(|u| **u != Unidade::Painel)
                .copied()
                .collect();
            (selecao::lista(&nomes), raiz.clone())
        }),
        unidades
            .contains(&Unidade::Painel)
            .then(|| ("painel".to_owned(), vec!["--manifest-path", PAINEL])),
    ]
    .into_iter()
    .flatten()
    .collect();
    for (nome, alvo) in grupos {
        c.etapa(&format!("compilar os testes: {nome}"));
        c.rodar(
            "cargo",
            &com(&["nextest", "run", "--no-run", "--locked"], &alvo, &[]),
        )?;
        c.etapa(&format!("rodar os testes: {nome}"));
        c.rodar(
            "cargo",
            &com(
                &["nextest", "run", "--locked", "--no-tests=warn"],
                &alvo,
                &[],
            ),
        )?;
    }
    Ok(())
}

/// O caminho do RPM: release com LTO.
fn build_release(c: &mut Checagem) -> Feito {
    c.etapa("build de release do termostato (o do RPM)");
    c.rodar(
        "cargo",
        &["build", "--release", "--locked", "-p", "termostato"],
    )
}

/// O que só aparece no wasm32 (o código atrás de `cfg(target_arch = "wasm32")`)
/// e os doctests do painel, que o nextest não roda.
fn painel_alem(c: &mut Checagem) -> Feito {
    c.etapa("clippy do painel para wasm32");
    painel::alvo_instalado(c)?;
    c.rodar(
        "cargo",
        &[
            "clippy",
            "--manifest-path",
            PAINEL,
            "--lib",
            "--target",
            painel::WASM,
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    c.etapa("doctests do painel");
    c.rodar(
        "cargo",
        &["test", "--doc", "--manifest-path", PAINEL, "--locked"],
    )
}

/// Enquanto se trabalha, antes do commit.
pub fn dev(c: &mut Checagem) -> Result<String, Falha> {
    let a = alcance(c, false);
    formatar(c, false)?;
    conferir_dist(c, &a)?;
    clippy(c, &a.unidades)?;
    testes(c, &a.unidades)?;
    Ok(format!(
        "cargo xtask dev passou ({}).",
        selecao::lista(&a.unidades)
    ))
}

/// O gate do PR.
pub fn pr(c: &mut Checagem) -> Result<String, Falha> {
    let a = alcance(c, true);
    formatar(c, true)?;
    conferir_dist(c, &a)?;
    clippy(c, &a.unidades)?;
    testes(c, &a.unidades)?;
    if a.unidades.contains(&Unidade::Termostato) {
        build_release(c)?;
    }
    if a.unidades.contains(&Unidade::Painel) {
        painel_alem(c)?;
    }
    Ok(format!(
        "cargo xtask pr passou ({}).",
        selecao::lista(&a.unidades)
    ))
}

/// Antes de publicar uma versão: tudo.
pub fn release(c: &mut Checagem) -> Result<String, Falha> {
    c.etapa("ferramentas do release");
    c.saida("cargo", &["deny", "--version"])
        .map_err(|_| falha("o release confere as dependências com o cargo-deny; instale uma vez com: cargo install cargo-deny --locked"))?;
    let todas: BTreeSet<Unidade> = TODAS.into_iter().collect();
    let a = alcance(c, true);
    formatar(c, true)?;
    conferir_dist(c, &a)?;
    clippy(c, &todas)?;
    testes(c, &todas)?;
    build_release(c)?;
    painel_alem(c)?;
    c.etapa("painel para o navegador (perfil wasm-release)");
    c.rodar("cargo", &painel::BUILD_WASM)?;

    c.etapa("documentação");
    let mut doc = c.comando(
        "cargo",
        &[
            "doc",
            "--no-deps",
            "--locked",
            "-p",
            "termostato",
            "-p",
            "xtask",
        ],
    );
    doc.env("RUSTDOCFLAGS", "-D warnings");
    run::rodar_comando(doc, "cargo", &["doc", "-p", "termostato", "-p", "xtask"])?;
    let mut doc = c.comando(
        "cargo",
        &["doc", "--no-deps", "--locked", "--manifest-path", PAINEL],
    );
    doc.env("RUSTDOCFLAGS", "-D warnings");
    run::rodar_comando(doc, "cargo", &["doc", "--manifest-path", PAINEL])?;

    c.etapa("cargo deny");
    let config = c.raiz.join("deny.toml");
    let config = config.to_string_lossy();
    c.rodar("cargo", &["deny", "--locked", "check", "--config", &config])?;
    c.rodar(
        "cargo",
        &[
            "deny",
            "--locked",
            "--manifest-path",
            PAINEL,
            "check",
            "--config",
            &config,
        ],
    )?;
    Ok("cargo xtask release passou: termostato, xtask e painel.".to_owned())
}

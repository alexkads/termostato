//! O que a mudança toca e o que ela alcança, para conferir só isso. Tem testes
//! próprios: o que o seletor deixa de fora não é conferido.
//!
//! As unidades são três. O `termostato` e o `xtask` são do workspace da raiz, e
//! o `painel` tem workspace próprio. O painel alcança o termostato, porque o
//! termostato embute o `painel/dist/` com `include_bytes!` (`src/painel.rs`).

use std::collections::BTreeSet;
use std::fmt;

use crate::run::Checagem;

/// Uma parte do repositório que se confere por inteiro.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unidade {
    Termostato,
    Xtask,
    Painel,
}

/// Todas, na ordem em que se conferem.
pub const TODAS: [Unidade; 3] = [Unidade::Termostato, Unidade::Xtask, Unidade::Painel];

impl fmt::Display for Unidade {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Unidade::Termostato => "termostato",
            Unidade::Xtask => "xtask",
            Unidade::Painel => "painel",
        })
    }
}

/// As unidades que um arquivo mudado toca, pelo caminho relativo à raiz.
fn do_arquivo(arquivo: &str) -> &'static [Unidade] {
    use Unidade::{Painel, Termostato, Xtask};
    let arquivo = arquivo.trim_start_matches("./");
    if arquivo == "rust-toolchain.toml" || arquivo.starts_with(".cargo/") {
        &TODAS
    } else if arquivo == "Cargo.toml" || arquivo == "Cargo.lock" {
        // As dependências, os lints e os perfis do workspace da raiz.
        &[Termostato, Xtask]
    } else if arquivo.starts_with("xtask/") {
        &[Xtask]
    } else if arquivo.starts_with("painel/dist/") {
        // O que se embute no binário, e não o código do painel.
        &[Termostato]
    } else if arquivo.starts_with("painel/") {
        &[Painel]
    } else if arquivo.starts_with("src/") {
        &[Termostato]
    } else {
        // Documentação, instalador, Grafana, pacote: nada que compile.
        &[]
    }
}

/// As unidades que os arquivos mudados tocam.
pub fn tocadas<'a>(arquivos: impl IntoIterator<Item = &'a str>) -> BTreeSet<Unidade> {
    arquivos
        .into_iter()
        .flat_map(|a| do_arquivo(a).iter().copied())
        .collect()
}

/// As tocadas e as que dependem delas, em qualquer profundidade (aqui, só o
/// termostato depende de alguém: do painel, pelo `dist/`).
pub fn alcancadas(tocadas: &BTreeSet<Unidade>) -> BTreeSet<Unidade> {
    let mut todas = tocadas.clone();
    if todas.contains(&Unidade::Painel) {
        todas.insert(Unidade::Termostato);
    }
    todas
}

/// Se o código do painel mudou e o `dist/` que o termostato embute não: o
/// binário iria com o painel velho. O `painel/Cargo.toml` fica de fora: perfis e
/// comentários não mudam o wasm, e uma dependência nova muda o `Cargo.lock`.
pub fn dist_atrasado<'a>(arquivos: impl IntoIterator<Item = &'a str> + Clone) -> bool {
    let fonte = [
        "painel/src/",
        "painel/web/",
        "painel/Cargo.lock",
    ];
    let mudou_fonte = arquivos
        .clone()
        .into_iter()
        .any(|a| fonte.iter().any(|f| a.starts_with(f)));
    let mudou_dist = arquivos.into_iter().any(|a| a.starts_with("painel/dist/"));
    mudou_fonte && !mudou_dist
}

/// O que mudou contra a base: os commits desde ela e o que ainda não foi
/// commitado, contando os arquivos novos.
#[derive(Debug)]
pub struct Mudanca {
    /// Os arquivos, relativos à raiz.
    pub arquivos: Vec<String>,
    /// `origin/main` ou o `CHECK_BASE`, e o commit em que o branch saiu dela.
    pub base: String,
}

impl Mudanca {
    /// O que mudou, ou o motivo de não se saber (então se confere tudo).
    pub fn ler(c: &Checagem) -> Result<Mudanca, String> {
        let base = std::env::var("CHECK_BASE").unwrap_or_else(|_| "origin/main".to_owned());
        let ponto = c
            .saida("git", &["merge-base", "HEAD", &base])
            .map_err(|_| format!("não achei a base {base} (CHECK_BASE)"))?;
        let ponto = ponto.trim();
        let mut arquivos = Vec::new();
        for args in [
            &["diff", "--name-only", "--no-renames", ponto][..],
            &["ls-files", "--others", "--exclude-standard"][..],
        ] {
            let saida = c.saida("git", args).map_err(|f| f.to_string())?;
            arquivos.extend(saida.lines().filter(|l| !l.is_empty()).map(str::to_owned));
        }
        arquivos.sort();
        arquivos.dedup();
        let curto = &ponto[..ponto.len().min(9)];
        Ok(Mudanca {
            arquivos,
            base: format!("{base} @ {curto}"),
        })
    }

    /// Os arquivos, para as funções acima.
    pub fn nomes(&self) -> impl Iterator<Item = &str> + Clone {
        self.arquivos.iter().map(String::as_str)
    }
}

/// `termostato, painel`, ou `nenhuma`.
pub fn lista(unidades: &BTreeSet<Unidade>) -> String {
    if unidades.is_empty() {
        return "nenhuma".to_owned();
    }
    unidades
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use Unidade::{Painel, Termostato, Xtask};

    fn conjunto(u: &[Unidade]) -> BTreeSet<Unidade> {
        u.iter().copied().collect()
    }

    #[test]
    fn documentacao_e_instalador_nao_tocam_nada() {
        let mudou = [
            "docs/painel.md",
            "README.md",
            "CHANGELOG.md",
            "instalar.sh",
            "grafana/x.json",
            "pacote/termostato.spec",
            "termostato.toml",
        ];
        assert_eq!(tocadas(mudou), conjunto(&[]));
    }

    #[test]
    fn o_codigo_do_termostato_toca_so_ele() {
        assert_eq!(tocadas(["src/freio.rs"]), conjunto(&[Termostato]));
        assert_eq!(
            alcancadas(&conjunto(&[Termostato])),
            conjunto(&[Termostato])
        );
    }

    #[test]
    fn o_manifesto_da_raiz_toca_o_termostato_e_o_xtask() {
        assert_eq!(tocadas(["Cargo.toml"]), conjunto(&[Termostato, Xtask]));
        assert_eq!(tocadas(["Cargo.lock"]), conjunto(&[Termostato, Xtask]));
    }

    #[test]
    fn o_toolchain_e_o_cargo_tocam_tudo() {
        assert_eq!(tocadas(["rust-toolchain.toml"]), conjunto(&TODAS));
        assert_eq!(tocadas([".cargo/config.toml"]), conjunto(&TODAS));
    }

    #[test]
    fn o_painel_alcanca_o_termostato_que_o_embute() {
        let t = tocadas(["painel/src/lib.rs"]);
        assert_eq!(t, conjunto(&[Painel]));
        assert_eq!(alcancadas(&t), conjunto(&[Termostato, Painel]));
    }

    #[test]
    fn o_dist_toca_o_termostato_e_nao_o_codigo_do_painel() {
        assert_eq!(
            tocadas(["painel/dist/painel_bg.wasm"]),
            conjunto(&[Termostato])
        );
    }

    #[test]
    fn o_xtask_toca_so_ele() {
        assert_eq!(tocadas(["xtask/src/selecao.rs"]), conjunto(&[Xtask]));
        assert_eq!(alcancadas(&conjunto(&[Xtask])), conjunto(&[Xtask]));
    }

    #[test]
    fn o_dist_atrasado_e_o_codigo_do_painel_sem_o_dist() {
        assert!(dist_atrasado(["painel/src/lib.rs"]));
        assert!(dist_atrasado(["painel/web/index.html", "src/painel.rs"]));
        assert!(dist_atrasado(["painel/Cargo.lock"]));
        assert!(!dist_atrasado(["painel/Cargo.toml"]), "perfis não mudam o wasm");
        assert!(!dist_atrasado([
            "painel/src/lib.rs",
            "painel/dist/painel_bg.wasm"
        ]));
        assert!(
            !dist_atrasado(["painel/exemplo.json"]),
            "o exemplo só serve aos testes"
        );
        assert!(!dist_atrasado(["src/painel.rs"]));
    }

    #[test]
    fn a_lista_diz_nenhuma_quando_vazia() {
        assert_eq!(lista(&conjunto(&[])), "nenhuma");
        assert_eq!(
            lista(&conjunto(&[Painel, Termostato])),
            "termostato, painel"
        );
    }
}

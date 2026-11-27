//! O cache de compilação na frente do rustc, para uma worktree nova não
//! compilar de novo o que outra já compilou. Cada worktree segue com a sua
//! pasta de build; o cache é que fica entre elas.
//!
//! - O `ravenrs-xcache` do RavenRS, se estiver no `PATH`, com a compilação
//!   incremental desligada (o que é incremental não se guarda). No fim o cache
//!   é cortado em `RAVENRS_XCACHE_MAX` (20G), o mesmo teto do RavenRS, porque o
//!   cache é um só para os dois projetos.
//! - Um `RUSTC_WRAPPER` já posto fica como está.
//! - O sccache não serve: ele põe a pasta de build na chave e quase não acerta
//!   entre worktrees.

use std::process::Command;

use crate::run::Checagem;

const XCACHE: &str = "ravenrs-xcache";

/// Com que cache a checagem roda.
#[derive(Debug)]
pub enum Cache {
    /// Nenhum: um `RUSTC_WRAPPER` já posto, ou o xcache fora do `PATH`.
    Nenhum,
    /// O `ravenrs-xcache` do `PATH`.
    Xcache,
}

impl Cache {
    /// Liga o cache em todos os comandos da checagem, se houver um.
    pub fn ligar(c: &mut Checagem) -> Cache {
        if std::env::var_os("RUSTC_WRAPPER").is_some() {
            println!("cache de compilação: o RUSTC_WRAPPER que já estava posto");
            return Cache::Nenhum;
        }
        let versao = Command::new(XCACHE).arg("--version").output();
        let Ok(versao) = versao
            .map_err(drop)
            .and_then(|s| if s.status.success() { Ok(s) } else { Err(()) })
        else {
            println!("cache de compilação: nenhum ({XCACHE} não está no PATH)");
            return Cache::Nenhum;
        };
        c.por_env("RUSTC_WRAPPER", XCACHE);
        c.por_env("CARGO_INCREMENTAL", "0");
        let _ = Command::new(XCACHE).args(["stats", "--reset"]).output();
        println!(
            "cache de compilação: {}",
            String::from_utf8_lossy(&versao.stdout).trim()
        );
        Cache::Xcache
    }

    /// O que o cache fez, e o corte que o impede de tomar o disco.
    pub fn relatorio(&self) {
        if let Cache::Xcache = self {
            println!();
            let _ = Command::new(XCACHE).arg("stats").status();
            let teto = std::env::var("RAVENRS_XCACHE_MAX").unwrap_or_else(|_| "20G".to_owned());
            let _ = Command::new(XCACHE)
                .args(["gc", "--max-size", &teto])
                .status();
        }
    }
}

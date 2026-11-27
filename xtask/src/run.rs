//! Rodar uma checagem: as etapas uma depois da outra, cada uma cronometrada, a
//! primeira falha parando tudo, e a tabela de tempos no fim, passando ou
//! falhando.

use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

/// Por que a checagem parou: o que falhou e o código com que terminou.
#[derive(Debug)]
pub struct Falha {
    /// O que rodou, como seria digitado, ou o que deu errado.
    pub o_que: String,
    /// O código de saída do processo, ou 1 quando ele não chegou a rodar.
    pub codigo: i32,
    /// Se foi um comando que terminou mal, e não uma mensagem.
    comando: bool,
}

impl fmt::Display for Falha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.comando {
            write!(f, "`{}` terminou com o código {}", self.o_que, self.codigo)
        } else {
            write!(f, "{}", self.o_que)
        }
    }
}

/// Uma falha que não é de comando: uma ferramenta que falta, um `dist/` atrasado.
pub fn falha(o_que: impl Into<String>) -> Falha {
    Falha {
        o_que: o_que.into(),
        codigo: 1,
        comando: false,
    }
}

/// O que uma etapa dá: feito, ou a falha que para a checagem.
pub type Feito = Result<(), Falha>;

/// Uma checagem rodando.
#[derive(Debug)]
pub struct Checagem {
    /// O nome: `dev`, `pr`, `release` ou `painel`.
    pub modo: String,
    /// A raiz do repositório, de onde todo comando roda.
    pub raiz: PathBuf,
    /// A pasta de build, a mesma para os dois workspaces (o da raiz e o do painel).
    pub alvo: PathBuf,
    /// As variáveis que todo comando recebe por cima das deste processo.
    env: Vec<(String, OsString)>,
    /// A etapa em curso: o título e quando começou.
    etapa: Option<(String, Instant)>,
    inicio: Instant,
    /// Duração, se falhou e título, uma linha por etapa.
    tempos: Vec<(Duration, bool, String)>,
}

impl Checagem {
    /// Uma checagem chamada `modo`, no repositório em `raiz`.
    pub fn nova(modo: &str, raiz: &Path) -> Checagem {
        // Um CARGO_TARGET_DIR por worktree (skill rust-checagens-xtask); sem ele,
        // o target da raiz, que o painel passa a dividir em vez de criar o seu.
        let alvo =
            std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| raiz.join("target"), PathBuf::from);
        let alvo = if alvo.is_absolute() {
            alvo
        } else {
            raiz.join(alvo)
        };
        let mut checagem = Checagem {
            modo: modo.to_owned(),
            raiz: raiz.to_owned(),
            alvo,
            env: Vec::new(),
            etapa: None,
            inicio: Instant::now(),
            tempos: Vec::new(),
        };
        let alvo = checagem.alvo.clone();
        checagem.por_env("CARGO_TARGET_DIR", alvo);
        checagem
    }

    /// Põe uma variável em todos os comandos que vêm depois. É por `Command::env`,
    /// porque `set_var` é `unsafe` na edição 2024.
    pub fn por_env(&mut self, nome: &str, valor: impl Into<OsString>) {
        self.env.retain(|(conhecido, _)| conhecido != nome);
        self.env.push((nome.to_owned(), valor.into()));
    }

    /// Começa uma etapa, fechando a anterior.
    pub fn etapa(&mut self, titulo: &str) {
        self.fechar(false);
        println!("\n\x1b[1m==> {titulo}\x1b[0m");
        self.etapa = Some((titulo.to_owned(), Instant::now()));
    }

    fn fechar(&mut self, falhou: bool) {
        if let Some((titulo, comeco)) = self.etapa.take() {
            self.tempos.push((comeco.elapsed(), falhou, titulo));
        }
    }

    /// Um comando pronto para rodar na raiz, com as variáveis da checagem.
    pub fn comando(&self, programa: &str, args: &[&str]) -> Command {
        let mut comando = Command::new(programa);
        comando.args(args).current_dir(&self.raiz);
        for (nome, valor) in &self.env {
            comando.env(nome, valor);
        }
        comando
    }

    /// Roda um comando; a checagem falha se ele não terminar com 0.
    pub fn rodar(&self, programa: &str, args: &[&str]) -> Feito {
        rodar_comando(self.comando(programa, args), programa, args)
    }

    /// Roda um comando cuja saída é lida: o que ele escreveu, se terminou com 0.
    pub fn saida(&self, programa: &str, args: &[&str]) -> Result<String, Falha> {
        let o_que = como_digitado(programa, args);
        let saida = self
            .comando(programa, args)
            .output()
            .map_err(|e| falha(format!("{o_que}: {e}")))?;
        if saida.status.success() {
            Ok(String::from_utf8_lossy(&saida.stdout).into_owned())
        } else {
            std::io::stderr().write_all(&saida.stderr).ok();
            Err(Falha {
                o_que,
                codigo: saida.status.code().unwrap_or(1),
                comando: true,
            })
        }
    }

    /// Fecha a checagem: a tabela de tempos, na tela e em
    /// `<alvo>/xtask/tempos-<modo>.txt`, e a linha que diz como foi. Devolve o
    /// código de saída do processo.
    pub fn terminar(mut self, feito: Result<String, Falha>, cache: &crate::cache::Cache) -> i32 {
        let codigo = match &feito {
            Ok(_) => 0,
            Err(f) => f.codigo.max(1),
        };
        self.fechar(codigo != 0);
        let tabela = tabela(&self.modo, &self.tempos, self.inicio.elapsed(), codigo == 0);
        println!("\n{tabela}");
        let pasta = self.alvo.join("xtask");
        let arquivo = pasta.join(format!("tempos-{}.txt", self.modo));
        if fs::create_dir_all(&pasta)
            .and_then(|()| fs::write(&arquivo, &tabela))
            .is_ok()
        {
            println!("(gravada em {})", arquivo.display());
        }
        cache.relatorio();
        match feito {
            Ok(mensagem) => println!("\n\x1b[1;32m{mensagem}\x1b[0m"),
            Err(f) => eprintln!("\n\x1b[1;31merro: {f}.\x1b[0m"),
        }
        codigo
    }
}

/// Roda um comando montado à parte (com uma variável só dele, por exemplo);
/// `programa` e `args` são o que a falha mostra.
pub fn rodar_comando(mut comando: Command, programa: &str, args: &[&str]) -> Feito {
    let o_que = como_digitado(programa, args);
    let status = comando
        .status()
        .map_err(|e| falha(format!("{o_que}: {e}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(Falha {
            o_que,
            codigo: status.code().unwrap_or(1),
            comando: true,
        })
    }
}

fn como_digitado(programa: &str, args: &[&str]) -> String {
    std::iter::once(programa)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `1m05s`, com os minutos alinhados.
fn duracao(d: Duration) -> String {
    let s = d.as_secs();
    format!("{:>3}m{:02}s", s / 60, s % 60)
}

/// A tabela de tempos de uma checagem.
fn tabela(
    modo: &str,
    tempos: &[(Duration, bool, String)],
    total: Duration,
    passou: bool,
) -> String {
    use std::fmt::Write as _;
    let mut t = format!("Tempos do cargo xtask {modo}:\n");
    for (d, falhou, titulo) in tempos {
        let marca = if *falhou { "  ✗ " } else { "    " };
        let _ = writeln!(t, "{}{marca}{titulo}", duracao(*d));
    }
    let _ = writeln!(
        t,
        "{}    total ({})",
        duracao(total),
        if passou { "passou" } else { "falhou" }
    );
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tabela_marca_a_etapa_que_falhou_e_da_o_total() {
        let tempos = vec![
            (Duration::from_secs(3), false, "fmt".to_owned()),
            (Duration::from_secs(65), true, "clippy".to_owned()),
        ];
        let t = tabela("dev", &tempos, Duration::from_secs(68), false);
        assert_eq!(
            t,
            "Tempos do cargo xtask dev:\n  0m03s    fmt\n  1m05s  ✗ clippy\n  1m08s    total (falhou)\n"
        );
    }

    #[test]
    fn a_falha_de_comando_diz_o_comando_e_o_codigo_e_a_mensagem_so_ela() {
        let f = Falha {
            o_que: "cargo test".to_owned(),
            codigo: 101,
            comando: true,
        };
        assert_eq!(f.to_string(), "`cargo test` terminou com o código 101");
        assert_eq!(falha("falta o nextest").to_string(), "falta o nextest");
    }
}

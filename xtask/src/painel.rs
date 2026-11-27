//! `cargo xtask painel`: compila o painel para o navegador e deixa o resultado
//! em `painel/dist/`, de onde o termostato o embute (`src/painel.rs`). Rode
//! depois de mudar qualquer coisa em `painel/`, e commite o `dist/` junto: o RPM
//! não compila wasm.

use std::fs;
use std::path::Path;

use crate::run::{Checagem, Falha, Feito, falha};

/// O alvo do navegador.
pub const WASM: &str = "wasm32-unknown-unknown";

/// O build do painel para o navegador, como o RPM vai embutir.
pub const BUILD_WASM: [&str; 11] = [
    "build",
    "--manifest-path",
    "painel/Cargo.toml",
    "--lib",
    "--target",
    WASM,
    "--profile",
    "wasm-release",
    "--locked",
    "--quiet",
    "--color=always",
];

/// Confere que o alvo wasm32 está instalado no toolchain do projeto. Sem o
/// rustup (outro jeito de instalar o Rust), deixa o cargo dizer.
pub fn alvo_instalado(c: &Checagem) -> Feito {
    let Ok(instalados) = c.saida("rustup", &["target", "list", "--installed"]) else {
        return Ok(());
    };
    if instalados.lines().any(|l| l.trim() == WASM) {
        Ok(())
    } else {
        Err(falha(format!(
            "falta o alvo {WASM} no toolchain do projeto; instale uma vez com: rustup target add {WASM}"
        )))
    }
}

/// A versão de um pacote no `Cargo.lock`.
pub fn versao_no_lock(lock: &str, pacote: &str) -> Option<String> {
    let nome = format!("name = \"{pacote}\"");
    let mut linhas = lock.lines();
    while let Some(linha) = linhas.next() {
        if linha.trim() == nome {
            let versao = linhas
                .next()?
                .trim()
                .strip_prefix("version = \"")?
                .strip_suffix('"')?;
            return Some(versao.to_owned());
        }
    }
    None
}

/// O `wasm-bindgen` da linha de comando tem de ser o mesmo da biblioteca.
fn bindgen_certo(c: &Checagem) -> Feito {
    let lock = fs::read_to_string(c.raiz.join("painel/Cargo.lock"))
        .map_err(|e| falha(format!("painel/Cargo.lock: {e}")))?;
    let quer = versao_no_lock(&lock, "wasm-bindgen")
        .ok_or_else(|| falha("o painel/Cargo.lock não tem o wasm-bindgen"))?;
    let instale = format!("cargo install wasm-bindgen-cli --version {quer} --locked");
    let tem = c
        .saida("wasm-bindgen", &["--version"])
        .map_err(|_| falha(format!("falta o wasm-bindgen; instale com: {instale}")))?;
    if tem.split_whitespace().nth(1) == Some(quer.as_str()) {
        Ok(())
    } else {
        Err(falha(format!(
            "o wasm-bindgen instalado é {}, e o painel usa {quer}; troque com: {instale}",
            tem.trim()
        )))
    }
}

fn copiar(de: &Path, para: &Path) -> Feito {
    fs::copy(de, para)
        .map(drop)
        .map_err(|e| falha(format!("{} → {}: {e}", de.display(), para.display())))
}

/// O tamanho de um arquivo depois do `gzip -9`, se houver gzip.
fn com_gzip(c: &Checagem, arquivo: &Path) -> Option<usize> {
    let saida = c
        .comando("gzip", &["-9", "-c"])
        .arg(arquivo)
        .output()
        .ok()?;
    saida.status.success().then_some(saida.stdout.len())
}

/// Compila o painel e regrava o `painel/dist/`.
pub fn compilar(c: &mut Checagem) -> Result<String, Falha> {
    c.etapa("ferramentas do painel");
    alvo_instalado(c)?;
    bindgen_certo(c)?;

    c.etapa("painel para wasm32 (perfil wasm-release)");
    c.rodar("cargo", &BUILD_WASM)?;

    c.etapa("wasm-bindgen");
    let dist = c.raiz.join("painel/dist");
    if dist.exists() {
        fs::remove_dir_all(&dist).map_err(|e| falha(format!("{}: {e}", dist.display())))?;
    }
    fs::create_dir_all(&dist).map_err(|e| falha(format!("{}: {e}", dist.display())))?;
    let wasm = c.alvo.join(WASM).join("wasm-release/painel.wasm");
    let wasm = wasm.to_string_lossy();
    let args = [
        "--target",
        "web",
        "--no-typescript",
        "--out-dir",
        "painel/dist",
        "--out-name",
        "painel",
        &wasm,
    ];
    c.rodar("wasm-bindgen", &args)?;

    let final_ = dist.join("painel_bg.wasm");
    if c.saida("wasm-opt", &["--version"]).is_ok() {
        c.etapa("wasm-opt -Oz");
        let saida = final_.to_string_lossy();
        c.rodar(
            "wasm-opt",
            &[
                "-Oz",
                "--enable-bulk-memory",
                "--enable-mutable-globals",
                "--enable-multivalue",
                "--enable-nontrapping-float-to-int",
                "--enable-reference-types",
                "--enable-sign-ext",
                "-o",
                &saida,
                &saida,
            ],
        )?;
    } else {
        println!("sem o wasm-opt (binaryen), o .wasm fica maior");
    }
    copiar(
        &c.raiz.join("painel/web/index.html"),
        &dist.join("index.html"),
    )?;

    let bytes = fs::metadata(&final_).map_or(0, |m| m.len());
    let gz = com_gzip(c, &final_).map_or_else(String::new, |g| format!("; {g} com gzip"));
    Ok(format!(
        "painel em painel/dist/ ({bytes} bytes{gz}); commite o dist/ junto."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_versao_vem_do_pacote_certo_do_lock() {
        let lock = "[[package]]\nname = \"wasm-bindgen-futures\"\nversion = \"0.4.80\"\n\n[[package]]\nname = \"wasm-bindgen\"\nversion = \"0.2.130\"\n";
        assert_eq!(
            versao_no_lock(lock, "wasm-bindgen").as_deref(),
            Some("0.2.130")
        );
        assert_eq!(
            versao_no_lock(lock, "wasm-bindgen-futures").as_deref(),
            Some("0.4.80")
        );
        assert_eq!(versao_no_lock(lock, "serde"), None);
    }
}

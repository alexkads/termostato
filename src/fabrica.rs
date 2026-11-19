//! Os valores de fábrica: o que estava no hardware antes da primeira vez.
//!
//! Ficam em `<estado>/fabrica.toml` (padrão `/var/lib/termostato`), gravados
//! **só na primeira subida**: depois disso o hardware já tem os valores do
//! termostato, e ler de novo guardaria os nossos 38 W como se fossem de fábrica.
//!
//! Parar o serviço **não** devolve nada disso: um reinício ou um crash que
//! voltasse o PL1 a 100 W deixaria a máquina sem limite justo quando ela está
//! compilando. Só o `termostato --desinstalar` devolve.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Piso {
    pub ventoinha: String,
    pub rpm: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fabrica {
    pub pl1_uw: Option<u64>,
    #[serde(default)]
    pub pisos: Vec<Piso>,
    /// O TCC offset de fábrica (0 quando não guardado: o chip liga com 0).
    #[serde(default)]
    pub tcc_offset_c: Option<u8>,
}

pub fn arquivo(estado: &Path) -> PathBuf {
    estado.join("fabrica.toml")
}

pub fn ler(estado: &Path) -> Result<Option<Fabrica>, String> {
    let caminho = arquivo(estado);
    match fs::read_to_string(&caminho) {
        Ok(texto) => toml::from_str(&texto)
            .map(Some)
            .map_err(|e| format!("{}: {e}", caminho.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", caminho.display())),
    }
}

/// Grava `achado` se ainda não houver valores de fábrica. Devolve se gravou.
pub fn guardar_se_faltar(estado: &Path, achado: &Fabrica) -> Result<bool, String> {
    if ler(estado)?.is_some() {
        return Ok(false);
    }
    fs::create_dir_all(estado).map_err(|e| format!("{}: {e}", estado.display()))?;
    let texto = toml::to_string(achado).map_err(|e| e.to_string())?;
    let caminho = arquivo(estado);
    fs::write(&caminho, texto).map_err(|e| format!("{}: {e}", caminho.display()))?;
    Ok(true)
}

/// Regrava o arquivo (para acrescentar um valor que faltava, como o TCC).
pub fn regravar(estado: &Path, f: &Fabrica) -> Result<(), String> {
    let texto = toml::to_string(f).map_err(|e| e.to_string())?;
    let caminho = arquivo(estado);
    fs::write(&caminho, texto).map_err(|e| format!("{}: {e}", caminho.display()))
}

pub fn apagar(estado: &Path) -> Result<(), String> {
    let caminho = arquivo(estado);
    match fs::remove_file(&caminho) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("{}: {e}", caminho.display()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn exemplo(pl1: u64) -> Fabrica {
        Fabrica {
            pl1_uw: Some(pl1),
            pisos: vec![Piso {
                ventoinha: "1".into(),
                rpm: 2160,
            }],
            tcc_offset_c: None,
        }
    }

    #[test]
    fn so_a_primeira_subida_grava() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(ler(d.path()).unwrap(), None);
        assert!(guardar_se_faltar(d.path(), &exemplo(100_000_000)).unwrap());
        // Na segunda subida o hardware já tem os 38 W do termostato: não grava.
        assert!(!guardar_se_faltar(d.path(), &exemplo(38_000_000)).unwrap());
        assert_eq!(ler(d.path()).unwrap(), Some(exemplo(100_000_000)));
    }

    #[test]
    fn apagar_sem_arquivo_nao_e_erro() {
        let d = tempfile::tempdir().unwrap();
        apagar(d.path()).unwrap();
        guardar_se_faltar(d.path(), &exemplo(1)).unwrap();
        apagar(d.path()).unwrap();
        assert_eq!(ler(d.path()).unwrap(), None);
    }
}

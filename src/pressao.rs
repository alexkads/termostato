//! O sentido da memória: a pressão (PSI) e o quanto sobra.
//!
//! O PSI (`/proc/pressure/memory`) mede **tempo perdido**: `some avg10=12.5`
//! quer dizer que, nos últimos 10 s, em 12,5 % do tempo algum processo ficou
//! parado esperando memória. É o sinal do sufoco de verdade, e ele aparece
//! **antes** de a máquina parar de responder. Em 19/11/2026 a fedora travou
//! por falta de memória (OOM global, com 16 GB de swap trocando sem parar);
//! este é o sentido que teria visto isso chegando.

use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Pressao {
    /// PSI `some avg10` da memória, em %.
    pub memoria_some_pct: f64,
    /// PSI `full avg10` da memória, em %: todos os processos parados.
    pub memoria_full_pct: f64,
    /// PSI `some avg10` da CPU, em %.
    pub cpu_some_pct: f64,
    /// PSI `some avg10` do disco, em %.
    pub io_some_pct: f64,
    /// `MemAvailable` sobre `MemTotal`, em %.
    pub memoria_disponivel_pct: f64,
    /// Swap em uso, em MB.
    pub swap_usado_mb: f64,
}

/// O `avg10` da linha que começa com `tipo` (`some` ou `full`).
pub fn avg10(texto: &str, tipo: &str) -> Option<f64> {
    texto
        .lines()
        .find(|l| l.starts_with(tipo))?
        .split_whitespace()
        .find_map(|c| c.strip_prefix("avg10="))?
        .parse()
        .ok()
}

/// Um campo do `/proc/meminfo`, em kB.
pub fn meminfo_kb(texto: &str, campo: &str) -> Option<f64> {
    texto
        .lines()
        .find(|l| l.split(':').next() == Some(campo))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

pub fn ler(proc: &Path) -> Option<Pressao> {
    let ler = |nome: &str| fs::read_to_string(proc.join(nome)).ok();
    let memoria = ler("pressure/memory")?;
    let cpu = ler("pressure/cpu").unwrap_or_default();
    let io = ler("pressure/io").unwrap_or_default();
    let meminfo = ler("meminfo").unwrap_or_default();
    let total = meminfo_kb(&meminfo, "MemTotal").unwrap_or(0.0);
    let disponivel = meminfo_kb(&meminfo, "MemAvailable").unwrap_or(0.0);
    let swap_total = meminfo_kb(&meminfo, "SwapTotal").unwrap_or(0.0);
    let swap_livre = meminfo_kb(&meminfo, "SwapFree").unwrap_or(0.0);
    Some(Pressao {
        memoria_some_pct: avg10(&memoria, "some").unwrap_or(0.0),
        memoria_full_pct: avg10(&memoria, "full").unwrap_or(0.0),
        cpu_some_pct: avg10(&cpu, "some").unwrap_or(0.0),
        io_some_pct: avg10(&io, "some").unwrap_or(0.0),
        memoria_disponivel_pct: if total > 0.0 {
            disponivel / total * 100.0
        } else {
            100.0
        },
        swap_usado_mb: (swap_total - swap_livre).max(0.0) / 1024.0,
    })
}

#[cfg(test)]
pub mod pressao_de_teste {
    use std::fs;
    use std::path::Path;

    pub fn escrever(raiz: &Path, some: f64, full: f64, disponivel_pct: f64) {
        fs::create_dir_all(raiz.join("pressure")).unwrap();
        fs::write(
            raiz.join("pressure/memory"),
            format!(
                "some avg10={some:.2} avg60=0.00 avg300=0.00 total=1\nfull avg10={full:.2} avg60=0.00 avg300=0.00 total=1\n"
            ),
        )
        .unwrap();
        fs::write(
            raiz.join("pressure/cpu"),
            "some avg10=3.10 avg60=0.00 avg300=0.00 total=1\n",
        )
        .unwrap();
        fs::write(
            raiz.join("pressure/io"),
            "some avg10=0.50 avg60=0.00 avg300=0.00 total=1\n",
        )
        .unwrap();
        let total = 16_000_000.0;
        fs::write(
            raiz.join("meminfo"),
            format!(
                "MemTotal:       {total:.0} kB\nMemFree:          100000 kB\nMemAvailable:   {:.0} kB\nSwapTotal:      16000000 kB\nSwapFree:       15000000 kB\n",
                total * disponivel_pct / 100.0
            ),
        )
        .unwrap();
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn le_o_psi_e_o_meminfo() {
        let d = tempfile::tempdir().unwrap();
        pressao_de_teste::escrever(d.path(), 12.5, 4.0, 8.0);
        let p = ler(d.path()).unwrap();
        assert!((p.memoria_some_pct - 12.5).abs() < 1e-9);
        assert!((p.memoria_full_pct - 4.0).abs() < 1e-9);
        assert!((p.cpu_some_pct - 3.1).abs() < 1e-9);
        assert!((p.memoria_disponivel_pct - 8.0).abs() < 1e-6);
        assert!((p.swap_usado_mb - 1_000_000.0 / 1024.0).abs() < 1e-6);
    }

    #[test]
    fn sem_psi_nao_ha_leitura() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(ler(d.path()), None);
    }
}

//! O vigia: o que realmente acontece na máquina, além do calor.
//!
//! Nasceu do travamento de 19/11/2026: a fedora parou às 18:32, e só depois,
//! cavando o dump do OOM no `journalctl -k`, apareceu o motivo. Oito `rustc` de
//! ~1,5 GB de RAM mais ~1,4 GB de swap cada esgotaram os 16 GB de swap. O
//! termostato não tinha guardado nada disso. O vigia lê:
//!
//! - **os motivos do freio** que o próprio chip registra (`MSR 0x690`,
//!   `MSR_CORE_PERF_LIMIT_REASONS` do Haswell): térmico, PL1, PL2, corrente,
//!   PROCHOT externo (o SMC ou o carregador) e o regulador de tensão, agora e
//!   desde o boot;
//! - **a bateria e o carregador** (`/sys/class/power_supply`): em 22/11 a
//!   bateria descarregava com o carregador ligado a partir de ~17 W de CPU;
//! - **o swap** (`/proc/vmstat`): páginas que entram e saem por segundo, e as
//!   mortes do OOM killer;
//! - **quem segura a memória** (`/proc/<pid>/status`): RAM e swap somados por
//!   nome. O OOM killer escolhe pela pontuação, não pelo tamanho: em 19/11 ele
//!   matou o traefik (18 MB) e deixou os `rustc`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Os bits do `MSR_CORE_PERF_LIMIT_REASONS` (0x690) no Haswell: o bit de
/// "agora" e o de "desde a última limpeza" (o log, 16 acima).
pub const MSR_MOTIVOS: u64 = 0x690;
const MOTIVOS: &[(&str, u32)] = &[
    ("prochot", 0),
    ("termico", 1),
    ("vr", 6),
    ("corrente", 8),
    ("pl1", 10),
    ("pl2", 11),
    ("turbo_maximo", 12),
];

/// Por que o chip está abaixo da frequência pedida.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Motivos {
    /// (motivo, agora, desde o boot), na ordem de `MOTIVOS`.
    pub lista: Vec<(&'static str, bool, bool)>,
}

impl Motivos {
    pub fn decodificar(valor: u64) -> Self {
        Self {
            lista: MOTIVOS
                .iter()
                .map(|(nome, bit)| (*nome, valor >> bit & 1 == 1, valor >> (bit + 16) & 1 == 1))
                .collect(),
        }
    }

    pub fn agora(&self) -> Vec<&'static str> {
        self.lista.iter().filter(|m| m.1).map(|m| m.0).collect()
    }

    pub fn desde_o_boot(&self) -> Vec<&'static str> {
        self.lista.iter().filter(|m| m.2).map(|m| m.0).collect()
    }
}

/// A bateria e o carregador.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Bateria {
    pub carregador_ligado: Option<bool>,
    /// "Charging", "Discharging", "Full", "Not charging"…
    pub estado: String,
    /// Positivo carregando, negativo descarregando.
    pub amperes: Option<f64>,
    pub carga_pct: Option<f64>,
}

impl Bateria {
    /// A bateria descarrega com o carregador ligado: o carregador não dá conta.
    pub fn descarrega_no_carregador(&self) -> bool {
        self.carregador_ligado == Some(true) && self.amperes.is_some_and(|a| a < -0.1)
    }
}

fn ler_texto(p: &Path) -> Option<String> {
    fs::read_to_string(p).ok().map(|t| t.trim().to_string())
}

/// Acha o carregador (`type = Mains`) e a bateria (`type = Battery`).
pub fn bateria(sysfs: &Path) -> Option<Bateria> {
    let base = sysfs.join("class/power_supply");
    let mut b = Bateria::default();
    let mut achou = false;
    for e in fs::read_dir(&base).ok()?.filter_map(Result::ok) {
        let p = e.path();
        match ler_texto(&p.join("type")).as_deref() {
            Some("Mains") => {
                b.carregador_ligado = ler_texto(&p.join("online")).map(|v| v == "1");
            }
            Some("Battery") => {
                achou = true;
                b.estado = ler_texto(&p.join("status")).unwrap_or_default();
                let micro = ler_texto(&p.join("current_now")).and_then(|v| v.parse::<f64>().ok());
                b.amperes = micro.map(|ua| {
                    let a = ua.abs() / 1e6;
                    if b.estado == "Discharging" { -a } else { a }
                });
                b.carga_pct = ler_texto(&p.join("capacity")).and_then(|v| v.parse().ok());
            }
            _ => {}
        }
    }
    (achou || b.carregador_ligado.is_some()).then_some(b)
}

/// Os contadores do `/proc/vmstat` que importam para o travamento.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Vmstat {
    pub pswpin: u64,
    pub pswpout: u64,
    pub oom_kill: u64,
}

pub fn vmstat(texto: &str) -> Vmstat {
    let mut v = Vmstat::default();
    for linha in texto.lines() {
        let mut c = linha.split_whitespace();
        let (Some(nome), Some(n)) = (c.next(), c.next().and_then(|n| n.parse().ok())) else {
            continue;
        };
        match nome {
            "pswpin" => v.pswpin = n,
            "pswpout" => v.pswpout = n,
            "oom_kill" => v.oom_kill = n,
            _ => {}
        }
    }
    v
}

/// O swap em MB/s entre duas leituras (páginas de 4 KiB): (entra, sai).
pub fn swap_mb_s(antes: Vmstat, agora: Vmstat, segundos: f64) -> (f64, f64) {
    if segundos <= 0.0 {
        return (0.0, 0.0);
    }
    let mb = |a: u64, b: u64| b.saturating_sub(a) as f64 * 4096.0 / 1e6 / segundos;
    (
        mb(antes.pswpin, agora.pswpin),
        mb(antes.pswpout, agora.pswpout),
    )
}

/// Quem segura a memória: um nome, quantas cópias, RAM e swap somados.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MemoriaDe {
    pub nome: String,
    pub copias: u32,
    pub rss_mb: f64,
    pub swap_mb: f64,
}

/// `Name`, `VmRSS` e `VmSwap` de um `/proc/<pid>/status`, em MB.
pub fn ler_status(texto: &str) -> Option<(String, f64, f64)> {
    let mut nome = None;
    let (mut rss, mut swap) = (0.0, 0.0);
    for linha in texto.lines() {
        let Some((chave, valor)) = linha.split_once(':') else {
            continue;
        };
        let kb = || {
            valor
                .split_whitespace()
                .next()
                .and_then(|n| n.parse::<f64>().ok())
                .unwrap_or(0.0)
                / 1024.0
        };
        match chave {
            "Name" => nome = Some(valor.trim().to_string()),
            "VmRSS" => rss = kb(),
            "VmSwap" => swap = kb(),
            _ => {}
        }
    }
    nome.map(|n| (n, rss, swap))
}

/// Os `quantos` nomes que mais seguram memória (RAM + swap).
pub fn maiores_em_memoria(proc: &Path, quantos: usize) -> Vec<MemoriaDe> {
    let mut por_nome: BTreeMap<String, MemoriaDe> = BTreeMap::new();
    for e in fs::read_dir(proc)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
    {
        let pid = e.file_name();
        if !pid
            .to_str()
            .is_some_and(|p| p.bytes().all(|b| b.is_ascii_digit()))
        {
            continue;
        }
        let Some((nome, rss, swap)) = fs::read_to_string(e.path().join("status"))
            .ok()
            .and_then(|t| ler_status(&t))
        else {
            continue;
        };
        let m = por_nome.entry(nome.clone()).or_insert_with(|| MemoriaDe {
            nome,
            ..MemoriaDe::default()
        });
        m.copias += 1;
        m.rss_mb += rss;
        m.swap_mb += swap;
    }
    let mut v: Vec<MemoriaDe> = por_nome.into_values().collect();
    v.sort_by(|a, b| (b.rss_mb + b.swap_mb).total_cmp(&(a.rss_mb + a.swap_mb)));
    v.truncate(quantos);
    v
}

/// O identificador deste boot: muda a cada reinício.
pub fn boot_id(proc: &Path) -> String {
    ler_texto(&proc.join("sys/kernel/random/boot_id")).unwrap_or_default()
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn decodifica_os_motivos_da_fedora() {
        // Lido na fedora em 22/11/2026: só o turbo máximo agora; térmico,
        // corrente, PL1, PL2 e turbo desde o boot; nunca PROCHOT nem VR.
        let m = Motivos::decodificar(0x3d22_1000);
        assert_eq!(m.agora(), vec!["turbo_maximo"]);
        assert_eq!(
            m.desde_o_boot(),
            vec!["termico", "corrente", "pl1", "pl2", "turbo_maximo"]
        );
        let m = Motivos::decodificar(1 | 1 << 16);
        assert_eq!(m.agora(), vec!["prochot"]);
        assert_eq!(m.desde_o_boot(), vec!["prochot"]);
    }

    fn escrever(raiz: &Path, caminho: &str, texto: &str) {
        let p = raiz.join(caminho);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, texto).unwrap();
    }

    #[test]
    fn bateria_descarregando_no_carregador() {
        let d = tempfile::tempdir().unwrap();
        escrever(d.path(), "class/power_supply/ADP1/type", "Mains\n");
        escrever(d.path(), "class/power_supply/ADP1/online", "1\n");
        escrever(d.path(), "class/power_supply/BAT0/type", "Battery\n");
        escrever(d.path(), "class/power_supply/BAT0/status", "Discharging\n");
        escrever(d.path(), "class/power_supply/BAT0/current_now", "1312000\n");
        escrever(d.path(), "class/power_supply/BAT0/capacity", "90\n");
        let b = bateria(d.path()).unwrap();
        assert_eq!(b.carregador_ligado, Some(true));
        assert_eq!(b.amperes, Some(-1.312));
        assert_eq!(b.carga_pct, Some(90.0));
        assert!(b.descarrega_no_carregador());
        escrever(d.path(), "class/power_supply/BAT0/status", "Charging\n");
        let b = bateria(d.path()).unwrap();
        assert_eq!(b.amperes, Some(1.312));
        assert!(!b.descarrega_no_carregador());
        assert_eq!(bateria(&d.path().join("nada")), None);
    }

    #[test]
    fn swap_pelo_delta_do_vmstat() {
        let a = vmstat("nr_free_pages 1\npswpin 1000\npswpout 2000\noom_kill 0\n");
        let b = vmstat("pswpin 3442\npswpout 2000\noom_kill 2\n");
        assert_eq!(b.oom_kill, 2);
        let (entra, sai) = swap_mb_s(a, b, 2.0);
        assert!((entra - 2442.0 * 4096.0 / 1e6 / 2.0).abs() < 1e-9);
        assert!(sai.abs() < 1e-9);
        assert_eq!(swap_mb_s(a, b, 0.0), (0.0, 0.0));
    }

    #[test]
    fn soma_a_memoria_por_nome() {
        let d = tempfile::tempdir().unwrap();
        let s = |nome, rss, swap| format!("Name:\t{nome}\nVmRSS:\t{rss} kB\nVmSwap:\t{swap} kB\n");
        escrever(d.path(), "10/status", &s("rustc", 1_572_864, 1_048_576));
        escrever(d.path(), "11/status", &s("rustc", 1_572_864, 1_048_576));
        escrever(d.path(), "12/status", &s("traefik", 18_432, 0));
        escrever(d.path(), "13/status", "Name:\tkthreadd\n");
        escrever(d.path(), "self/status", &s("intruso", 99_999_999, 0));
        let v = maiores_em_memoria(d.path(), 2);
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].nome.as_str(), v[0].copias), ("rustc", 2));
        assert!((v[0].rss_mb - 3072.0).abs() < 1e-9);
        assert!((v[0].swap_mb - 2048.0).abs() < 1e-9);
        assert_eq!(v[1].nome, "traefik");
    }
}

//! O sentido dos processos: quem gasta CPU e memória, e em que cgroup mora.
//!
//! A cada leitura, `/proc/[pid]/stat` dá o nome (`comm`), o tempo de CPU
//! (`utime` + `stime`, em ticks) e a memória residente; `/proc/[pid]/cgroup`
//! dá o cgroup (a linha `0::/caminho` do cgroup v2). O uso de CPU vem do
//! **delta** de ticks desde a leitura anterior, somado por nome e por cgroup.
//!
//! A fatia é da máquina inteira: 1,0 = todos os núcleos ocupados. Um processo
//! que ocupa um núcleo inteiro num i7 de 8 threads tem fatia 0,125.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Ticks por segundo do `/proc` (`CLK_TCK`). É 100 em todo Linux x86_64 atual;
/// ler do `sysconf` exigiria `unsafe` ou uma dependência, e o projeto proíbe o
/// primeiro.
pub const TICKS_POR_S: f64 = 100.0;

/// Um processo numa leitura.
#[derive(Debug, Clone, PartialEq)]
pub struct Amostra {
    pub pid: u32,
    pub nome: String,
    pub ticks: u64,
    pub rss_mb: f64,
    pub cgroup: String,
}

/// O uso somado de um nome (ou de um cgroup) entre duas leituras.
#[derive(Debug, Clone, PartialEq)]
pub struct Uso {
    pub chave: String,
    /// Fatia da CPU da máquina inteira (0 a 1).
    pub fatia: f64,
    pub copias: u32,
    pub rss_mb: f64,
}

/// Interpreta uma linha de `/proc/[pid]/stat`. O nome vem entre parênteses e
/// pode ter espaços e parênteses dentro: o fim dele é o **último** `)`.
pub fn ler_stat(pid: u32, texto: &str, cgroup: &str) -> Option<Amostra> {
    let abre = texto.find('(')?;
    let fecha = texto.rfind(')')?;
    let nome = texto.get(abre + 1..fecha)?.to_string();
    let resto: Vec<&str> = texto.get(fecha + 2..)?.split_whitespace().collect();
    // Depois do `)`: estado é o 3º campo do stat; utime o 14º, stime o 15º,
    // rss (páginas) o 24º. Aqui os índices começam no estado.
    let utime: u64 = resto.get(11)?.parse().ok()?;
    let stime: u64 = resto.get(12)?.parse().ok()?;
    let rss_paginas: u64 = resto.get(21)?.parse().ok()?;
    Some(Amostra {
        pid,
        nome,
        ticks: utime + stime,
        rss_mb: rss_paginas as f64 * 4096.0 / 1_048_576.0,
        cgroup: cgroup.to_string(),
    })
}

/// O cgroup v2 de `/proc/[pid]/cgroup` (`0::/system.slice/x.service`).
pub fn ler_cgroup(texto: &str) -> String {
    texto
        .lines()
        .find_map(|l| l.strip_prefix("0::"))
        .unwrap_or("/")
        .trim()
        .to_string()
}

/// Todos os processos de agora. Um processo que termina no meio da leitura é
/// só ignorado.
pub fn ler_todos(proc: &Path) -> Vec<Amostra> {
    let Ok(dirs) = fs::read_dir(proc) else {
        return Vec::new();
    };
    dirs.filter_map(Result::ok)
        .filter_map(|e| {
            let pid: u32 = e.file_name().to_str()?.parse().ok()?;
            let stat = fs::read_to_string(e.path().join("stat")).ok()?;
            let cgroup = fs::read_to_string(e.path().join("cgroup"))
                .map_or_else(|_| "/".into(), |t| ler_cgroup(&t));
            ler_stat(pid, &stat, &cgroup)
        })
        .collect()
}

/// Guarda os ticks da leitura anterior para calcular o delta.
#[derive(Debug, Default)]
pub struct Observador {
    anterior: HashMap<u32, u64>,
    quando_s: Option<f64>,
}

/// O que uma leitura viu: por nome e por cgroup, do mais gastador ao menor.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Visao {
    pub por_nome: Vec<Uso>,
    pub por_cgroup: Vec<Uso>,
}

impl Observador {
    /// `agora_s`: segundos desde a subida; `nucleos`: threads da máquina.
    pub fn observar(&mut self, agora_s: f64, amostras: &[Amostra], nucleos: f64) -> Visao {
        let dt = self.quando_s.map(|antes| agora_s - antes);
        let mut por_nome: HashMap<&str, Uso> = HashMap::new();
        let mut por_cgroup: HashMap<&str, Uso> = HashMap::new();
        for a in amostras {
            let delta = self
                .anterior
                .get(&a.pid)
                .map_or(0, |&antes| a.ticks.saturating_sub(antes));
            let fatia = match dt {
                Some(dt) if dt > 0.0 && nucleos > 0.0 => {
                    delta as f64 / (TICKS_POR_S * dt * nucleos)
                }
                _ => 0.0,
            };
            for (mapa, chave) in [
                (&mut por_nome, a.nome.as_str()),
                (&mut por_cgroup, a.cgroup.as_str()),
            ] {
                let u = mapa.entry(chave).or_insert_with(|| Uso {
                    chave: chave.to_string(),
                    fatia: 0.0,
                    copias: 0,
                    rss_mb: 0.0,
                });
                u.fatia += fatia;
                u.copias += 1;
                u.rss_mb += a.rss_mb;
            }
        }
        self.anterior = amostras.iter().map(|a| (a.pid, a.ticks)).collect();
        self.quando_s = Some(agora_s);
        let ordenar = |m: HashMap<&str, Uso>| {
            let mut v: Vec<Uso> = m.into_values().collect();
            v.sort_by(|a, b| {
                b.fatia
                    .total_cmp(&a.fatia)
                    .then_with(|| a.chave.cmp(&b.chave))
            });
            v
        };
        Visao {
            por_nome: ordenar(por_nome),
            por_cgroup: ordenar(por_cgroup),
        }
    }
}

#[cfg(test)]
pub mod proc_de_teste {
    //! Um `/proc` de mentira numa pasta temporária.
    use std::fs;
    use std::path::Path;

    /// Escreve um processo com `ticks` de CPU, `rss_paginas` e o cgroup.
    pub fn processo(raiz: &Path, pid: u32, nome: &str, ticks: u64, rss_paginas: u64, cgroup: &str) {
        let d = raiz.join(pid.to_string());
        fs::create_dir_all(&d).unwrap();
        // pid (nome) estado ppid … utime stime … rss: os campos que importam
        // nas posições certas, o resto zero.
        let mut campos = vec!["0".to_string(); 44];
        campos[0] = "S".into();
        campos[11] = ticks.to_string();
        campos[12] = "0".into();
        campos[21] = rss_paginas.to_string();
        fs::write(
            d.join("stat"),
            format!("{pid} ({nome}) {}\n", campos.join(" ")),
        )
        .unwrap();
        fs::write(d.join("cgroup"), format!("0::{cgroup}\n")).unwrap();
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn nome_com_espaco_e_parentese() {
        let a = ler_stat(
            7,
            "7 (tokio (rt) 1) S 1 2 3 4 5 6 7 8 9 10 150 50 0 0 0 0 0 0 0 0 256 0",
            "/x",
        )
        .unwrap();
        assert_eq!(a.nome, "tokio (rt) 1");
        assert_eq!(a.ticks, 200);
        assert!((a.rss_mb - 1.0).abs() < 1e-9);
    }

    #[test]
    fn cgroup_v2() {
        assert_eq!(
            ler_cgroup("0::/system.slice/docker-abc.scope\n"),
            "/system.slice/docker-abc.scope"
        );
        assert_eq!(ler_cgroup("lixo"), "/");
    }

    #[test]
    fn fatia_pelo_delta_somada_por_nome_e_cgroup() {
        let d = tempfile::tempdir().unwrap();
        let bat = "/user.slice/user-1000.slice/user@1000.service/app.slice/bateria-noturna.service";
        for pid in [10, 11] {
            proc_de_teste::processo(d.path(), pid, "rustc", 0, 25_600, bat);
        }
        proc_de_teste::processo(d.path(), 20, "gnome-shell", 0, 51_200, "/user.slice/x");
        let mut o = Observador::default();
        assert!(
            o.observar(0.0, &ler_todos(d.path()), 8.0)
                .por_nome
                .iter()
                .all(|u| u.fatia == 0.0)
        );
        // 2 s depois: cada rustc gastou 200 ticks (um núcleo inteiro), o
        // gnome-shell 20.
        for pid in [10, 11] {
            proc_de_teste::processo(d.path(), pid, "rustc", 200, 25_600, bat);
        }
        proc_de_teste::processo(d.path(), 20, "gnome-shell", 20, 51_200, "/user.slice/x");
        let v = o.observar(2.0, &ler_todos(d.path()), 8.0);
        let rustc = &v.por_nome[0];
        assert_eq!(rustc.chave, "rustc");
        assert_eq!(rustc.copias, 2);
        assert!(
            (rustc.fatia - 0.25).abs() < 1e-9,
            "2 núcleos de 8: {}",
            rustc.fatia
        );
        assert!((rustc.rss_mb - 200.0).abs() < 1e-9);
        assert_eq!(v.por_cgroup[0].chave, bat);
        assert!((v.por_nome[1].fatia - 0.0125).abs() < 1e-9);
    }

    #[test]
    fn processo_novo_nao_conta_na_primeira_vez() {
        let d = tempfile::tempdir().unwrap();
        let mut o = Observador::default();
        o.observar(0.0, &ler_todos(d.path()), 8.0);
        proc_de_teste::processo(d.path(), 5, "cc", 9_999, 1, "/");
        let v = o.observar(2.0, &ler_todos(d.path()), 8.0);
        assert!(
            v.por_nome[0].fatia.abs() < 1e-9,
            "sem leitura anterior não há delta"
        );
    }
}

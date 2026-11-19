//! As mãos: segurar quem abusa, no próprio cgroup, e devolver depois.
//!
//! O freio do RAPL aperta a CPU **inteira**. As mãos apertam **só o
//! culpado**, no cgroup em que ele já mora (o serviço da bateria noturna, a
//! sessão do SSH, o escopo do contêiner), sem movê-lo de lá:
//!
//! - **CPU** (`cpu.max`): com a curva de freio acima de `cpu_a_partir_de`, o
//!   cgroup que mais gasta fica com no máximo `fatia × núcleos × (1 −
//!   intensidade)` núcleos, nunca menos que `minimo_cpus`. Num contêiner,
//!   o pedido vai ao Docker (`NanoCpus`), o mesmo controle que o `docker
//!   update --cpus` usa.
//! - **Memória** (`memory.high`): com a pressão de memória (PSI) acima de
//!   `memoria_aperto_pct`, ou menos de 10 % disponível, o cgroup que mais ocupa
//!   fica com o `memory.high` em 90 % do que usa agora. O kernel o obriga a
//!   devolver memória e ele anda mais devagar; **ninguém é morto**.
//!
//! Com a máquina calma (`calmo_s` sem freio e sem pressão), os limites voltam
//! ao original, um por leitura. Os originais ficam em `<estado>/segurados.json`;
//! ao subir, o termostato devolve qualquer limite que tenha ficado de um crash.
//!
//! No modo **observar** (o padrão), nada é gravado: só o que faria vai ao
//! journal, às métricas e ao Grafana.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::{Config, Modo};
use crate::pressao::Pressao;
use crate::processos::Uso;

/// O que as mãos decidiram numa leitura.
#[derive(Debug, Clone, PartialEq)]
pub enum Acao {
    SegurarCpu {
        cgroup: String,
        cpus: f64,
        fatia: f64,
    },
    SegurarMemoria {
        cgroup: String,
        rss_mb: f64,
    },
    Devolver {
        cgroup: String,
    },
}

/// Um cgroup segurado, com o que ele tinha antes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Segurado {
    /// O `cpu.max` original (ex.: `max 100000`), quando a CPU foi segurada.
    pub cpu_max: Option<String>,
    /// O `NanoCpus` original no Docker, quando o contêiner foi segurado por lá.
    pub docker_nano_cpus: Option<i64>,
    /// O `memory.high` original (ex.: `max`), quando a memória foi segurada.
    pub memory_high: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Segurados {
    pub cgroups: BTreeMap<String, Segurado>,
}

pub fn arquivo(estado: &Path) -> PathBuf {
    estado.join("segurados.json")
}

impl Segurados {
    pub fn ler(estado: &Path) -> Self {
        fs::read_to_string(arquivo(estado))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn gravar(&self, estado: &Path) -> Result<(), String> {
        fs::create_dir_all(estado).map_err(|e| format!("{}: {e}", estado.display()))?;
        let texto = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(arquivo(estado), texto).map_err(|e| e.to_string())
    }
}

/// O id do contêiner, se o cgroup for de um (`/system.slice/docker-<id>.scope`).
pub fn id_do_conteiner(cgroup: &str) -> Option<&str> {
    let nome = cgroup.rsplit('/').next()?;
    let id = nome.strip_prefix("docker-")?.strip_suffix(".scope")?;
    (id.len() == 64 && id.bytes().all(|b| b.is_ascii_hexdigit())).then_some(id)
}

/// Um cgroup que nunca pode ser segurado: pela lista (começo do caminho ou do
/// nome do contêiner) ou por ser a raiz.
pub fn intocavel(cgroup: &str, nome_do_conteiner: Option<&str>, cfg: &Config) -> bool {
    cgroup == "/"
        || cfg.maos.nunca.iter().any(|n| {
            cgroup.starts_with(n.as_str())
                || nome_do_conteiner
                    .is_some_and(|c| c.trim_start_matches('/').starts_with(n.as_str()))
        })
}

/// A regra, pura. `nomes` dá o nome do contêiner de um cgroup, quando houver.
#[derive(Default)]
pub struct Maos {
    calmo_desde_s: Option<f64>,
}

impl Maos {
    #[allow(clippy::too_many_arguments)]
    pub fn decidir(
        &mut self,
        agora_s: f64,
        cgroups: &[Uso],
        intensidade: f64,
        pressao: Option<&Pressao>,
        segurados: &Segurados,
        nomes: &dyn Fn(&str) -> Option<String>,
        nucleos: f64,
        cfg: &Config,
    ) -> Vec<Acao> {
        let m = &cfg.maos;
        if m.modo == Modo::Desligado {
            return Vec::new();
        }
        let livre = |u: &&Uso| !intocavel(&u.chave, nomes(&u.chave).as_deref(), cfg);
        let aperto_de_memoria = pressao.is_some_and(|p| {
            p.memoria_some_pct >= m.memoria_aperto_pct || p.memoria_disponivel_pct < 10.0
        });
        let calmo =
            intensidade <= 0.0 && pressao.is_none_or(|p| p.memoria_some_pct < m.memoria_folga_pct);
        let mut acoes = Vec::new();

        if intensidade >= m.cpu_a_partir_de {
            let ja = |c: &str| {
                segurados
                    .cgroups
                    .get(c)
                    .is_some_and(|s| s.cpu_max.is_some() || s.docker_nano_cpus.is_some())
            };
            if let Some(u) = cgroups
                .iter()
                .filter(livre)
                .find(|u| u.fatia >= m.fatia_minima && !ja(&u.chave))
            {
                let cpus = (u.fatia * nucleos * (1.0 - intensidade)).max(m.minimo_cpus);
                acoes.push(Acao::SegurarCpu {
                    cgroup: u.chave.clone(),
                    cpus,
                    fatia: u.fatia,
                });
            }
        }
        if aperto_de_memoria {
            let ja = |c: &str| {
                segurados
                    .cgroups
                    .get(c)
                    .is_some_and(|s| s.memory_high.is_some())
            };
            let maior = cgroups
                .iter()
                .filter(livre)
                .filter(|u| !ja(&u.chave) && u.rss_mb >= 256.0)
                .max_by(|a, b| a.rss_mb.total_cmp(&b.rss_mb));
            if let Some(u) = maior {
                acoes.push(Acao::SegurarMemoria {
                    cgroup: u.chave.clone(),
                    rss_mb: u.rss_mb,
                });
            }
        }

        if calmo {
            let desde = *self.calmo_desde_s.get_or_insert(agora_s);
            if agora_s - desde >= m.calmo_s
                && let Some(c) = segurados.cgroups.keys().next()
            {
                acoes.push(Acao::Devolver { cgroup: c.clone() });
            }
        } else {
            self.calmo_desde_s = None;
        }
        acoes
    }
}

/// Executa as ações (só no modo agir), guardando os originais.
pub struct Executor {
    cgroup_raiz: PathBuf,
    docker: PathBuf,
}

impl Executor {
    pub fn new(cfg: &Config) -> Self {
        Self {
            cgroup_raiz: PathBuf::from(&cfg.sysfs).join("fs/cgroup"),
            docker: PathBuf::from(&cfg.docker_socket),
        }
    }

    fn caminho(&self, cgroup: &str, arquivo: &str) -> PathBuf {
        self.cgroup_raiz
            .join(cgroup.trim_start_matches('/'))
            .join(arquivo)
    }

    fn ler(&self, cgroup: &str, arquivo: &str) -> Result<String, String> {
        let p = self.caminho(cgroup, arquivo);
        fs::read_to_string(&p)
            .map(|t| t.trim().to_string())
            .map_err(|e| format!("{}: {e}", p.display()))
    }

    fn gravar(&self, cgroup: &str, arquivo: &str, valor: &str) -> Result<(), String> {
        let p = self.caminho(cgroup, arquivo);
        fs::write(&p, valor).map_err(|e| format!("{}: {e}", p.display()))
    }

    /// Aplica uma ação. Devolve o texto do que foi feito.
    pub fn aplicar(&self, acao: &Acao, segurados: &mut Segurados) -> Result<String, String> {
        match acao {
            Acao::SegurarCpu { cgroup, cpus, .. } => {
                let s = segurados.cgroups.entry(cgroup.clone()).or_default();
                if let Some(id) = id_do_conteiner(cgroup) {
                    let original = docker_nano_cpus(&self.docker, id)?;
                    docker_atualizar(&self.docker, id, (cpus * 1e9) as i64)?;
                    s.docker_nano_cpus = Some(original);
                    Ok(format!("pedi ao Docker: {cpus:.1} núcleos"))
                } else {
                    let original = self.ler(cgroup, "cpu.max")?;
                    self.gravar(
                        cgroup,
                        "cpu.max",
                        &format!("{} 100000", (cpus * 100_000.0) as u64),
                    )?;
                    s.cpu_max = Some(original);
                    Ok(format!("cpu.max em {cpus:.1} núcleos"))
                }
            }
            Acao::SegurarMemoria { cgroup, .. } => {
                let original = self.ler(cgroup, "memory.high")?;
                let atual: u64 = self
                    .ler(cgroup, "memory.current")?
                    .parse()
                    .map_err(|e| format!("memory.current: {e}"))?;
                let alvo = atual / 10 * 9;
                self.gravar(cgroup, "memory.high", &alvo.to_string())?;
                segurados
                    .cgroups
                    .entry(cgroup.clone())
                    .or_default()
                    .memory_high = Some(original);
                Ok(format!("memory.high em {} MB", alvo / 1_048_576))
            }
            Acao::Devolver { cgroup } => {
                let Some(s) = segurados.cgroups.remove(cgroup) else {
                    return Ok("nada a devolver".into());
                };
                let mut feito = Vec::new();
                // Um cgroup que sumiu (o processo terminou) não tem o que devolver.
                if !self.caminho(cgroup, "").exists() && id_do_conteiner(cgroup).is_none() {
                    return Ok("o cgroup já não existe".into());
                }
                if let Some(v) = &s.cpu_max {
                    self.gravar(cgroup, "cpu.max", v)?;
                    feito.push(format!("cpu.max {v}"));
                }
                if let (Some(n), Some(id)) = (s.docker_nano_cpus, id_do_conteiner(cgroup)) {
                    match docker_atualizar(&self.docker, id, n) {
                        Ok(()) => feito.push(format!("Docker NanoCpus {n}")),
                        Err(e) if e.contains("404") => {
                            feito.push("o contêiner já não existe".into());
                        }
                        Err(e) => return Err(e),
                    }
                }
                if let Some(v) = &s.memory_high {
                    self.gravar(cgroup, "memory.high", v)?;
                    feito.push(format!("memory.high {v}"));
                }
                Ok(format!("devolvido: {}", feito.join(", ")))
            }
        }
    }
}

/// Um pedido HTTP/1.0 ao Docker pelo socket. Devolve (status, corpo).
pub fn docker_pedir(
    socket: &Path,
    metodo: &str,
    caminho: &str,
    corpo: Option<&str>,
) -> Result<(u16, String), String> {
    let mut s =
        UnixStream::connect(socket).map_err(|e| format!("Docker em {}: {e}", socket.display()))?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok();
    s.set_write_timeout(Some(Duration::from_secs(3))).ok();
    let corpo = corpo.unwrap_or("");
    let pedido = format!(
        "{metodo} {caminho} HTTP/1.0\r\nHost: docker\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{corpo}",
        corpo.len()
    );
    s.write_all(pedido.as_bytes())
        .map_err(|e| format!("Docker: {e}"))?;
    let mut resposta = String::new();
    s.read_to_string(&mut resposta)
        .map_err(|e| format!("Docker: {e}"))?;
    let status = resposta
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or("Docker: resposta sem status")?;
    let corpo = resposta
        .split_once("\r\n\r\n")
        .map_or("", |(_, c)| c)
        .to_string();
    Ok((status, corpo))
}

/// O nome de um contêiner, sem a barra (`/monitoramento-grafana-1` → `monitoramento-grafana-1`).
pub fn docker_nome(socket: &Path, id: &str) -> Option<String> {
    let (status, corpo) =
        docker_pedir(socket, "GET", &format!("/containers/{id}/json"), None).ok()?;
    if status != 200 {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(&corpo).ok()?;
    Some(v["Name"].as_str()?.trim_start_matches('/').to_string())
}

fn docker_nano_cpus(socket: &Path, id: &str) -> Result<i64, String> {
    let (status, corpo) = docker_pedir(socket, "GET", &format!("/containers/{id}/json"), None)?;
    if status != 200 {
        return Err(format!("Docker respondeu {status} ao ler {id}"));
    }
    let v: serde_json::Value = serde_json::from_str(&corpo).map_err(|e| e.to_string())?;
    Ok(v["HostConfig"]["NanoCpus"].as_i64().unwrap_or(0))
}

fn docker_atualizar(socket: &Path, id: &str, nano_cpus: i64) -> Result<(), String> {
    let corpo = serde_json::json!({ "NanoCpus": nano_cpus }).to_string();
    let (status, resposta) = docker_pedir(
        socket,
        "POST",
        &format!("/containers/{id}/update"),
        Some(&corpo),
    )?;
    if status == 200 {
        Ok(())
    } else {
        Err(format!("Docker respondeu {status}: {}", resposta.trim()))
    }
}

/// O modo observar: registra no estado simulado o que **seria** feito, sem
/// tocar em nada. A regra de decisão é a mesma do modo agir; sem isto, cada
/// leitura decidia de novo segurar o mesmo cgroup (931 anotações iguais no
/// Grafana em 19/11/2026).
pub fn simular(simulados: &mut Segurados, acao: &Acao) {
    match acao {
        Acao::SegurarCpu { cgroup, .. } => {
            simulados.cgroups.entry(cgroup.clone()).or_default().cpu_max = Some("observado".into());
        }
        Acao::SegurarMemoria { cgroup, .. } => {
            simulados
                .cgroups
                .entry(cgroup.clone())
                .or_default()
                .memory_high = Some("observado".into());
        }
        Acao::Devolver { cgroup } => {
            simulados.cgroups.remove(cgroup);
        }
    }
}

/// Ao subir: devolve o que ficou segurado de um crash.
pub fn devolver_tudo(exec: &Executor, segurados: &mut Segurados) -> Vec<String> {
    let cgroups: Vec<String> = segurados.cgroups.keys().cloned().collect();
    cgroups
        .into_iter()
        .map(|c| {
            let r = exec.aplicar(&Acao::Devolver { cgroup: c.clone() }, segurados);
            format!("{c}: {}", r.unwrap_or_else(|e| format!("⚠️ {e}")))
        })
        .collect()
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::pressao::Pressao;

    const BAT: &str =
        "/user.slice/user-1000.slice/user@1000.service/app.slice/bateria-noturna.service";
    const DOCKER: &str = "/system.slice/docker-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef.scope";

    fn uso(c: &str, fatia: f64, rss_mb: f64) -> Uso {
        Uso {
            chave: c.into(),
            fatia,
            copias: 1,
            rss_mb,
        }
    }

    fn sem_nome(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn segura_a_cpu_do_que_mais_gasta_e_nunca_o_intocavel() {
        let c = Config::default();
        let mut m = Maos::default();
        let usos = [
            uso("/system.slice/sshd.service", 0.9, 10.0),
            uso(BAT, 0.8, 10.0),
        ];
        let a = m.decidir(
            0.0,
            &usos,
            0.5,
            None,
            &Segurados::default(),
            &sem_nome,
            8.0,
            &c,
        );
        assert_eq!(
            a,
            [Acao::SegurarCpu {
                cgroup: BAT.into(),
                cpus: 3.2,
                fatia: 0.8
            }],
            "0,8 × 8 × (1 − 0,5)"
        );
    }

    #[test]
    fn nunca_abaixo_do_minimo_e_nada_sem_freio() {
        let c = Config::default();
        let mut m = Maos::default();
        let a = m.decidir(
            0.0,
            &[uso(BAT, 0.2, 10.0)],
            1.0,
            None,
            &Segurados::default(),
            &sem_nome,
            8.0,
            &c,
        );
        assert_eq!(
            a,
            [Acao::SegurarCpu {
                cgroup: BAT.into(),
                cpus: 1.0,
                fatia: 0.2
            }]
        );
        assert!(
            m.decidir(
                0.0,
                &[uso(BAT, 0.9, 10.0)],
                0.1,
                None,
                &Segurados::default(),
                &sem_nome,
                8.0,
                &c
            )
            .is_empty()
        );
    }

    #[test]
    fn conteiner_do_monitoramento_pelo_nome() {
        let c = Config::default();
        let mut m = Maos::default();
        let nomes = |cg: &str| (cg == DOCKER).then(|| "monitoramento-grafana-1".to_string());
        let a = m.decidir(
            0.0,
            &[uso(DOCKER, 0.9, 10.0)],
            0.6,
            None,
            &Segurados::default(),
            &nomes,
            8.0,
            &c,
        );
        assert!(a.is_empty());
    }

    #[test]
    fn memoria_apertada_segura_o_maior() {
        let c = Config::default();
        let mut m = Maos::default();
        let p = Pressao {
            memoria_some_pct: 25.0,
            memoria_disponivel_pct: 30.0,
            ..Pressao::default()
        };
        let usos = [uso(BAT, 0.1, 6_000.0), uso("/user.slice/x", 0.1, 900.0)];
        let a = m.decidir(
            0.0,
            &usos,
            0.0,
            Some(&p),
            &Segurados::default(),
            &sem_nome,
            8.0,
            &c,
        );
        assert_eq!(
            a,
            [Acao::SegurarMemoria {
                cgroup: BAT.into(),
                rss_mb: 6_000.0
            }]
        );
    }

    #[test]
    fn devolve_depois_de_calmo_por_60_s() {
        let c = Config::default();
        let mut m = Maos::default();
        let mut s = Segurados::default();
        s.cgroups.insert(
            BAT.into(),
            Segurado {
                cpu_max: Some("max 100000".into()),
                ..Segurado::default()
            },
        );
        let p = Pressao::default();
        assert!(
            m.decidir(0.0, &[], 0.0, Some(&p), &s, &sem_nome, 8.0, &c)
                .is_empty()
        );
        assert!(
            m.decidir(30.0, &[], 0.0, Some(&p), &s, &sem_nome, 8.0, &c)
                .is_empty()
        );
        // Um freio no meio zera a calma.
        m.decidir(40.0, &[], 0.2, Some(&p), &s, &sem_nome, 8.0, &c);
        assert!(
            m.decidir(90.0, &[], 0.0, Some(&p), &s, &sem_nome, 8.0, &c)
                .is_empty()
        );
        let a = m.decidir(151.0, &[], 0.0, Some(&p), &s, &sem_nome, 8.0, &c);
        assert_eq!(a, [Acao::Devolver { cgroup: BAT.into() }]);
    }

    #[test]
    fn executa_e_devolve_no_cgroup() {
        let sys = tempfile::tempdir().unwrap();
        let c = Config {
            sysfs: sys.path().display().to_string(),
            ..Config::default()
        };
        let d = sys
            .path()
            .join("fs/cgroup")
            .join(BAT.trim_start_matches('/'));
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("cpu.max"), "max 100000\n").unwrap();
        fs::write(d.join("memory.high"), "max\n").unwrap();
        fs::write(d.join("memory.current"), "1048576000\n").unwrap();
        let e = Executor::new(&c);
        let mut s = Segurados::default();
        e.aplicar(
            &Acao::SegurarCpu {
                cgroup: BAT.into(),
                cpus: 2.0,
                fatia: 0.8,
            },
            &mut s,
        )
        .unwrap();
        e.aplicar(
            &Acao::SegurarMemoria {
                cgroup: BAT.into(),
                rss_mb: 1000.0,
            },
            &mut s,
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(d.join("cpu.max")).unwrap(),
            "200000 100000"
        );
        assert_eq!(
            fs::read_to_string(d.join("memory.high")).unwrap(),
            "943718400"
        );
        // Um crash: o próximo termostato lê o estado e devolve tudo.
        let est = tempfile::tempdir().unwrap();
        s.gravar(est.path()).unwrap();
        let mut lido = Segurados::ler(est.path());
        let r = devolver_tudo(&e, &mut lido);
        assert_eq!(r.len(), 1, "{r:?}");
        assert_eq!(fs::read_to_string(d.join("cpu.max")).unwrap(), "max 100000");
        assert_eq!(fs::read_to_string(d.join("memory.high")).unwrap(), "max");
        assert!(lido.cgroups.is_empty());
    }

    #[test]
    fn observar_anuncia_uma_vez_e_devolve_com_a_calma() {
        let c = Config::default();
        let mut m = Maos::default();
        let mut simulados = Segurados::default();
        let usos = [uso(BAT, 0.8, 10.0)];
        let primeira = m.decidir(0.0, &usos, 0.5, None, &simulados, &sem_nome, 8.0, &c);
        assert_eq!(primeira.len(), 1);
        simular(&mut simulados, &primeira[0]);
        // Nas leituras seguintes, com o mesmo calor, nada de novo a anunciar.
        for t in 1..10 {
            let a = m.decidir(
                2.0 * f64::from(t),
                &usos,
                0.5,
                None,
                &simulados,
                &sem_nome,
                8.0,
                &c,
            );
            assert!(a.is_empty(), "repetiu na leitura {t}: {a:?}");
        }
        // A máquina se acalma: depois de 60 s, "devolveria", uma vez.
        let p = crate::pressao::Pressao::default();
        assert!(
            m.decidir(100.0, &[], 0.0, Some(&p), &simulados, &sem_nome, 8.0, &c)
                .is_empty()
        );
        let a = m.decidir(161.0, &[], 0.0, Some(&p), &simulados, &sem_nome, 8.0, &c);
        assert_eq!(a, [Acao::Devolver { cgroup: BAT.into() }]);
        simular(&mut simulados, &a[0]);
        assert!(simulados.cgroups.is_empty());
    }

    #[test]
    fn id_do_conteiner_so_no_escopo_do_docker() {
        assert!(id_do_conteiner(DOCKER).is_some());
        assert_eq!(id_do_conteiner(BAT), None);
        assert_eq!(id_do_conteiner("/system.slice/docker-abc.scope"), None);
    }

    #[test]
    fn fala_com_o_docker_pelo_socket() {
        use std::os::unix::net::UnixListener;
        let d = tempfile::tempdir().unwrap();
        let sock = d.path().join("docker.sock");
        let ouvinte = UnixListener::bind(&sock).unwrap();
        let t = std::thread::spawn(move || {
            let mut pedidos = Vec::new();
            for _ in 0..2 {
                let (mut c, _) = ouvinte.accept().unwrap();
                let mut buf = [0u8; 4096];
                let n = c.read(&mut buf).unwrap();
                let pedido = String::from_utf8_lossy(&buf[..n]).to_string();
                let corpo = if pedido.starts_with("GET") {
                    r#"{"Name":"/stupefied_goldberg","HostConfig":{"NanoCpus":0}}"#
                } else {
                    "{}"
                };
                write!(
                    c,
                    "HTTP/1.0 200 OK\r\nContent-Length: {}\r\n\r\n{corpo}",
                    corpo.len()
                )
                .unwrap();
                pedidos.push(pedido);
            }
            pedidos
        });
        let id = id_do_conteiner(DOCKER).unwrap();
        assert_eq!(
            docker_nome(&sock, id).as_deref(),
            Some("stupefied_goldberg")
        );
        docker_atualizar(&sock, id, 1_500_000_000).unwrap();
        let pedidos = t.join().unwrap();
        assert!(pedidos[1].starts_with(&format!("POST /containers/{id}/update")));
        assert!(pedidos[1].ends_with(r#"{"NanoCpus":1500000000}"#));
    }
}

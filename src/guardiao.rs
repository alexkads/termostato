//! O guardião: os superpoderes juntos, numa leitura.
//!
//! Antes da curva de freio, ele **vê** os processos e diz quanto os suspeitos
//! do histórico negativo costumam esquentar (a terceira fonte da previsão).
//! Depois da curva, ele **aprende** com o que aconteceu, lê a **pressão de
//! memória**, decide o que as **mãos** fazem e **avisa** o Grafana de tudo.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::avisos::{Avisador, Aviso};
use crate::config::{Config, Modo};
use crate::freio::Decisao;
use crate::historico::{self, Historico, Suspeito};
use crate::maos::{self, Acao, Executor, Maos, Segurados};
use crate::metricas::{ProcessoLido, Retrato};
use crate::pressao::{self, Pressao};
use crate::processos::{self, Observador, Visao};

/// De quanto em quanto tempo o histórico vai para o disco.
const GRAVAR_HISTORICO_S: f64 = 60.0;

pub struct Guardiao {
    proc: PathBuf,
    estado: PathBuf,
    nucleos: f64,
    observador: Observador,
    historico: Historico,
    maos: Maos,
    segurados: Segurados,
    /// No modo observar: o que estaria segurado, só em memória.
    simulados: Segurados,
    executor: Executor,
    avisador: Avisador,
    visao: Visao,
    suspeitos: Vec<Suspeito>,
    gravado_em_s: f64,
    /// Nome do contêiner por cgroup, perguntado ao Docker uma vez só.
    nomes: HashMap<String, Option<String>>,
}

fn agora_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl Guardiao {
    /// Sobe lendo o histórico e devolvendo o que ficou segurado de um crash.
    pub fn novo(cfg: &Config, avisador: Avisador) -> Self {
        let estado = PathBuf::from(&cfg.estado);
        let mut historico = Historico::ler(&estado);
        historico.esquecer(agora_unix(), cfg.processos.esquecer_dias);
        let executor = Executor::new(cfg);
        let mut segurados = Segurados::ler(&estado);
        if !segurados.cgroups.is_empty() {
            for linha in maos::devolver_tudo(&executor, &mut segurados) {
                println!("↩️ ficou segurado de antes: {linha}");
            }
            let _ = segurados.gravar(&estado);
        }
        let nucleos = std::thread::available_parallelism().map_or(1.0, |n| n.get() as f64);
        println!(
            "🧠 guardião: {} processos no histórico ({} no negativo) · mãos: {:?} · {} núcleos",
            historico.fichas.len(),
            historico
                .fichas
                .keys()
                .filter(|n| historico.eh_suspeito(n, cfg))
                .count(),
            cfg.maos.modo,
            nucleos
        );
        Self {
            proc: PathBuf::from(&cfg.proc),
            estado,
            nucleos,
            observador: Observador::default(),
            historico,
            maos: Maos::default(),
            segurados,
            simulados: Segurados::default(),
            executor,
            avisador,
            visao: Visao::default(),
            suspeitos: Vec::new(),
            gravado_em_s: 0.0,
            nomes: HashMap::new(),
        }
    }

    pub fn avisador(&self) -> &Avisador {
        &self.avisador
    }

    /// Antes da curva: vê os processos e devolve a temperatura prevista pelos
    /// suspeitos ativos, se houver.
    pub fn antes_do_freio(&mut self, agora_s: f64, temperatura: f64, cfg: &Config) -> Option<f64> {
        let amostras = processos::ler_todos(&self.proc);
        self.visao = self.observador.observar(agora_s, &amostras, self.nucleos);
        if !cfg.processos.antecipar {
            self.suspeitos.clear();
            return None;
        }
        self.suspeitos = self.historico.suspeitos_ativos(&self.visao.por_nome, cfg);
        historico::prevista_por_processos(temperatura, &self.suspeitos, cfg)
    }

    fn nome_do_conteiner(&mut self, cgroup: &str, cfg: &Config) -> Option<String> {
        let id = maos::id_do_conteiner(cgroup)?;
        self.nomes
            .entry(cgroup.to_string())
            .or_insert_with(|| maos::docker_nome(std::path::Path::new(&cfg.docker_socket), id))
            .clone()
    }

    /// Um nome legível para um cgroup: o contêiner, o serviço ou o escopo.
    fn rotulo(&mut self, cgroup: &str, cfg: &Config) -> String {
        if let Some(n) = self.nome_do_conteiner(cgroup, cfg) {
            return format!("contêiner {n}");
        }
        cgroup.rsplit('/').next().unwrap_or(cgroup).to_string()
    }

    /// Depois da curva: aprende, lê a memória, decide as mãos e avisa.
    pub fn depois_do_freio(
        &mut self,
        agora_s: f64,
        temperatura: f64,
        d: &Decisao,
        cfg: &Config,
        retrato: &mut Retrato,
    ) {
        if d.antecipado
            && d.mudanca == Some(crate::freio::Mudanca::Acionou)
            && let Some(s) = self.suspeitos.first()
        {
            let texto = format!(
                "🧠 {} ({} cópias, {:.0} % da CPU) tem histórico de {:+.1} °C/s: freio antecipado em {temperatura:.0} °C, PL1 em {:.1} W",
                s.nome,
                s.copias,
                s.fatia * 100.0,
                s.nota_c_s,
                d.pl1_w
            );
            println!("{texto}");
            self.avisador
                .avisar(Aviso::novo(texto, &["freio", "antecipado", &s.nome]));
            retrato.contar("freio_antecipado");
        }

        if cfg.processos.aprender {
            for nome in self.historico.aprender(
                &self.visao.por_nome,
                temperatura,
                d.tendencia_c_s,
                agora_unix(),
                cfg,
            ) {
                let f = &self.historico.fichas[&nome];
                let texto = format!(
                    "📒 {nome} entrou no histórico negativo ({:+.1} °C/s em {} observações)",
                    f.nota_c_s, f.observacoes
                );
                println!("{texto}");
                self.avisador
                    .avisar(Aviso::novo(texto, &["historico", &nome]));
                retrato.contar("historico_negativo");
            }
            if agora_s - self.gravado_em_s >= GRAVAR_HISTORICO_S {
                if let Err(e) = self.historico.gravar(&self.estado) {
                    eprintln!("⚠️ histórico: {e}");
                }
                self.gravado_em_s = agora_s;
            }
        }

        let pressao = pressao::ler(&self.proc);
        self.mexer(agora_s, d.intensidade, pressao.as_ref(), cfg, retrato);
        self.retratar(pressao, cfg, retrato);
    }

    fn mexer(
        &mut self,
        agora_s: f64,
        intensidade: f64,
        pressao: Option<&Pressao>,
        cfg: &Config,
        retrato: &mut Retrato,
    ) {
        let cgroups = self.visao.por_cgroup.clone();
        // Pergunta ao Docker o nome dos contêineres que importam agora.
        let mut nomes = HashMap::new();
        for u in cgroups.iter().take(8) {
            if let Some(n) = self.nome_do_conteiner(&u.chave, cfg) {
                nomes.insert(u.chave.clone(), n);
            }
        }
        let olhar = |c: &str| nomes.get(c).cloned();
        let estado_das_maos = if cfg.maos.modo == Modo::Agir {
            &self.segurados
        } else {
            &self.simulados
        };
        let acoes = self.maos.decidir(
            agora_s,
            &cgroups,
            intensidade,
            pressao,
            estado_das_maos,
            &olhar,
            self.nucleos,
            cfg,
        );
        for acao in acoes {
            let (cgroup, tipo, o_que) = match &acao {
                Acao::SegurarCpu {
                    cgroup,
                    cpus,
                    fatia,
                } => (
                    cgroup.clone(),
                    "segurar_cpu",
                    format!("CPU de {:.1} para {cpus:.1} núcleos", fatia * self.nucleos),
                ),
                Acao::SegurarMemoria { cgroup, rss_mb } => (
                    cgroup.clone(),
                    "segurar_memoria",
                    format!("memória ({rss_mb:.0} MB) em 90 % do que usa"),
                ),
                Acao::Devolver { cgroup } => (
                    cgroup.clone(),
                    "devolver",
                    "limites de volta ao original".to_string(),
                ),
            };
            let rotulo = self.rotulo(&cgroup, cfg);
            let (texto, tags): (String, Vec<&str>) = if cfg.maos.modo == Modo::Agir {
                match self.executor.aplicar(&acao, &mut self.segurados) {
                    Ok(feito) => {
                        let _ = self.segurados.gravar(&self.estado);
                        (
                            format!("🤚 {rotulo}: {o_que} ({feito})"),
                            vec![tipo, "agir"],
                        )
                    }
                    Err(e) => (
                        format!("⚠️ {rotulo}: não consegui {o_que}: {e}"),
                        vec![tipo, "erro"],
                    ),
                }
            } else {
                maos::simular(&mut self.simulados, &acao);
                let verbo = if matches!(acao, Acao::Devolver { .. }) {
                    "devolveria"
                } else {
                    "seguraria"
                };
                (
                    format!("👀 (observar) {verbo} {rotulo}: {o_que}"),
                    vec![tipo, "observar"],
                )
            };
            println!("{texto}");
            let mut todas = tags.clone();
            todas.push(&rotulo);
            self.avisador.avisar(Aviso::novo(texto, &todas));
            // O que só foi observado conta à parte: o alerta por e-mail olha só o
            // que foi feito de verdade.
            if cfg.maos.modo == Modo::Agir {
                retrato.contar(tipo);
            } else {
                retrato.contar(&format!("{tipo}_observado"));
            }
        }
    }

    fn retratar(&mut self, pressao: Option<Pressao>, cfg: &Config, retrato: &mut Retrato) {
        retrato.pressao = pressao;
        retrato.processos = self
            .visao
            .por_nome
            .iter()
            .take(5)
            .map(|u| ProcessoLido {
                nome: u.chave.clone(),
                fatia: u.fatia,
                copias: u.copias,
                rss_mb: u.rss_mb,
            })
            .collect();
        retrato.suspeitos = self
            .historico
            .fichas
            .iter()
            .filter(|(n, _)| self.historico.eh_suspeito(n, cfg))
            .map(|(n, f)| (n.clone(), f.nota_c_s))
            .collect();
        retrato.segurados = self.segurados.cgroups.len();
        retrato.freio_antecipado = !self.suspeitos.is_empty();
    }

    /// Ao parar: grava o histórico e devolve tudo o que está segurado.
    /// O modo saiu de agir (um ajuste): devolve o que estava segurado.
    pub fn soltar_as_maos(&mut self) {
        for linha in maos::devolver_tudo(&self.executor, &mut self.segurados) {
            println!("↩️ {linha}");
        }
        let _ = self.segurados.gravar(&self.estado);
        self.simulados = Segurados::default();
    }

    pub fn parar(&mut self) {
        let _ = self.historico.gravar(&self.estado);
        for linha in maos::devolver_tudo(&self.executor, &mut self.segurados) {
            println!("↩️ {linha}");
        }
        let _ = self.segurados.gravar(&self.estado);
    }
}

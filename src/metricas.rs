//! O retrato de cada leitura e o texto que o Prometheus lê em `/metrics`.
//!
//! O formato de exposição do Prometheus é texto simples, e escrito aqui ele
//! dispensa uma dependência inteira para um punhado de linhas.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::pressao::Pressao;

/// Um dos processos que mais gastam CPU agora.
#[derive(Debug, Clone, Default)]
pub struct ProcessoLido {
    pub nome: String,
    pub fatia: f64,
    pub copias: u32,
    pub rss_mb: f64,
}

#[derive(Debug, Clone, Default)]
pub struct VentoinhaLida {
    pub numero: String,
    pub rpm: Option<u64>,
    pub piso: Option<u64>,
    pub maximo: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct Retrato {
    pub temperatura_c: Option<f64>,
    pub potencia_w: Option<f64>,
    pub pl1_w: Option<f64>,
    pub pl2_w: Option<f64>,
    pub pl1_alvo_w: f64,
    pub freio_acionado: bool,
    pub temperatura_prevista_c: Option<f64>,
    pub tendencia_c_s: Option<f64>,
    /// 0 = limite normal; 1 = freio no máximo.
    pub freio_intensidade: f64,
    pub acionamentos: u64,
    pub segundos_acionado: f64,
    pub frequencia_mhz: Option<f64>,
    pub ventoinhas: Vec<VentoinhaLida>,
    pub falhas_de_leitura: u64,
    pub leituras: u64,
    pub pressao: Option<Pressao>,
    /// Os 5 nomes que mais gastam CPU agora.
    pub processos: Vec<ProcessoLido>,
    /// O histórico negativo inteiro: nome e nota.
    pub suspeitos: Vec<(String, f64)>,
    /// Cgroups segurados agora (só no modo agir).
    pub segurados: usize,
    /// Há suspeito gastando CPU agora.
    pub freio_antecipado: bool,
    /// Quantas vezes cada ação aconteceu.
    pub acoes: BTreeMap<String, u64>,
}

/// Escapa um valor de rótulo do Prometheus.
fn rotulo(v: &str) -> String {
    v.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
}

fn metrica(saida: &mut String, nome: &str, tipo: &str, ajuda: &str) {
    let _ = writeln!(saida, "# HELP {nome} {ajuda}");
    let _ = writeln!(saida, "# TYPE {nome} {tipo}");
}

fn valor(saida: &mut String, nome: &str, rotulos: &str, v: f64) {
    let _ = writeln!(saida, "{nome}{rotulos} {v}");
}

impl Retrato {
    pub fn contar(&mut self, acao: &str) {
        *self.acoes.entry(acao.to_string()).or_default() += 1;
    }

    // Outra lista corrida, como a de `em_texto`.
    #[allow(clippy::too_many_lines)]
    fn superpoderes(&self, s: &mut String) {
        if let Some(p) = &self.pressao {
            metrica(
                s,
                "termostato_pressao_pct",
                "gauge",
                "Pressão (PSI avg10, %): tempo com processos parados esperando o recurso.",
            );
            for (recurso, tipo, v) in [
                ("memoria", "some", p.memoria_some_pct),
                ("memoria", "full", p.memoria_full_pct),
                ("cpu", "some", p.cpu_some_pct),
                ("io", "some", p.io_some_pct),
            ] {
                valor(
                    s,
                    "termostato_pressao_pct",
                    &format!("{{recurso=\"{recurso}\",tipo=\"{tipo}\"}}"),
                    v,
                );
            }
            metrica(
                s,
                "termostato_memoria_disponivel_pct",
                "gauge",
                "MemAvailable sobre MemTotal, em %.",
            );
            valor(
                s,
                "termostato_memoria_disponivel_pct",
                "",
                p.memoria_disponivel_pct,
            );
            metrica(
                s,
                "termostato_swap_usado_mb",
                "gauge",
                "Swap em uso, em MB.",
            );
            valor(s, "termostato_swap_usado_mb", "", p.swap_usado_mb);
        }
        if !self.processos.is_empty() {
            metrica(
                s,
                "termostato_processo_fatia",
                "gauge",
                "Fatia da CPU da máquina (0 a 1) dos 5 nomes que mais gastam agora.",
            );
            for p in &self.processos {
                valor(
                    s,
                    "termostato_processo_fatia",
                    &format!("{{processo=\"{}\"}}", rotulo(&p.nome)),
                    p.fatia,
                );
            }
            metrica(
                s,
                "termostato_processo_copias",
                "gauge",
                "Quantos processos de cada um dos 5 nomes que mais gastam CPU.",
            );
            for p in &self.processos {
                valor(
                    s,
                    "termostato_processo_copias",
                    &format!("{{processo=\"{}\"}}", rotulo(&p.nome)),
                    f64::from(p.copias),
                );
            }
            metrica(
                s,
                "termostato_processo_memoria_mb",
                "gauge",
                "Memória residente somada dos 5 nomes que mais gastam CPU.",
            );
            for p in &self.processos {
                valor(
                    s,
                    "termostato_processo_memoria_mb",
                    &format!("{{processo=\"{}\"}}", rotulo(&p.nome)),
                    p.rss_mb,
                );
            }
        }
        metrica(
            s,
            "termostato_processo_nota_c_s",
            "gauge",
            "Nota de calor (°C/s) de cada nome do histórico negativo.",
        );
        for (nome, nota) in &self.suspeitos {
            valor(
                s,
                "termostato_processo_nota_c_s",
                &format!("{{processo=\"{}\"}}", rotulo(nome)),
                *nota,
            );
        }
        metrica(
            s,
            "termostato_freio_antecipado",
            "gauge",
            "1 quando um suspeito do histórico negativo está gastando CPU.",
        );
        valor(
            s,
            "termostato_freio_antecipado",
            "",
            if self.freio_antecipado { 1.0 } else { 0.0 },
        );
        metrica(
            s,
            "termostato_segurados",
            "gauge",
            "Cgroups com CPU ou memória segurados agora.",
        );
        valor(s, "termostato_segurados", "", self.segurados as f64);
        metrica(
            s,
            "termostato_acoes_total",
            "counter",
            "Ações do termostato, por tipo (inclui as do modo observar).",
        );
        for (acao, n) in &self.acoes {
            valor(
                s,
                "termostato_acoes_total",
                &format!("{{acao=\"{}\"}}", rotulo(acao)),
                *n as f64,
            );
        }
    }

    // Uma lista corrida de métricas: dividir só espalharia a ordem do texto.
    #[allow(clippy::too_many_lines)]
    pub fn em_texto(&self) -> String {
        let mut s = String::new();
        let versao = env!("CARGO_PKG_VERSION");

        metrica(&mut s, "termostato_info", "gauge", "Versão do termostato.");
        valor(
            &mut s,
            "termostato_info",
            &format!("{{versao=\"{versao}\"}}"),
            1.0,
        );

        if let Some(t) = self.temperatura_c {
            metrica(
                &mut s,
                "termostato_temperatura_celsius",
                "gauge",
                "Temperatura do pacote da CPU (coretemp temp1).",
            );
            valor(&mut s, "termostato_temperatura_celsius", "", t);
        }
        if let Some(t) = self.temperatura_prevista_c {
            metrica(
                &mut s,
                "termostato_temperatura_prevista_celsius",
                "gauge",
                "Temperatura prevista para daqui a previsao_s, pela reta das últimas leituras.",
            );
            valor(&mut s, "termostato_temperatura_prevista_celsius", "", t);
        }
        if let Some(t) = self.tendencia_c_s {
            metrica(
                &mut s,
                "termostato_tendencia_celsius_por_segundo",
                "gauge",
                "Inclinação da temperatura nas últimas leituras (negativa esfriando).",
            );
            valor(&mut s, "termostato_tendencia_celsius_por_segundo", "", t);
        }
        metrica(
            &mut s,
            "termostato_freio_intensidade",
            "gauge",
            "0 = limite normal; 1 = freio de calor no máximo (pl1_freio_w).",
        );
        valor(
            &mut s,
            "termostato_freio_intensidade",
            "",
            self.freio_intensidade,
        );
        if let Some(p) = self.potencia_w {
            metrica(
                &mut s,
                "termostato_potencia_watts",
                "gauge",
                "Potência média do pacote desde a leitura anterior (RAPL energy_uj).",
            );
            valor(&mut s, "termostato_potencia_watts", "", p);
        }

        metrica(
            &mut s,
            "termostato_limite_watts",
            "gauge",
            "Limites de potência do pacote: pl1 (sustentado), pl2 (curto) e o pl1 que o termostato quer.",
        );
        if let Some(v) = self.pl1_w {
            valor(&mut s, "termostato_limite_watts", "{limite=\"pl1\"}", v);
        }
        if let Some(v) = self.pl2_w {
            valor(&mut s, "termostato_limite_watts", "{limite=\"pl2\"}", v);
        }
        valor(
            &mut s,
            "termostato_limite_watts",
            "{limite=\"pl1_alvo\"}",
            self.pl1_alvo_w,
        );

        metrica(
            &mut s,
            "termostato_freio_acionado",
            "gauge",
            "1 enquanto o freio de calor segura o PL1 no valor de freio.",
        );
        valor(
            &mut s,
            "termostato_freio_acionado",
            "",
            if self.freio_acionado { 1.0 } else { 0.0 },
        );

        metrica(
            &mut s,
            "termostato_freio_acionamentos_total",
            "counter",
            "Vezes que o freio de calor foi acionado.",
        );
        valor(
            &mut s,
            "termostato_freio_acionamentos_total",
            "",
            self.acionamentos as f64,
        );

        metrica(
            &mut s,
            "termostato_freio_acionado_segundos_total",
            "counter",
            "Tempo com o freio acionado.",
        );
        valor(
            &mut s,
            "termostato_freio_acionado_segundos_total",
            "",
            self.segundos_acionado,
        );

        if let Some(f) = self.frequencia_mhz {
            metrica(
                &mut s,
                "termostato_frequencia_mhz",
                "gauge",
                "Frequência média dos núcleos.",
            );
            valor(&mut s, "termostato_frequencia_mhz", "", f);
        }

        if !self.ventoinhas.is_empty() {
            metrica(
                &mut s,
                "termostato_ventoinha_rpm",
                "gauge",
                "Rotação de cada ventoinha (applesmc): atual, piso e máximo.",
            );
            for v in &self.ventoinhas {
                for (medida, lido) in [("atual", v.rpm), ("piso", v.piso), ("maximo", v.maximo)] {
                    if let Some(rpm) = lido {
                        let rotulos = format!("{{ventoinha=\"{}\",medida=\"{medida}\"}}", v.numero);
                        valor(&mut s, "termostato_ventoinha_rpm", &rotulos, rpm as f64);
                    }
                }
            }
        }

        self.superpoderes(&mut s);

        metrica(
            &mut s,
            "termostato_leituras_total",
            "counter",
            "Leituras do hardware.",
        );
        valor(
            &mut s,
            "termostato_leituras_total",
            "",
            self.leituras as f64,
        );
        metrica(
            &mut s,
            "termostato_falhas_de_leitura_total",
            "counter",
            "Leituras ou escritas do sysfs que falharam.",
        );
        valor(
            &mut s,
            "termostato_falhas_de_leitura_total",
            "",
            self.falhas_de_leitura as f64,
        );
        s
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn texto_tem_o_que_o_painel_usa() {
        let r = Retrato {
            temperatura_c: Some(83.0),
            potencia_w: Some(37.5),
            pl1_w: Some(38.0),
            pl2_w: Some(125.0),
            pl1_alvo_w: 38.0,
            freio_acionado: true,
            acionamentos: 2,
            ventoinhas: vec![VentoinhaLida {
                numero: "1".into(),
                rpm: Some(6156),
                piso: Some(6156),
                maximo: Some(6156),
            }],
            ..Retrato::default()
        };
        let t = r.em_texto();
        for linha in [
            "termostato_temperatura_celsius 83",
            "termostato_potencia_watts 37.5",
            "termostato_limite_watts{limite=\"pl1\"} 38",
            "termostato_limite_watts{limite=\"pl2\"} 125",
            "termostato_freio_acionado 1",
            "termostato_freio_acionamentos_total 2",
            "termostato_ventoinha_rpm{ventoinha=\"1\",medida=\"piso\"} 6156",
        ] {
            assert!(t.lines().any(|l| l == linha), "falta {linha:?} em\n{t}");
        }
    }

    #[test]
    fn sem_leitura_a_metrica_some_em_vez_de_mentir_zero() {
        let t = Retrato::default().em_texto();
        assert!(!t.contains("termostato_temperatura_celsius "));
        assert!(t.contains("termostato_freio_acionado 0"));
    }
}

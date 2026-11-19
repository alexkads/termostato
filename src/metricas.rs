//! O retrato de cada leitura e o texto que o Prometheus lê em `/metrics`.
//!
//! O formato de exposição do Prometheus é texto simples, e escrito aqui ele
//! dispensa uma dependência inteira para um punhado de linhas.

use std::fmt::Write;

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
    pub freio_puxado: bool,
    pub freadas: u64,
    pub segundos_freado: f64,
    pub frequencia_mhz: Option<f64>,
    pub ventoinhas: Vec<VentoinhaLida>,
    pub falhas_de_leitura: u64,
    pub leituras: u64,
}

fn metrica(saida: &mut String, nome: &str, tipo: &str, ajuda: &str) {
    let _ = writeln!(saida, "# HELP {nome} {ajuda}");
    let _ = writeln!(saida, "# TYPE {nome} {tipo}");
}

fn valor(saida: &mut String, nome: &str, rotulos: &str, v: f64) {
    let _ = writeln!(saida, "{nome}{rotulos} {v}");
}

impl Retrato {
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
            "termostato_freio_puxado",
            "gauge",
            "1 enquanto o freio de calor segura o PL1 no valor de freio.",
        );
        valor(
            &mut s,
            "termostato_freio_puxado",
            "",
            if self.freio_puxado { 1.0 } else { 0.0 },
        );

        metrica(
            &mut s,
            "termostato_freadas_total",
            "counter",
            "Vezes que o freio de calor puxou.",
        );
        valor(&mut s, "termostato_freadas_total", "", self.freadas as f64);

        metrica(
            &mut s,
            "termostato_freado_segundos_total",
            "counter",
            "Tempo com o freio puxado.",
        );
        valor(
            &mut s,
            "termostato_freado_segundos_total",
            "",
            self.segundos_freado,
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
            freio_puxado: true,
            freadas: 2,
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
            "termostato_freio_puxado 1",
            "termostato_freadas_total 2",
            "termostato_ventoinha_rpm{ventoinha=\"1\",medida=\"piso\"} 6156",
        ] {
            assert!(t.lines().any(|l| l == linha), "falta {linha:?} em\n{t}");
        }
    }

    #[test]
    fn sem_leitura_a_metrica_some_em_vez_de_mentir_zero() {
        let t = Retrato::default().em_texto();
        assert!(!t.contains("termostato_temperatura_celsius "));
        assert!(t.contains("termostato_freio_puxado 0"));
    }
}

//! O retrato de cada leitura e o texto que o Prometheus lê em `/metrics`.
//!
//! O formato de exposição do Prometheus é texto simples, e escrito aqui ele
//! dispensa uma dependência inteira para um punhado de linhas.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::pressao::Pressao;
use crate::vigia::{Bateria, MemoriaDe, Motivos};

/// Um dos processos que mais gastam CPU agora.
#[derive(Debug, Clone, Default)]
pub struct ProcessoLido {
    pub nome: String,
    pub fatia: f64,
    pub copias: u32,
    pub rss_mb: f64,
}

/// Uma ficha do histórico negativo, para o painel.
#[derive(Debug, Clone, Default)]
pub struct FichaLida {
    pub nome: String,
    pub nota_c_s: f64,
    pub maior_tendencia_c_s: f64,
    pub observacoes: u32,
    pub visto_em: u64,
    pub suspeito: bool,
}

/// O que cada regra pediu na última leitura, para o fluxo da decisão do painel.
#[derive(Debug, Clone, Default)]
pub struct DecisaoLida {
    pub pela_curva_w: f64,
    pub do_alvo_w: Option<f64>,
    pub prevista_processos_c: Option<f64>,
    pub antecipado: bool,
    pub pelo_alvo: bool,
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
    /// O alvo de temperatura em vigor (0 = sem alvo).
    pub alvo_c: f64,
    /// Quantos ajustes (`termostato chave=valor`) estão em vigor.
    pub ajustes: usize,
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
    /// O TCC offset em vigor no chip e o TjMax.
    pub tcc_offset_c: Option<u8>,
    pub tjmax_c: Option<u8>,
    /// Quantas vezes o gatilho de pressão do kernel acordou o termostato.
    pub kernel_avisos: u64,
    /// Estrangulamento térmico desde o boot: (pacote, soma dos núcleos).
    pub estrangulamento: Option<(u64, u64)>,
    /// Potência dos domínios do RAPL além do pacote (core, uncore, dram).
    pub dominios_w: Vec<(String, f64)>,
    /// Os sensores do applesmc.
    pub sensores_do_mac: Vec<(String, f64)>,
    /// O histórico negativo inteiro, com o que o painel mostra.
    pub fichas: Vec<FichaLida>,
    pub decisao: DecisaoLida,
    /// A temperatura de cada núcleo físico: (núcleo, °C).
    pub nucleos_c: Vec<(u32, f64)>,
    /// Estrangulamentos de cada núcleo desde o boot: (núcleo, vezes).
    pub nucleos_estrangulamento: Vec<(u32, u64)>,
    /// Tempo total com o pacote estrangulado desde o boot.
    pub estrangulado_s: Option<f64>,
    /// Por que o chip está abaixo da frequência pedida (MSR 0x690).
    pub motivos: Option<Motivos>,
    pub bateria: Option<Bateria>,
    /// Swap em MB/s: (entra, sai).
    pub swap_mb_s: Option<(f64, f64)>,
    /// Mortes do OOM killer desde o boot.
    pub oom_mortes: Option<u64>,
    /// Quem mais segura memória (RAM + swap), por nome.
    pub memoria_maiores: Vec<MemoriaDe>,
    /// O chip já passou do corte do TCC: o offset não está sendo respeitado.
    pub tcc_ignorado: bool,
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

    /// Onde o chip se freia de verdade: o TjMax menos o offset, ou o TjMax se
    /// ele já passou do corte (em 22/11/2026, o Haswell da fedora ia a 100 °C
    /// com o offset de 8 °C gravado).
    pub fn corte_c(&self) -> Option<u8> {
        let (o, tj) = (self.tcc_offset_c?, self.tjmax_c?);
        Some(if self.tcc_ignorado {
            tj
        } else {
            tj.saturating_sub(o)
        })
    }

    // Uma lista corrida de métricas, como a de `em_texto`.
    #[allow(clippy::too_many_lines)]
    fn do_vigia(&self, s: &mut String) {
        if !self.nucleos_c.is_empty() {
            metrica(
                s,
                "termostato_nucleo_celsius",
                "gauge",
                "Temperatura de cada núcleo físico (coretemp).",
            );
            for (n, c) in &self.nucleos_c {
                valor(
                    s,
                    "termostato_nucleo_celsius",
                    &format!("{{nucleo=\"{n}\"}}"),
                    *c,
                );
            }
        }
        if !self.nucleos_estrangulamento.is_empty() {
            metrica(
                s,
                "termostato_nucleo_estrangulamento_total",
                "counter",
                "Estrangulamentos térmicos de cada núcleo físico desde o boot.",
            );
            for (n, v) in &self.nucleos_estrangulamento {
                valor(
                    s,
                    "termostato_nucleo_estrangulamento_total",
                    &format!("{{nucleo=\"{n}\"}}"),
                    *v as f64,
                );
            }
        }
        if let Some(t) = self.estrangulado_s {
            metrica(
                s,
                "termostato_estrangulado_segundos_total",
                "counter",
                "Tempo com o pacote estrangulado desde o boot.",
            );
            valor(s, "termostato_estrangulado_segundos_total", "", t);
        }
        if let Some(m) = &self.motivos {
            metrica(
                s,
                "termostato_freio_motivo",
                "gauge",
                "Por que o chip está abaixo da frequência pedida (MSR 0x690): 1 se o motivo vale agora ou valeu desde o boot. prochot = o SMC ou o carregador freando por fora.",
            );
            for (nome, agora, desde) in &m.lista {
                for (quando, v) in [("agora", agora), ("desde_o_boot", desde)] {
                    valor(
                        s,
                        "termostato_freio_motivo",
                        &format!("{{motivo=\"{nome}\",quando=\"{quando}\"}}"),
                        f64::from(u8::from(*v)),
                    );
                }
            }
        }
        if let Some(b) = &self.bateria {
            if let Some(a) = b.amperes {
                metrica(
                    s,
                    "termostato_bateria_amperes",
                    "gauge",
                    "Corrente da bateria: positiva carregando, negativa descarregando.",
                );
                valor(s, "termostato_bateria_amperes", "", a);
            }
            if let Some(p) = b.carga_pct {
                metrica(
                    s,
                    "termostato_bateria_carga_pct",
                    "gauge",
                    "Carga da bateria, em %.",
                );
                valor(s, "termostato_bateria_carga_pct", "", p);
            }
            if let Some(l) = b.carregador_ligado {
                metrica(
                    s,
                    "termostato_carregador_ligado",
                    "gauge",
                    "1 com o carregador ligado.",
                );
                valor(
                    s,
                    "termostato_carregador_ligado",
                    "",
                    f64::from(u8::from(l)),
                );
            }
        }
        if let Some((entra, sai)) = self.swap_mb_s {
            metrica(
                s,
                "termostato_swap_mb_por_segundo",
                "gauge",
                "Swap trocado desde a leitura anterior, em MB/s.",
            );
            valor(
                s,
                "termostato_swap_mb_por_segundo",
                "{sentido=\"entra\"}",
                entra,
            );
            valor(
                s,
                "termostato_swap_mb_por_segundo",
                "{sentido=\"sai\"}",
                sai,
            );
        }
        if let Some(n) = self.oom_mortes {
            metrica(
                s,
                "termostato_oom_mortes_total",
                "counter",
                "Processos mortos pelo OOM killer desde o boot (/proc/vmstat oom_kill).",
            );
            valor(s, "termostato_oom_mortes_total", "", n as f64);
        }
        if !self.memoria_maiores.is_empty() {
            metrica(
                s,
                "termostato_memoria_mb",
                "gauge",
                "Quem mais segura memória, somado por nome: RAM (ram) e swap (swap), em MB.",
            );
            for m in &self.memoria_maiores {
                let n = rotulo(&m.nome);
                valor(
                    s,
                    "termostato_memoria_mb",
                    &format!("{{processo=\"{n}\",tipo=\"ram\"}}"),
                    m.rss_mb,
                );
                valor(
                    s,
                    "termostato_memoria_mb",
                    &format!("{{processo=\"{n}\",tipo=\"swap\"}}"),
                    m.swap_mb,
                );
            }
        }
    }

    fn do_kernel(&self, s: &mut String) {
        if let Some(corte) = self.corte_c() {
            metrica(
                s,
                "termostato_tcc_corte_celsius",
                "gauge",
                "Temperatura em que o próprio chip se freia: TjMax menos o TCC offset, ou o TjMax se o chip já passou do corte.",
            );
            valor(s, "termostato_tcc_corte_celsius", "", f64::from(corte));
            metrica(
                s,
                "termostato_tcc_respeitado",
                "gauge",
                "0 se o chip já passou do corte do TCC offset (o offset gravado não segura).",
            );
            valor(
                s,
                "termostato_tcc_respeitado",
                "",
                f64::from(u8::from(!self.tcc_ignorado)),
            );
            metrica(
                s,
                "termostato_tcc_offset_celsius",
                "gauge",
                "TCC offset gravado no chip (MSR 0x1A2).",
            );
            valor(
                s,
                "termostato_tcc_offset_celsius",
                "",
                f64::from(self.tcc_offset_c.unwrap_or(0)),
            );
        }
        metrica(
            s,
            "termostato_kernel_avisos_total",
            "counter",
            "Vezes que o gatilho de pressão do kernel acordou o termostato.",
        );
        valor(
            s,
            "termostato_kernel_avisos_total",
            "",
            self.kernel_avisos as f64,
        );
        if let Some((pacote, nucleos)) = self.estrangulamento {
            metrica(
                s,
                "termostato_estrangulamento_total",
                "counter",
                "Estrangulamentos térmicos do chip desde o boot, pelo kernel.",
            );
            valor(
                s,
                "termostato_estrangulamento_total",
                "{onde=\"pacote\"}",
                pacote as f64,
            );
            valor(
                s,
                "termostato_estrangulamento_total",
                "{onde=\"nucleos\"}",
                nucleos as f64,
            );
        }
        if !self.dominios_w.is_empty() {
            metrica(
                s,
                "termostato_dominio_watts",
                "gauge",
                "Potência de cada domínio do RAPL: core (núcleos), uncore (gráfico e resto do chip), dram (memória).",
            );
            for (d, w) in &self.dominios_w {
                valor(
                    s,
                    "termostato_dominio_watts",
                    &format!("{{dominio=\"{}\"}}", rotulo(d)),
                    *w,
                );
            }
        }
        if !self.sensores_do_mac.is_empty() {
            metrica(
                s,
                "termostato_sensor_celsius",
                "gauge",
                "Sensores de temperatura do applesmc, pela chave do SMC (TC0P = CPU, TG0P = GPU, Ts0P = apoio de mãos, TH0* = aletas, TB*T = bateria).",
            );
            for (n, c) in &self.sensores_do_mac {
                valor(
                    s,
                    "termostato_sensor_celsius",
                    &format!("{{sensor=\"{}\"}}", rotulo(n)),
                    *c,
                );
            }
        }
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
        self.do_kernel(s);
        self.do_vigia(s);
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
            "termostato_alvo_celsius",
            "gauge",
            "Alvo de temperatura em vigor (termostato CPU50C); 0 = sem alvo, só a curva.",
        );
        valor(&mut s, "termostato_alvo_celsius", "", self.alvo_c);
        metrica(
            &mut s,
            "termostato_ajustes",
            "gauge",
            "Ajustes (termostato chave=valor) em vigor por cima do /etc/termostato.toml.",
        );
        valor(&mut s, "termostato_ajustes", "", self.ajustes as f64);

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
    fn texto_tem_o_que_o_vigia_le() {
        let mut r = Retrato {
            tcc_offset_c: Some(8),
            tjmax_c: Some(100),
            nucleos_c: vec![(0, 97.0), (3, 92.0)],
            nucleos_estrangulamento: vec![(2, 9719)],
            estrangulado_s: Some(16.4),
            motivos: Some(Motivos::decodificar(0x3d22_1000)),
            bateria: Some(Bateria {
                carregador_ligado: Some(true),
                estado: "Discharging".into(),
                amperes: Some(-1.312),
                carga_pct: Some(90.0),
            }),
            swap_mb_s: Some((41.0, 2.5)),
            oom_mortes: Some(4),
            memoria_maiores: vec![MemoriaDe {
                nome: "rustc".into(),
                copias: 8,
                rss_mb: 12288.0,
                swap_mb: 11264.0,
            }],
            ..Retrato::default()
        };
        let t = r.em_texto();
        for linha in [
            "termostato_tcc_corte_celsius 92",
            "termostato_tcc_respeitado 1",
            "termostato_nucleo_celsius{nucleo=\"3\"} 92",
            "termostato_nucleo_estrangulamento_total{nucleo=\"2\"} 9719",
            "termostato_estrangulado_segundos_total 16.4",
            "termostato_freio_motivo{motivo=\"termico\",quando=\"desde_o_boot\"} 1",
            "termostato_freio_motivo{motivo=\"prochot\",quando=\"desde_o_boot\"} 0",
            "termostato_freio_motivo{motivo=\"turbo_maximo\",quando=\"agora\"} 1",
            "termostato_bateria_amperes -1.312",
            "termostato_carregador_ligado 1",
            "termostato_swap_mb_por_segundo{sentido=\"entra\"} 41",
            "termostato_oom_mortes_total 4",
            "termostato_memoria_mb{processo=\"rustc\",tipo=\"swap\"} 11264",
        ] {
            assert!(t.contains(linha), "faltou {linha}");
        }
        // O chip passou do corte: o corte de verdade é o TjMax.
        r.tcc_ignorado = true;
        let t = r.em_texto();
        assert!(t.contains("termostato_tcc_corte_celsius 100"));
        assert!(t.contains("termostato_tcc_respeitado 0"));
    }

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

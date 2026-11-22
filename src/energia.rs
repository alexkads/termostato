//! A energia: cada PROCHOT e cada descarga da bateria no carregador, com
//! registro.
//!
//! Em 22/11/2026, o carregador de 85 W da fedora foi trocado por um mais fraco.
//! Em 3 h, a caixa-preta viu dois efeitos:
//! - a bateria descarregando em 71 % das leituras com a CPU acima de 30 W;
//! - um PROCHOT externo: o SMC freou a CPU para 798 MHz por 2 s, porque a
//!   energia não deu conta.
//!
//! Este módulo transforma isso em eventos (⛔ PROCHOT, 🔌 carregador, 🔋 bateria)
//! e em acumulados (segundos, Wh, episódios). Ele é puro: recebe a leitura da
//! volta e devolve os avisos a emitir, e o `kernel.rs` os escreve no journal
//! e no Grafana.

use crate::vigia::{Bateria, Motivos};

/// No máximo uma anotação de PROCHOT por minuto no Grafana. Todos contam.
const PROCHOT_AVISO_S: f64 = 60.0;
/// A carga abaixo disso com o carregador ligado vira aviso; rearma acima de
/// `BATERIA_REARMA_PCT`.
const BATERIA_BAIXA_PCT: f64 = 80.0;
const BATERIA_REARMA_PCT: f64 = 85.0;
/// A bateria descarregando com o carregador ligado por tanto tempo seguido
/// vira aviso.
const DESCARGA_AVISO_S: f64 = 300.0;

/// Um aviso a emitir: o texto e as tags do Grafana.
#[derive(Debug, Clone, PartialEq)]
pub struct Aviso {
    pub texto: String,
    pub tags: &'static [&'static str],
    /// Só no journal, sem anotação (o PROCHOT repetido dentro do minuto).
    pub so_no_journal: bool,
}

impl Aviso {
    fn novo(texto: String, tags: &'static [&'static str]) -> Self {
        Self {
            texto,
            tags,
            so_no_journal: false,
        }
    }
}

/// O que acompanha cada aviso: como estava a máquina.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Contexto {
    pub mhz: Option<f64>,
    pub chip_c: Option<f64>,
    pub cpu_w: Option<f64>,
}

/// Os acumulados, para as métricas.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EnergiaLida {
    /// Leituras em que cada motivo apareceu (agora ou no log).
    pub motivo_leituras: Vec<(&'static str, u64)>,
    pub prochot_eventos: u64,
    pub prochot_s: f64,
    pub descarga_s: f64,
    pub descarga_wh: f64,
    pub descarga_episodios: u64,
}

#[derive(Debug, Default)]
pub struct Energia {
    lida: EnergiaLida,
    prochot_desde: Option<f64>,
    ultimo_aviso_prochot: Option<f64>,
    avisou_este_prochot: bool,
    descarregando_desde: Option<f64>,
    avisou_descarga: bool,
    carregador_antes: Option<bool>,
    avisou_bateria_baixa: bool,
}

fn um(v: Option<f64>, casas: usize) -> String {
    v.map_or_else(|| "?".into(), |x| format!("{x:.casas$}"))
}

impl Energia {
    pub fn lida(&self) -> &EnergiaLida {
        &self.lida
    }

    /// Os motivos do freio da volta. `dt` é o tempo desde a anterior.
    pub fn motivos(
        &mut self,
        agora: f64,
        dt: f64,
        m: &Motivos,
        ctx: Contexto,
        bateria: Option<&Bateria>,
    ) -> Vec<Aviso> {
        for nome in m.na_janela() {
            match self
                .lida
                .motivo_leituras
                .iter_mut()
                .find(|(n, _)| *n == nome)
            {
                Some(item) => item.1 += 1,
                None => self.lida.motivo_leituras.push((nome, 1)),
            }
        }
        let mut avisos = Vec::new();
        let prochot = m.na_janela().contains(&"prochot");
        match (prochot, self.prochot_desde) {
            (true, None) => {
                self.prochot_desde = Some(agora);
                self.lida.prochot_eventos += 1;
                self.lida.prochot_s += dt;
                let anota = self
                    .ultimo_aviso_prochot
                    .is_none_or(|u| agora - u >= PROCHOT_AVISO_S);
                if anota {
                    self.ultimo_aviso_prochot = Some(agora);
                }
                self.avisou_este_prochot = true;
                let a = bateria.and_then(|b| b.amperes);
                let mut aviso = Aviso::novo(
                    format!(
                        "⛔ PROCHOT: a CPU foi freada por fora (o SMC ou o carregador): {} MHz, chip {} °C, CPU {} W, bateria {} A",
                        um(ctx.mhz, 0),
                        um(ctx.chip_c, 0),
                        um(ctx.cpu_w, 0),
                        a.map_or_else(|| "?".into(), |a| format!("{a:+.2}")),
                    ),
                    &["prochot", "energia"],
                );
                aviso.so_no_journal = !anota;
                avisos.push(aviso);
            }
            (true, Some(_)) => self.lida.prochot_s += dt,
            (false, Some(desde)) => {
                self.prochot_desde = None;
                if self.avisou_este_prochot {
                    self.avisou_este_prochot = false;
                    let mut aviso = Aviso::novo(
                        format!(
                            "✅ PROCHOT acabou depois de {:.0} s",
                            (agora - desde).max(dt)
                        ),
                        &["prochot", "energia"],
                    );
                    aviso.so_no_journal = true;
                    avisos.push(aviso);
                }
            }
            (false, None) => {}
        }
        avisos
    }

    /// A bateria e o carregador da volta.
    pub fn bateria(&mut self, agora: f64, dt: f64, b: &Bateria, cpu_w: Option<f64>) -> Vec<Aviso> {
        let mut avisos = Vec::new();
        let pct = b.carga_pct;
        let carga = um(pct, 0);
        if let (Some(antes), Some(agora_ligado)) = (self.carregador_antes, b.carregador_ligado)
            && antes != agora_ligado
        {
            avisos.push(Aviso::novo(
                if agora_ligado {
                    format!("🔌 carregador ligado de novo (bateria em {carga} %)")
                } else {
                    format!("🔌 carregador desligado: a fedora está na bateria ({carga} %)")
                },
                &["bateria", "carregador"],
            ));
        }
        if b.carregador_ligado.is_some() {
            self.carregador_antes = b.carregador_ligado;
        }

        if b.descarrega_no_carregador() {
            let a = b.amperes.unwrap_or(0.0).abs();
            self.lida.descarga_s += dt;
            if let Some(v) = b.volts {
                self.lida.descarga_wh += a * v * dt / 3600.0;
            }
            let desde = if let Some(d) = self.descarregando_desde {
                d
            } else {
                self.lida.descarga_episodios += 1;
                self.descarregando_desde = Some(agora);
                agora
            };
            if !self.avisou_descarga && agora - desde >= DESCARGA_AVISO_S {
                self.avisou_descarga = true;
                avisos.push(Aviso::novo(
                    format!(
                        "🔋 a bateria descarrega com o carregador ligado há {:.0} min ({:+.2} A, CPU em {} W): o carregador não dá conta da carga",
                        (agora - desde) / 60.0,
                        -a,
                        um(cpu_w, 0)
                    ),
                    &["bateria"],
                ));
            }
        } else {
            self.descarregando_desde = None;
            self.avisou_descarga = false;
        }

        if b.carregador_ligado == Some(true)
            && let Some(p) = pct
        {
            if !self.avisou_bateria_baixa && p < BATERIA_BAIXA_PCT {
                self.avisou_bateria_baixa = true;
                avisos.push(Aviso::novo(
                    format!(
                        "🔋 a bateria caiu para {p:.0} % mesmo com o carregador ligado: ele não acompanha a carga, e com a bateria vazia o SMC trava a CPU em ~800 MHz"
                    ),
                    &["bateria"],
                ));
            } else if p > BATERIA_REARMA_PCT {
                self.avisou_bateria_baixa = false;
            }
        }
        avisos
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn motivos(agora: bool, log: bool) -> Motivos {
        let v = u64::from(agora) | u64::from(log) << 16 | 1 << 1;
        Motivos::de(v, 0)
    }

    #[test]
    fn prochot_abre_conta_e_fecha() {
        let mut e = Energia::default();
        let ctx = Contexto {
            mhz: Some(798.0),
            chip_c: Some(82.0),
            cpu_w: Some(34.0),
        };
        assert!(
            e.motivos(0.0, 2.0, &motivos(false, false), ctx, None)
                .is_empty()
        );
        let a = e.motivos(2.0, 2.0, &motivos(true, true), ctx, None);
        assert_eq!(a.len(), 1);
        assert!(
            a[0].texto.starts_with("⛔ PROCHOT") && a[0].texto.contains("798 MHz"),
            "{}",
            a[0].texto
        );
        assert!(!a[0].so_no_journal);
        assert!(
            e.motivos(4.0, 2.0, &motivos(true, true), ctx, None)
                .is_empty()
        );
        let a = e.motivos(6.0, 2.0, &motivos(false, false), ctx, None);
        assert_eq!(a[0].texto, "✅ PROCHOT acabou depois de 4 s");
        let l = e.lida();
        assert_eq!(l.prochot_eventos, 1);
        assert!((l.prochot_s - 4.0).abs() < 1e-9);
        assert!(l.motivo_leituras.contains(&("prochot", 2)));
        assert!(l.motivo_leituras.contains(&("termico", 4)));
    }

    #[test]
    fn prochot_so_no_log_tambem_conta() {
        // Durou menos que o intervalo: o bit de agora já caiu, o log ficou.
        let mut e = Energia::default();
        let a = e.motivos(2.0, 2.0, &motivos(false, true), Contexto::default(), None);
        assert_eq!(a.len(), 1);
        assert_eq!(e.lida().prochot_eventos, 1);
    }

    #[test]
    fn uma_anotacao_de_prochot_por_minuto() {
        let mut e = Energia::default();
        let c = Contexto::default();
        let mut anotados = 0;
        for i in 0..10 {
            let t = f64::from(i) * 4.0;
            for a in e.motivos(t, 2.0, &motivos(true, true), c, None) {
                anotados += usize::from(!a.so_no_journal && a.texto.starts_with('⛔'));
            }
            e.motivos(t + 2.0, 2.0, &motivos(false, false), c, None);
        }
        assert_eq!(e.lida().prochot_eventos, 10);
        assert_eq!(anotados, 1, "10 eventos em 40 s, uma anotação");
    }

    fn bateria(ligado: bool, a: f64, pct: f64) -> Bateria {
        Bateria {
            carregador_ligado: Some(ligado),
            estado: if a < 0.0 { "Discharging" } else { "Charging" }.into(),
            amperes: Some(a),
            carga_pct: Some(pct),
            volts: Some(12.5),
        }
    }

    #[test]
    fn soma_os_wh_e_os_episodios_no_carregador() {
        let mut e = Energia::default();
        e.bateria(0.0, 2.0, &bateria(true, -2.0, 90.0), Some(35.0));
        e.bateria(2.0, 2.0, &bateria(true, -2.0, 90.0), Some(35.0));
        e.bateria(4.0, 2.0, &bateria(true, 0.4, 90.0), Some(8.0));
        e.bateria(6.0, 2.0, &bateria(true, -1.0, 90.0), Some(30.0));
        // Sem carregador, a descarga é normal: não conta.
        e.bateria(8.0, 2.0, &bateria(false, -5.0, 89.0), Some(30.0));
        let l = e.lida();
        assert_eq!(l.descarga_episodios, 2);
        assert!((l.descarga_s - 6.0).abs() < 1e-9);
        let wh = (2.0 * 12.5 * 2.0 * 2.0 + 1.0 * 12.5 * 2.0) / 3600.0;
        assert!((l.descarga_wh - wh).abs() < 1e-9);
    }

    #[test]
    fn avisa_o_carregador_nas_duas_direcoes() {
        let mut e = Energia::default();
        assert!(
            e.bateria(0.0, 2.0, &bateria(true, 0.4, 90.0), None)
                .is_empty()
        );
        let a = e.bateria(2.0, 2.0, &bateria(false, -3.0, 90.0), None);
        assert!(
            a[0].texto.starts_with("🔌 carregador desligado"),
            "{}",
            a[0].texto
        );
        let a = e.bateria(4.0, 2.0, &bateria(true, 1.0, 89.0), None);
        assert!(
            a[0].texto.starts_with("🔌 carregador ligado"),
            "{}",
            a[0].texto
        );
    }

    #[test]
    fn bateria_baixa_no_carregador_avisa_uma_vez_e_rearma() {
        let mut e = Energia::default();
        let baixa = |e: &mut Energia, pct| {
            e.bateria(0.0, 2.0, &bateria(true, 0.1, pct), None)
                .iter()
                .filter(|a| a.texto.contains("caiu para"))
                .count()
        };
        assert_eq!(baixa(&mut e, 90.0), 0);
        assert_eq!(baixa(&mut e, 79.0), 1);
        assert_eq!(baixa(&mut e, 75.0), 0);
        assert_eq!(baixa(&mut e, 83.0), 0);
        assert_eq!(baixa(&mut e, 86.0), 0);
        assert_eq!(baixa(&mut e, 78.0), 1);
    }

    #[test]
    fn descarga_longa_avisa_depois_de_5_min() {
        let mut e = Energia::default();
        let mut avisos = 0;
        for i in 0..200 {
            let t = f64::from(i) * 2.0;
            avisos += e
                .bateria(t, 2.0, &bateria(true, -1.5, 90.0), Some(30.0))
                .iter()
                .filter(|a| a.texto.contains("há 5 min"))
                .count();
        }
        assert_eq!(avisos, 1);
    }
}

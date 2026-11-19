//! A conversa com o kernel e com o chip (0.6, "MegaBlaster").
//!
//! - **TCC offset** (`tcc.rs`): a CPU se freia sozinha `tcc_offset_c` graus
//!   antes do TjMax, gravado pelo driver `msr` do kernel. Conferido a cada
//!   30 s, porque o firmware o zera no boot e ao voltar da suspensão.
//! - **Gatilho de pressão** (`gatilhos.rs`): o kernel acorda o termostato
//!   quando a memória aperta, sem esperar a próxima leitura.
//! - **Estrangulamento**: os contadores do kernel de quantas vezes o chip se
//!   estrangulou por calor desde o boot.
//! - **Domínios do RAPL**: a potência dos núcleos, do gráfico e da memória.
//! - **Sensores do Mac**: os 35 do `applesmc`.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::avisos::{Avisador, Aviso};
use crate::config::Config;
use crate::gatilhos::{self, Alarme};
use crate::hardware::{self, Dominio};
use crate::metricas::Retrato;
use crate::tcc::Msr;

/// De quantas em quantas voltas o TCC offset é conferido (15 × 2 s = 30 s).
const CONFERIR_TCC_A_CADA: u64 = 15;
/// Intervalo mínimo entre dois avisos de pressão no journal.
const AVISO_DE_PRESSAO_S: f64 = 30.0;

pub struct Kernel {
    sysfs: PathBuf,
    msr: Option<Msr>,
    tcc_alvo: u8,
    alarme: Option<Arc<Alarme>>,
    dominios: Vec<Dominio>,
    voltas: u64,
    ultimo_aviso_de_pressao_s: Option<f64>,
}

impl Kernel {
    /// Grava o TCC offset e arma o gatilho. Devolve também o TCC offset que o
    /// chip tinha antes, para os valores de fábrica.
    pub fn novo(cfg: &Config, avisador: &Avisador) -> (Self, Option<u8>) {
        let sysfs = PathBuf::from(&cfg.sysfs);
        let msr = match Msr::achar(&PathBuf::from(&cfg.dev_cpu)) {
            Ok(m) => Some(m),
            Err(e) => {
                eprintln!("⚠️ registros da CPU: {e}: sem TCC offset");
                None
            }
        };
        let tcc_de_antes = msr.as_ref().and_then(|m| m.estado().ok()).map(|(o, _)| o);
        let mut k = Self {
            dominios: hardware::dominios(&sysfs),
            sysfs,
            msr,
            tcc_alvo: cfg.tcc_offset_c,
            alarme: None,
            voltas: 0,
            ultimo_aviso_de_pressao_s: None,
        };
        k.garantir_tcc(avisador, true);
        if cfg.psi_parado_ms > 0 {
            match gatilhos::armar(
                &PathBuf::from(&cfg.proc),
                cfg.psi_parado_ms,
                cfg.psi_janela_ms,
            ) {
                Ok(a) => {
                    println!(
                        "⚡ gatilho de pressão armado: o kernel acorda o termostato quando a memória parar {} ms em {} ms",
                        cfg.psi_parado_ms, cfg.psi_janela_ms
                    );
                    k.alarme = Some(a);
                }
                Err(e) => eprintln!("⚠️ gatilho de pressão: {e}: sigo só com as leituras"),
            }
        }
        (k, tcc_de_antes)
    }

    pub fn alarme(&self) -> Option<&Alarme> {
        self.alarme.as_deref()
    }

    /// Grava o TCC offset se o chip não o tiver. `subindo` registra mesmo sem
    /// mudança, para o journal dizer o que vale.
    fn garantir_tcc(&mut self, avisador: &Avisador, subindo: bool) -> Option<(u8, u8)> {
        let m = self.msr.as_ref()?;
        let (atual, tjmax) = match m.estado() {
            Ok(e) => e,
            Err(e) => {
                eprintln!("⚠️ TCC offset: {e}");
                return None;
            }
        };
        if atual == self.tcc_alvo {
            if subindo && self.tcc_alvo > 0 {
                println!(
                    "🛡️ TCC offset já em {} °C: o chip se freia sozinho a {} °C",
                    atual,
                    tjmax.saturating_sub(atual)
                );
            }
            return Some((atual, tjmax));
        }
        match m.definir_offset(self.tcc_alvo) {
            Ok(tjmax) => {
                let texto = format!(
                    "🛡️ TCC offset {} → {} °C: o chip se freia sozinho a {} °C (TjMax {tjmax} °C){}",
                    atual,
                    self.tcc_alvo,
                    tjmax.saturating_sub(self.tcc_alvo),
                    if subindo {
                        ""
                    } else {
                        " — alguém o tinha zerado"
                    }
                );
                println!("{texto}");
                avisador.avisar(Aviso::novo(texto, &["tcc"]));
                Some((self.tcc_alvo, tjmax))
            }
            Err(e) => {
                eprintln!("⚠️ TCC offset: {e}");
                None
            }
        }
    }

    /// Uma volta: confere o TCC de vez em quando e retrata o que o kernel diz.
    pub fn a_cada_volta(&mut self, agora_s: f64, avisador: &Avisador, retrato: &mut Retrato) {
        self.voltas += 1;
        if (self.voltas % CONFERIR_TCC_A_CADA == 1 || retrato.tcc_offset_c.is_none())
            && let Some((offset, tjmax)) = self.garantir_tcc(avisador, false)
        {
            retrato.tcc_offset_c = Some(offset);
            retrato.tjmax_c = Some(tjmax);
        }
        if let Some(a) = &self.alarme {
            retrato.kernel_avisos = a.vezes.load(Ordering::Relaxed);
            if a.tocou.swap(false, Ordering::Relaxed) {
                retrato.contar("kernel_avisou_memoria");
                let pode = self
                    .ultimo_aviso_de_pressao_s
                    .is_none_or(|u| agora_s - u >= AVISO_DE_PRESSAO_S);
                if pode {
                    self.ultimo_aviso_de_pressao_s = Some(agora_s);
                    let texto = "⚡ o kernel avisou: a memória está apertando (processos parados esperando memória)".to_string();
                    println!("{texto}");
                    avisador.avisar(Aviso::novo(texto, &["memoria", "kernel"]));
                }
            }
        }
        retrato.estrangulamento = hardware::estrangulamento(&self.sysfs);
        retrato.dominios_w = self
            .dominios
            .iter_mut()
            .filter_map(|d| d.potencia_w().map(|w| (d.nome.clone(), w)))
            .collect();
        retrato.sensores_do_mac = hardware::sensores_do_mac(&self.sysfs);
    }
}

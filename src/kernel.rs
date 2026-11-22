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
//! - **O vigia** (`vigia.rs`): cada núcleo, os motivos do freio do chip, a
//!   bateria, o swap, o OOM e quem segura a memória.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use crate::avisos::{Avisador, Aviso};
use crate::config::Config;
use crate::energia::{self, Contexto, Energia};
use crate::gatilhos::{self, Alarme};
use crate::hardware::{self, Dominio};
use crate::metricas::Retrato;
use crate::tcc::Msr;
use crate::vigia::{self, Motivos, Vmstat};

/// De quantas em quantas voltas o TCC offset é conferido (15 × 2 s = 30 s).
const CONFERIR_TCC_A_CADA: u64 = 15;
/// Intervalo mínimo entre dois avisos de pressão no journal.
const AVISO_DE_PRESSAO_S: f64 = 30.0;
/// De quantas em quantas voltas quem segura a memória é lido (5 × 2 s = 10 s):
/// ler o `/proc/<pid>/status` de ~500 processos a cada volta é caro à toa.
const MEMORIA_A_CADA: u64 = 5;
/// Acima do corte por mais que isso, o TCC offset não está sendo respeitado.
const FOLGA_DO_CORTE_C: f64 = 2.0;

pub struct Kernel {
    sysfs: PathBuf,
    msr: Option<Msr>,
    tcc_alvo: u8,
    alarme: Option<Arc<Alarme>>,
    dominios: Vec<Dominio>,
    voltas: u64,
    ultimo_aviso_de_pressao_s: Option<f64>,
    proc: PathBuf,
    vmstat_anterior: Option<(Vmstat, f64)>,
    avisou_tcc: bool,
    energia: Energia,
    /// Os bits de log do `MSR 0x690` vistos neste boot (o termostato os limpa).
    motivos_acumulado: u64,
    motivos_arquivo: PathBuf,
    limpar_o_log: bool,
    leu_os_motivos: bool,
    ultima_volta_s: Option<f64>,
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
            proc: PathBuf::from(&cfg.proc),
            vmstat_anterior: None,
            avisou_tcc: false,
            energia: Energia::default(),
            motivos_acumulado: 0,
            motivos_arquivo: PathBuf::new(),
            limpar_o_log: true,
            leu_os_motivos: false,
            ultima_volta_s: None,
        };
        let boot = vigia::boot_id(&k.proc);
        k.motivos_arquivo = PathBuf::from(&cfg.estado).join(format!("motivos-{boot}"));
        k.motivos_acumulado = std::fs::read_to_string(&k.motivos_arquivo)
            .ok()
            .and_then(|t| t.trim().parse().ok())
            .unwrap_or(0);
        apagar_motivos_de_outros_boots(&PathBuf::from(&cfg.estado), &k.motivos_arquivo);
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
        self.vigiar(agora_s, avisador, retrato);
    }

    /// Lê o `MSR 0x690` e limpa o log, para a próxima leitura dizer só o que
    /// aconteceu desde esta. O acumulado do boot fica num arquivo do estado.
    fn ler_os_motivos(&mut self) -> Option<Motivos> {
        let msr = self.msr.as_ref()?;
        let v = msr.ler(vigia::MSR_MOTIVOS).ok()?;
        let log = v & vigia::MASCARA_DO_LOG;
        if log & !self.motivos_acumulado != 0 {
            self.motivos_acumulado |= log;
            let _ = std::fs::write(
                &self.motivos_arquivo,
                format!("{}\n", self.motivos_acumulado),
            );
        }
        if self.limpar_o_log
            && log != 0
            && let Err(e) = msr.gravar(vigia::MSR_MOTIVOS, v & !vigia::MASCARA_DO_LOG)
        {
            self.limpar_o_log = false;
            eprintln!(
                "⚠️ não consegui limpar o log do MSR 0x690 ({e}): sigo só com o bit de agora, e um PROCHOT curto entre duas leituras pode passar"
            );
        }
        let primeira = !self.leu_os_motivos;
        self.leu_os_motivos = true;
        Some(Motivos::de(
            vigia::valor_da_janela(v, primeira),
            self.motivos_acumulado,
        ))
    }

    /// O que realmente acontece na máquina: núcleos, motivos do freio,
    /// bateria, swap e memória.
    fn vigiar(&mut self, agora_s: f64, avisador: &Avisador, retrato: &mut Retrato) {
        retrato.nucleos_c = hardware::nucleos_celsius(&self.sysfs);
        retrato.nucleos_estrangulamento = hardware::estrangulamento_por_nucleo(&self.sysfs);
        retrato.estrangulado_s =
            hardware::estrangulado_ms(&self.sysfs).map(|ms| ms as f64 / 1000.0);
        let dt = self.ultima_volta_s.map_or(0.0, |u| agora_s - u);
        self.ultima_volta_s = Some(agora_s);
        retrato.bateria = vigia::bateria(&self.sysfs);
        retrato.motivos = self.ler_os_motivos();
        let ctx = Contexto {
            mhz: retrato.frequencia_mhz,
            chip_c: retrato.temperatura_c,
            cpu_w: retrato.potencia_w,
        };
        let mut avisos = Vec::new();
        if let Some(m) = &retrato.motivos {
            avisos.extend(
                self.energia
                    .motivos(agora_s, dt, m, ctx, retrato.bateria.as_ref()),
            );
        }
        if let Some(b) = &retrato.bateria {
            avisos.extend(self.energia.bateria(agora_s, dt, b, retrato.potencia_w));
        }
        for a in avisos {
            emitir(&a, avisador, retrato);
        }
        retrato.energia = self.energia.lida().clone();
        if let Ok(texto) = std::fs::read_to_string(self.proc.join("vmstat")) {
            let v = vigia::vmstat(&texto);
            if let Some((antes, quando)) = self.vmstat_anterior {
                retrato.swap_mb_s = Some(vigia::swap_mb_s(antes, v, agora_s - quando));
            }
            retrato.oom_mortes = Some(v.oom_kill);
            self.vmstat_anterior = Some((v, agora_s));
        }
        if self.voltas % MEMORIA_A_CADA == 1 {
            retrato.memoria_maiores = vigia::maiores_em_memoria(&self.proc, 5);
        }
        if let (Some(t), Some(o), Some(tj)) =
            (retrato.temperatura_c, retrato.tcc_offset_c, retrato.tjmax_c)
            && o > 0
            && t > f64::from(tj.saturating_sub(o)) + FOLGA_DO_CORTE_C
        {
            retrato.tcc_ignorado = true;
            if !self.avisou_tcc {
                self.avisou_tcc = true;
                let texto = format!(
                    "🛡️ o chip chegou a {t:.0} °C, acima do corte de {} °C do TCC offset: este chip não respeita o offset, e quem o protege é o TjMax de {tj} °C",
                    tj.saturating_sub(o)
                );
                println!("{texto}");
                avisador.avisar(Aviso::novo(texto, &["tcc"]));
            }
        }
    }
}

/// Um aviso da energia vai para o journal e, se não for repetido, para o
/// Grafana; cada um conta em `termostato_acoes_total`.
fn emitir(a: &energia::Aviso, avisador: &Avisador, retrato: &mut Retrato) {
    println!("{}", a.texto);
    retrato.contar(a.tags.first().copied().unwrap_or("energia"));
    if !a.so_no_journal {
        avisador.avisar(Aviso::novo(a.texto.clone(), a.tags));
    }
}

/// O acumulado dos motivos vale só para o boot dele.
fn apagar_motivos_de_outros_boots(estado: &std::path::Path, deste: &std::path::Path) {
    for e in std::fs::read_dir(estado)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
    {
        let p = e.path();
        if p != deste
            && e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("motivos-"))
        {
            let _ = std::fs::remove_file(p);
        }
    }
}

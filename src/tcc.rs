//! A rede de segurança no chip: o TCC offset.
//!
//! O `MSR 0x1A2` (`MSR_TEMPERATURE_TARGET`) guarda o TjMax (bits 16–23, 100 °C
//! no i7-4770HQ) e o **TCC offset** (bits 24–29): o circuito térmico da própria
//! CPU passa a reduzir o clock `offset` graus antes do TjMax. Com offset 8, o
//! chip se freia sozinho a 92 °C, sem software no meio, em microssegundos, e
//! continua valendo se o termostato travar ou o sistema sufocar.
//!
//! O driver próprio do kernel (`intel_tcc_cooling`) não atende o Haswell: o
//! modelo não está na lista dele. Quem grava aqui é o driver genérico `msr` do
//! kernel (`/dev/cpu/N/msr`), e as conferências que o driver de TCC faria ficam
//! com o termostato:
//!
//! - só grava se o `MSR_PLATFORM_INFO` (0xCE, bit 30) diz que o offset é
//!   programável;
//! - lê antes e mexe **só nos 6 bits** do offset, mantendo o resto;
//! - recusa offsets acima de 15 °C;
//! - lê de novo para confirmar.
//!
//! O offset volta a 0 a cada boot; o termostato o grava ao subir (cedo, antes
//! dos pesados) e o devolve a 0 só no `--desinstalar`. O kernel registra cada
//! gravação direta em MSR e marca a sessão como "tainted", um rótulo para
//! relatórios de bug, sem efeito no funcionamento.

use std::fs::{self, OpenOptions};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

pub const MSR_TEMPERATURE_TARGET: u64 = 0x1A2;
pub const MSR_PLATFORM_INFO: u64 = 0xCE;
pub const OFFSET_MAXIMO_C: u8 = 15;

/// Os bits 24–29.
const MASCARA: u64 = 0x3F << 24;

pub fn tjmax(valor: u64) -> u8 {
    ((valor >> 16) & 0xFF) as u8
}

pub fn offset(valor: u64) -> u8 {
    ((valor & MASCARA) >> 24) as u8
}

/// O valor com o offset trocado e o resto intacto.
pub fn com_offset(valor: u64, offset_c: u8) -> u64 {
    (valor & !MASCARA) | (u64::from(offset_c & 0x3F) << 24)
}

pub fn programavel(platform_info: u64) -> bool {
    platform_info >> 30 & 1 == 1
}

/// Os registros de cada CPU, pelo driver `msr`. Nos testes, arquivos comuns
/// numa pasta temporária fazem o papel de `/dev/cpu`.
pub struct Msr {
    dispositivos: Vec<PathBuf>,
}

impl Msr {
    pub fn achar(dev_cpu: &Path) -> Result<Self, String> {
        let mut dispositivos: Vec<PathBuf> = fs::read_dir(dev_cpu)
            .map_err(|e| format!("{}: {e} (o módulo msr está carregado?)", dev_cpu.display()))?
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
            })
            .map(|e| e.path().join("msr"))
            .filter(|p| p.exists())
            .collect();
        dispositivos.sort();
        if dispositivos.is_empty() {
            return Err(format!("nenhum {}/N/msr (modprobe msr)", dev_cpu.display()));
        }
        Ok(Self { dispositivos })
    }

    fn ler_em(caminho: &Path, registro: u64) -> Result<u64, String> {
        let f = OpenOptions::new()
            .read(true)
            .open(caminho)
            .map_err(|e| format!("{}: {e}", caminho.display()))?;
        let mut b = [0u8; 8];
        f.read_exact_at(&mut b, registro)
            .map_err(|e| format!("{} MSR {registro:#x}: {e}", caminho.display()))?;
        Ok(u64::from_le_bytes(b))
    }

    fn gravar_em(caminho: &Path, registro: u64, valor: u64) -> Result<(), String> {
        let f = OpenOptions::new()
            .write(true)
            .open(caminho)
            .map_err(|e| format!("{}: {e}", caminho.display()))?;
        f.write_all_at(&valor.to_le_bytes(), registro)
            .map_err(|e| format!("{} MSR {registro:#x}: {e}", caminho.display()))
    }

    /// Da CPU 0.
    pub fn ler(&self, registro: u64) -> Result<u64, String> {
        Self::ler_em(&self.dispositivos[0], registro)
    }

    /// Na CPU 0 (o `MSR 0x690` é do pacote).
    pub fn gravar(&self, registro: u64, valor: u64) -> Result<(), String> {
        Self::gravar_em(&self.dispositivos[0], registro, valor)
    }

    /// O offset em vigor (CPU 0) e o TjMax.
    pub fn estado(&self) -> Result<(u8, u8), String> {
        let v = self.ler(MSR_TEMPERATURE_TARGET)?;
        Ok((offset(v), tjmax(v)))
    }

    /// Grava o offset em todas as CPUs, com as conferências. Devolve o TjMax.
    pub fn definir_offset(&self, offset_c: u8) -> Result<u8, String> {
        if offset_c > OFFSET_MAXIMO_C {
            return Err(format!(
                "TCC offset {offset_c} °C acima do máximo de {OFFSET_MAXIMO_C} °C"
            ));
        }
        if !programavel(self.ler(MSR_PLATFORM_INFO)?) {
            return Err(
                "este processador não deixa programar o TCC offset (MSR 0xCE bit 30)".into(),
            );
        }
        for d in &self.dispositivos {
            let antes = Self::ler_em(d, MSR_TEMPERATURE_TARGET)?;
            if offset(antes) != offset_c {
                Self::gravar_em(d, MSR_TEMPERATURE_TARGET, com_offset(antes, offset_c))?;
            }
            let depois = Self::ler_em(d, MSR_TEMPERATURE_TARGET)?;
            if offset(depois) != offset_c {
                return Err(format!(
                    "{}: o chip ficou com offset {} em vez de {offset_c}",
                    d.display(),
                    offset(depois)
                ));
            }
        }
        Ok(tjmax(self.ler(MSR_TEMPERATURE_TARGET)?))
    }
}

#[cfg(test)]
pub mod msr_de_teste {
    //! Um `/dev/cpu` de mentira: cada `N/msr` é um arquivo esparso em que o
    //! registro R mora no byte R, como no dispositivo de verdade.
    use std::fs::{self, OpenOptions};
    use std::os::unix::fs::FileExt;
    use std::path::Path;

    pub fn montar(raiz: &Path, cpus: u32, platform_info: u64, temperature_target: u64) {
        for c in 0..cpus {
            let d = raiz.join(c.to_string());
            fs::create_dir_all(&d).unwrap();
            let f = OpenOptions::new()
                .create(true)
                .truncate(true)
                .read(true)
                .write(true)
                .open(d.join("msr"))
                .unwrap();
            f.write_all_at(&platform_info.to_le_bytes(), super::MSR_PLATFORM_INFO)
                .unwrap();
            f.write_all_at(
                &temperature_target.to_le_bytes(),
                super::MSR_TEMPERATURE_TARGET,
            )
            .unwrap();
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    /// Os valores lidos da fedora em 19/11/2026.
    const PLATFORM_INFO: u64 = 0x0008_0838_f301_1600;
    const TEMPERATURE_TARGET: u64 = 0x0064_1000;

    #[test]
    fn decodifica_os_valores_da_fedora() {
        assert!(programavel(PLATFORM_INFO));
        assert_eq!(tjmax(TEMPERATURE_TARGET), 100);
        assert_eq!(offset(TEMPERATURE_TARGET), 0);
    }

    #[test]
    fn mexe_so_nos_seis_bits() {
        let v = com_offset(TEMPERATURE_TARGET | 0xC000_0000, 8);
        assert_eq!(offset(v), 8);
        assert_eq!(tjmax(v), 100);
        assert_eq!(
            v & !MASCARA,
            TEMPERATURE_TARGET | 0xC000_0000,
            "o resto do registro fica"
        );
    }

    #[test]
    fn grava_em_todas_as_cpus_e_confere() {
        let d = tempfile::tempdir().unwrap();
        msr_de_teste::montar(d.path(), 8, PLATFORM_INFO, TEMPERATURE_TARGET);
        let m = Msr::achar(d.path()).unwrap();
        assert_eq!(m.definir_offset(8).unwrap(), 100);
        assert_eq!(m.estado().unwrap(), (8, 100));
        for c in 0..8 {
            let v = Msr::ler_em(
                &d.path().join(c.to_string()).join("msr"),
                MSR_TEMPERATURE_TARGET,
            )
            .unwrap();
            assert_eq!(offset(v), 8, "cpu {c}");
        }
        m.definir_offset(0).unwrap();
        assert_eq!(m.estado().unwrap(), (0, 100));
    }

    #[test]
    fn recusa_offset_alto_e_cpu_sem_offset_programavel() {
        let d = tempfile::tempdir().unwrap();
        msr_de_teste::montar(d.path(), 2, PLATFORM_INFO & !(1 << 30), TEMPERATURE_TARGET);
        let m = Msr::achar(d.path()).unwrap();
        assert!(m.definir_offset(16).is_err());
        assert!(m.definir_offset(8).unwrap_err().contains("não deixa"));
        assert_eq!(m.estado().unwrap().0, 0, "nada foi gravado");
    }
}

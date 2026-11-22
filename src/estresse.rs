//! `termostato estresse`: leva a CPU, a memória e o disco ao limite e mostra
//! todos os sensores lado a lado, para achar onde a máquina para.
//!
//!   sudo termostato estresse                     # os três, 60 s
//!   sudo termostato estresse cpu segundos=120
//!   termostato estresse memoria memoria_pct=70   # sem sudo: sem a potência
//!   sudo termostato estresse disco disco_mb=4096 pasta=/var/tmp
//!
//! O comando só gera carga e lê. Não mexe em limite, ventoinha nem ajuste: quem
//! decide o freio continua sendo o serviço, com a configuração em vigor. Assim
//! o teste mede a máquina como ela está.
//!
//! Nasceu do diagnóstico de 22/11/2026, feito à mão com `openssl speed` e um
//! laço de shell: o chip foi a 100 °C em 2 s, com as ventoinhas no máximo e o
//! dissipador parado em 42 °C (ver `docs/maquina.md`).
//!
//! As proteções:
//! - a memória nunca deixa menos de 10 % do total livre na largada, e é
//!   devolvida na hora se o disponível cair abaixo de 5 %;
//! - o disco recusa começar sem `disco_mb` + 2 GiB livres, e o arquivo é
//!   apagado no fim, também no Ctrl+C;
//! - com o chip em 99 °C ou mais por 30 s seguidos, a carga para: o limite foi
//!   achado.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use crate::hardware::{self, Rapl, Sensor};
use crate::pressao::meminfo_kb;

pub const SEGUNDOS_MIN: u64 = 5;
pub const SEGUNDOS_MAX: u64 = 1800;
pub const MEMORIA_PCT_MAX: f64 = 90.0;
/// O chip em 99 °C ou mais por tanto tempo: o limite foi achado.
const TETO_C: f64 = 99.0;
const TETO_S: f64 = 30.0;
/// Na largada, a fatia de memória nunca deixa menos que isso do total livre.
const RESERVA_PCT: f64 = 10.0;
/// Durante o teste, abaixo disso a memória é devolvida na hora.
const DEVOLVER_PCT: f64 = 5.0;
/// O disco só começa com `disco_mb` + isto de espaço livre.
const FOLGA_DISCO_MB: u64 = 2048;
const BLOCO: usize = 4 << 20;
/// Entre o núcleo mais quente e o mais frio, com a mesma carga: acima disso,
/// os núcleos estão desiguais.
const DESIGUAL_C: f64 = 8.0;
/// A memória é escrita e conferida nessa fatia por vez, para a bandeira de
/// parar ser vista logo.
const FATIA_MEMORIA: usize = 64 << 20;

#[derive(Debug, Clone, PartialEq)]
pub struct Opcoes {
    pub cpu: bool,
    pub memoria: bool,
    pub disco: bool,
    pub segundos: u64,
    pub threads: usize,
    pub memoria_pct: f64,
    pub disco_mb: u64,
    pub pasta: PathBuf,
    pub sysfs: PathBuf,
    pub proc: PathBuf,
    pub intervalo: Duration,
}

impl Default for Opcoes {
    fn default() -> Self {
        Self {
            cpu: true,
            memoria: true,
            disco: true,
            segundos: 60,
            threads: thread::available_parallelism().map_or(4, std::num::NonZero::get),
            memoria_pct: 50.0,
            disco_mb: 1024,
            pasta: PathBuf::from("/var/tmp"),
            sysfs: PathBuf::from("/sys"),
            proc: PathBuf::from("/proc"),
            intervalo: Duration::from_secs(2),
        }
    }
}

const USO: &str = "use: termostato estresse [cpu] [memoria] [disco] [segundos=60] [threads=N] [memoria_pct=50] [disco_mb=1024] [pasta=/var/tmp]";

/// Os argumentos depois de `estresse`. Sem nenhum alvo, estressa os três.
pub fn interpretar(args: &[String]) -> Result<Opcoes, String> {
    let mut op = Opcoes::default();
    let mut alvos = Vec::new();
    for a in args {
        let a = a.trim();
        if let Some((chave, valor)) = a.split_once('=') {
            let numero = |max: f64| -> Result<f64, String> {
                valor
                    .trim()
                    .replace(',', ".")
                    .parse::<f64>()
                    .ok()
                    .filter(|v| v.is_finite() && *v > 0.0 && *v <= max)
                    .ok_or_else(|| format!("{chave}={valor}: precisa ser um número de 1 a {max}"))
            };
            match chave.trim() {
                "segundos" => {
                    let s = numero(SEGUNDOS_MAX as f64)?.round() as u64;
                    if s < SEGUNDOS_MIN {
                        return Err(format!(
                            "segundos={valor}: o mínimo é {SEGUNDOS_MIN} (o máximo, {SEGUNDOS_MAX})"
                        ));
                    }
                    op.segundos = s;
                }
                "threads" => op.threads = numero(1024.0)?.round().max(1.0) as usize,
                "memoria_pct" | "memória_pct" => {
                    let p = numero(MEMORIA_PCT_MAX)?;
                    if p < 1.0 {
                        return Err(format!("{chave}={valor}: de 1 a {MEMORIA_PCT_MAX}"));
                    }
                    op.memoria_pct = p;
                }
                "disco_mb" => op.disco_mb = numero(10_000_000.0)?.round().max(1.0) as u64,
                "pasta" => op.pasta = PathBuf::from(valor.trim()),
                outra => return Err(format!("não conheço \"{outra}\". {USO}")),
            }
        } else {
            match a.to_lowercase().as_str() {
                "cpu" => alvos.push("cpu"),
                "memoria" | "memória" | "ram" => alvos.push("memoria"),
                "disco" => alvos.push("disco"),
                "tudo" => alvos.extend(["cpu", "memoria", "disco"]),
                _ => {
                    return Err(format!(
                        "\"{a}\" não é alvo: os alvos são cpu, memoria e disco. {USO}"
                    ));
                }
            }
        }
    }
    if !alvos.is_empty() {
        op.cpu = alvos.contains(&"cpu");
        op.memoria = alvos.contains(&"memoria");
        op.disco = alvos.contains(&"disco");
    }
    if op.disco && !op.pasta.is_dir() {
        return Err(format!("a pasta {} não existe", op.pasta.display()));
    }
    Ok(op)
}

// ── A carga ──────────────────────────────────────────────────────────────

/// Gira a CPU em inteiros até a bandeira cair. Devolve quantas voltas deu.
pub fn girar(parar: &AtomicBool, semente: u64) -> u64 {
    let mut x = semente | 1;
    let mut voltas = 0;
    while !parar.load(Ordering::Relaxed) {
        for _ in 0..200_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x = x.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        }
        std::hint::black_box(x);
        voltas += 1;
    }
    voltas
}

fn valor_da_palavra(padrao: u64, i: usize) -> u64 {
    padrao ^ (i as u64).wrapping_mul(0x0000_0100_0000_01B3)
}

/// Escreve o padrão em `buf`, uma palavra de 8 bytes por vez.
pub fn escrever_padrao(buf: &mut [u8], padrao: u64, inicio: usize) {
    for (i, palavra) in buf.as_chunks_mut::<8>().0.iter_mut().enumerate() {
        *palavra = valor_da_palavra(padrao, inicio + i).to_le_bytes();
    }
}

/// Quantas palavras de `buf` não batem com o padrão.
pub fn conferir_padrao(buf: &[u8], padrao: u64, inicio: usize) -> u64 {
    buf.as_chunks::<8>()
        .0
        .iter()
        .enumerate()
        .filter(|(i, palavra)| {
            u64::from_le_bytes(**palavra) != valor_da_palavra(padrao, inicio + i)
        })
        .count() as u64
}

/// Quanto de memória ocupar: a porcentagem do disponível, sem deixar menos que
/// `RESERVA_PCT` do total livre. `None` se a memória já está apertada.
pub fn fatia_de_memoria(total_kb: f64, disponivel_kb: f64, pct: f64) -> Option<u64> {
    let reserva = total_kb * RESERVA_PCT / 100.0;
    let quero = disponivel_kb * pct / 100.0;
    let posso = (disponivel_kb - reserva).max(0.0);
    let kb = quero.min(posso);
    (kb >= 1024.0).then_some((kb * 1024.0) as u64)
}

/// O espaço livre basta para o arquivo e a folga?
pub fn checar_espaco(livre_mb: u64, disco_mb: u64, pasta: &Path) -> Result<(), String> {
    let precisa = disco_mb + FOLGA_DISCO_MB;
    if livre_mb < precisa {
        return Err(format!(
            "pouco espaço em {}: {livre_mb} MB livres, e o teste precisa de {precisa} MB ({disco_mb} MB do arquivo + {FOLGA_DISCO_MB} MB de folga)",
            pasta.display()
        ));
    }
    Ok(())
}

fn espaco_livre_mb(pasta: &Path) -> Result<u64, String> {
    let s = rustix::fs::statvfs(pasta).map_err(|e| format!("{}: {e}", pasta.display()))?;
    Ok(s.f_bavail * s.f_frsize / (1024 * 1024))
}

/// Os contadores que as threads da carga atualizam e o monitor lê.
#[derive(Default)]
pub struct Contadores {
    pub cpu_voltas: AtomicU64,
    pub memoria_bytes: AtomicU64,
    pub memoria_voltas: AtomicU64,
    pub memoria_erros: AtomicU64,
    pub memoria_devolvida: AtomicBool,
    pub disco_gravado: AtomicU64,
    pub disco_lido: AtomicU64,
    pub disco_erros: AtomicU64,
}

/// Escreve e confere a memória até a bandeira cair ou o monitor pedir a
/// memória de volta.
fn estressar_memoria(bytes: u64, parar: &AtomicBool, devolver: &AtomicBool, c: &Contadores) {
    let mut buf = vec![0u8; bytes as usize & !7];
    c.memoria_bytes.store(buf.len() as u64, Ordering::Relaxed);
    let mut volta: u64 = 0;
    'fora: loop {
        let padrao = 0xA5A5_5A5A_0F0F_F0F0 ^ volta.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        for fase in 0..2 {
            for (n, fatia) in buf.chunks_mut(FATIA_MEMORIA).enumerate() {
                if parar.load(Ordering::Relaxed) || devolver.load(Ordering::Relaxed) {
                    break 'fora;
                }
                let inicio = n * FATIA_MEMORIA / 8;
                if fase == 0 {
                    escrever_padrao(fatia, padrao, inicio);
                } else {
                    let erros = conferir_padrao(fatia, padrao, inicio);
                    c.memoria_erros.fetch_add(erros, Ordering::Relaxed);
                }
            }
        }
        volta += 1;
        c.memoria_voltas.store(volta, Ordering::Relaxed);
    }
    if devolver.load(Ordering::Relaxed) {
        c.memoria_devolvida.store(true, Ordering::Relaxed);
    }
    c.memoria_bytes.store(0, Ordering::Relaxed);
}

/// O arquivo do disco, apagado ao sair de escopo.
struct Arquivo(PathBuf);

impl Drop for Arquivo {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn bloco_esperado(buf: &mut [u8], volta: u64, n: u64) {
    escrever_padrao(
        buf,
        volta.rotate_left(17) ^ n.wrapping_mul(0xD6E8_FEB8_6659_FD93),
        0,
    );
}

fn tirar_do_cache(f: &File) {
    #[cfg(target_os = "linux")]
    {
        let _ = rustix::fs::fadvise(f, 0, None, rustix::fs::Advice::DontNeed);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = f;
}

/// Grava, tira do cache, relê e confere, até a bandeira cair.
fn estressar_disco(
    caminho: &Path,
    mb: u64,
    parar: &AtomicBool,
    c: &Contadores,
) -> Result<(), String> {
    let erro = |e: std::io::Error| format!("{}: {e}", caminho.display());
    let _guarda = Arquivo(caminho.to_path_buf());
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(caminho)
        .map_err(erro)?;
    let blocos = (mb * 1024 * 1024).div_ceil(BLOCO as u64);
    let mut esperado = vec![0u8; BLOCO];
    let mut lido = vec![0u8; BLOCO];
    let mut volta = 0;
    while !parar.load(Ordering::Relaxed) {
        f.seek(SeekFrom::Start(0)).map_err(erro)?;
        for n in 0..blocos {
            if parar.load(Ordering::Relaxed) {
                return Ok(());
            }
            bloco_esperado(&mut esperado, volta, n);
            f.write_all(&esperado).map_err(erro)?;
            c.disco_gravado.fetch_add(BLOCO as u64, Ordering::Relaxed);
        }
        f.sync_all().map_err(erro)?;
        tirar_do_cache(&f);
        f.seek(SeekFrom::Start(0)).map_err(erro)?;
        for n in 0..blocos {
            if parar.load(Ordering::Relaxed) {
                return Ok(());
            }
            f.read_exact(&mut lido).map_err(erro)?;
            bloco_esperado(&mut esperado, volta, n);
            if lido != esperado {
                c.disco_erros.fetch_add(1, Ordering::Relaxed);
            }
            c.disco_lido.fetch_add(BLOCO as u64, Ordering::Relaxed);
        }
        volta += 1;
    }
    Ok(())
}

// ── A leitura ────────────────────────────────────────────────────────────

/// Uma leitura dos sensores.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Amostra {
    pub segundos: f64,
    pub chip_c: Option<f64>,
    /// `TC0P`: perto da CPU, na placa.
    pub perto_c: Option<f64>,
    /// Média dos `TH0*`: o dissipador.
    pub dissipador_c: Option<f64>,
    pub potencia_w: Option<f64>,
    pub mhz: Option<f64>,
    /// (RPM, máximo) de cada ventoinha.
    pub ventoinhas: Vec<(u64, u64)>,
    pub memoria_disponivel_pct: Option<f64>,
    pub gravando_mb_s: f64,
    pub lendo_mb_s: f64,
    /// Estrangulamentos do pacote desde a largada.
    pub estrangulamentos: u64,
    /// A temperatura de cada núcleo físico.
    pub nucleos_c: Vec<f64>,
    /// Estrangulamentos de cada núcleo desde a largada.
    pub estrangulamentos_nucleos: Vec<u64>,
    /// Tempo com o pacote estrangulado desde a largada, em ms.
    pub estrangulado_ms: u64,
}

impl Amostra {
    /// A média das ventoinhas em % do máximo.
    pub fn ventoinhas_pct(&self) -> Option<f64> {
        let v: Vec<f64> = self
            .ventoinhas
            .iter()
            .filter(|(_, max)| *max > 0)
            .map(|(rpm, max)| *rpm as f64 / *max as f64 * 100.0)
            .collect();
        (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
    }
}

struct Leitor {
    sysfs: PathBuf,
    proc: PathBuf,
    sensor: Option<Sensor>,
    rapl: Option<Rapl>,
    sem_root: bool,
    estrangulamento_inicio: u64,
    nucleos_inicio: Vec<(u32, u64)>,
    estrangulado_ms_inicio: u64,
}

impl Leitor {
    fn novo(sysfs: &Path, proc: &Path) -> Self {
        let mut rapl = Rapl::achar(sysfs).ok();
        let mut sem_root = false;
        if let Some(r) = rapl.as_mut()
            && r.potencia_w().is_err()
        {
            sem_root = true;
            rapl = None;
        }
        Self {
            sysfs: sysfs.to_path_buf(),
            proc: proc.to_path_buf(),
            sensor: Sensor::achar(sysfs).ok(),
            rapl,
            sem_root,
            estrangulamento_inicio: hardware::estrangulamento(sysfs).map_or(0, |e| e.0),
            nucleos_inicio: hardware::estrangulamento_por_nucleo(sysfs),
            estrangulado_ms_inicio: hardware::estrangulado_ms(sysfs).unwrap_or(0),
        }
    }

    fn memoria_kb(&self) -> Option<(f64, f64)> {
        let texto = fs::read_to_string(self.proc.join("meminfo")).ok()?;
        Some((
            meminfo_kb(&texto, "MemTotal")?,
            meminfo_kb(&texto, "MemAvailable")?,
        ))
    }

    fn ler(&mut self, segundos: f64) -> Amostra {
        let mac = hardware::sensores_do_mac(&self.sysfs);
        let perto_c = mac.iter().find(|(n, _)| n == "TC0P").map(|(_, c)| *c);
        let th: Vec<f64> = mac
            .iter()
            .filter(|(n, _)| n.starts_with("TH0"))
            .map(|(_, c)| *c)
            .collect();
        let dissipador_c = (!th.is_empty()).then(|| th.iter().sum::<f64>() / th.len() as f64);
        let ventoinhas = hardware::ventoinhas(&self.sysfs)
            .iter()
            .filter_map(|v| Some((v.rpm().ok()?, v.maximo().ok()?)))
            .collect();
        let estrangulamentos = hardware::estrangulamento(&self.sysfs)
            .map_or(0, |e| e.0.saturating_sub(self.estrangulamento_inicio));
        Amostra {
            segundos,
            chip_c: self.sensor.as_ref().and_then(|s| s.celsius().ok()),
            perto_c,
            dissipador_c,
            potencia_w: self
                .rapl
                .as_mut()
                .and_then(|r| r.potencia_w().ok().flatten()),
            mhz: hardware::frequencia_mhz(&self.sysfs),
            ventoinhas,
            memoria_disponivel_pct: self
                .memoria_kb()
                .filter(|(t, _)| *t > 0.0)
                .map(|(t, d)| d / t * 100.0),
            estrangulamentos,
            nucleos_c: hardware::nucleos_celsius(&self.sysfs)
                .into_iter()
                .map(|(_, c)| c)
                .collect(),
            estrangulamentos_nucleos: hardware::estrangulamento_por_nucleo(&self.sysfs)
                .into_iter()
                .map(|(id, n)| {
                    let antes = self
                        .nucleos_inicio
                        .iter()
                        .find(|(i, _)| *i == id)
                        .map_or(0, |(_, a)| *a);
                    n.saturating_sub(antes)
                })
                .collect(),
            estrangulado_ms: hardware::estrangulado_ms(&self.sysfs)
                .map_or(0, |m| m.saturating_sub(self.estrangulado_ms_inicio)),
            ..Amostra::default()
        }
    }
}

// ── O resumo e o diagnóstico ─────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motivo {
    Tempo,
    Interrompido,
    Teto,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Resumo {
    pub segundos: f64,
    pub chip_pico_c: Option<f64>,
    pub chip_medio_c: Option<f64>,
    pub perto_pico_c: Option<f64>,
    pub dissipador_inicio_c: Option<f64>,
    pub dissipador_pico_c: Option<f64>,
    pub potencia_media_w: Option<f64>,
    pub potencia_pico_w: Option<f64>,
    pub mhz_medio: Option<f64>,
    pub ventoinhas_pico_pct: Option<f64>,
    pub estrangulamentos: u64,
    pub memoria_erros: u64,
    pub memoria_voltas: u64,
    pub memoria_devolvida: bool,
    pub gravacao_mb_s: Option<f64>,
    pub leitura_mb_s: Option<f64>,
    pub disco_erros: u64,
    /// A média de cada núcleo na segunda metade do teste.
    pub nucleos_medio_c: Vec<f64>,
    pub nucleos_pico_c: Vec<f64>,
    pub estrangulamentos_nucleos: Vec<u64>,
    pub estrangulado_ms: u64,
}

fn media(v: impl Iterator<Item = f64>) -> Option<f64> {
    let (soma, n) = v.fold((0.0, 0usize), |(s, n), x| (s + x, n + 1));
    (n > 0).then(|| soma / n as f64)
}

fn pico(v: impl Iterator<Item = f64>) -> Option<f64> {
    v.fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x))))
}

/// O resumo das amostras. A média do chip olha a segunda metade do teste, já
/// sem a arrancada.
pub fn resumir(amostras: &[Amostra]) -> Resumo {
    let metade = &amostras[amostras.len() / 2..];
    let mb_s = |f: fn(&Amostra) -> f64| media(amostras.iter().map(f).filter(|v| *v > 0.0));
    Resumo {
        segundos: amostras.last().map_or(0.0, |a| a.segundos),
        chip_pico_c: pico(amostras.iter().filter_map(|a| a.chip_c)),
        chip_medio_c: media(metade.iter().filter_map(|a| a.chip_c)),
        perto_pico_c: pico(amostras.iter().filter_map(|a| a.perto_c)),
        dissipador_inicio_c: amostras.iter().find_map(|a| a.dissipador_c),
        dissipador_pico_c: pico(amostras.iter().filter_map(|a| a.dissipador_c)),
        potencia_media_w: media(amostras.iter().filter_map(|a| a.potencia_w)),
        potencia_pico_w: pico(amostras.iter().filter_map(|a| a.potencia_w)),
        mhz_medio: media(amostras.iter().filter_map(|a| a.mhz)),
        ventoinhas_pico_pct: pico(amostras.iter().filter_map(Amostra::ventoinhas_pct)),
        estrangulamentos: amostras.last().map_or(0, |a| a.estrangulamentos),
        gravacao_mb_s: mb_s(|a| a.gravando_mb_s),
        leitura_mb_s: mb_s(|a| a.lendo_mb_s),
        nucleos_medio_c: por_nucleo(metade, media),
        nucleos_pico_c: por_nucleo(amostras, pico),
        estrangulamentos_nucleos: amostras
            .last()
            .map_or_else(Vec::new, |a| a.estrangulamentos_nucleos.clone()),
        estrangulado_ms: amostras.last().map_or(0, |a| a.estrangulado_ms),
        ..Resumo::default()
    }
}

/// Junta cada núcleo pelas amostras com `f` (média ou pico).
fn por_nucleo(amostras: &[Amostra], f: fn(std::vec::IntoIter<f64>) -> Option<f64>) -> Vec<f64> {
    let n = amostras
        .iter()
        .map(|a| a.nucleos_c.len())
        .max()
        .unwrap_or(0);
    (0..n)
        .filter_map(|i| {
            let v: Vec<f64> = amostras
                .iter()
                .filter_map(|a| a.nucleos_c.get(i).copied())
                .collect();
            f(v.into_iter())
        })
        .collect()
}

/// Os núcleos com a mesma carga deveriam esquentar parecido. Uma diferença
/// grande entre o mais quente e o mais frio aponta pasta que não cobre o chip
/// por igual.
pub fn diagnosticar_nucleos(r: &Resumo) -> Option<String> {
    let v = &r.nucleos_medio_c;
    if v.len() < 2 {
        return None;
    }
    let (quente, tq) = v.iter().enumerate().fold(
        (0, f64::MIN),
        |m, (i, t)| if *t > m.1 { (i, *t) } else { m },
    );
    let (frio, tf) = v.iter().enumerate().fold(
        (0, f64::MAX),
        |m, (i, t)| if *t < m.1 { (i, *t) } else { m },
    );
    let d = tq - tf;
    Some(if d >= DESIGUAL_C {
        format!(
            "⚖️ Os núcleos estão desiguais: com a mesma carga, o núcleo {quente} ficou {d:.0} °C acima do núcleo {frio}. A pasta não cobre o chip por igual."
        )
    } else {
        format!("⚖️ Os núcleos estão parelhos: {d:.0} °C entre o mais quente e o mais frio.")
    })
}

/// Onde a máquina parou, numa linha.
pub fn diagnosticar(r: &Resumo) -> String {
    let chip = r.chip_pico_c.unwrap_or(0.0);
    let no_limite = r.estrangulamentos > 0 || chip >= TETO_C;
    if !no_limite {
        return format!(
            "✅ Aguentou: o chip chegou a {chip:.0} °C, sem estrangulamento. Há folga para mais carga."
        );
    }
    let subiu = match (r.dissipador_inicio_c, r.dissipador_pico_c) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    };
    let ventoinhas = r.ventoinhas_pico_pct;
    if ventoinhas.is_some_and(|v| v < 90.0) {
        return format!(
            "🌀 As ventoinhas não acompanharam: ficaram em {:.0} % do máximo com o chip a {chip:.0} °C. Confira o modo das ventoinhas (termostato ajustes).",
            ventoinhas.unwrap_or(0.0)
        );
    }
    match subiu {
        Some(s) if s < 5.0 => format!(
            "🧱 O calor não sai do chip: ele chegou a {chip:.0} °C com as ventoinhas no máximo, e o dissipador subiu só {s:.0} °C. O calor não chega às aletas: o heat pipe (a solda dele com a placa de cobre), o contato do dissipador ou a pasta."
        ),
        Some(s) => format!(
            "🔥 O resfriamento chegou ao limite: o chip foi a {chip:.0} °C e o dissipador esquentou junto (+{s:.0} °C), com as ventoinhas no máximo. O chassi não dá conta de mais calor que isso."
        ),
        None => format!(
            "🔥 O chip chegou ao limite ({chip:.0} °C, {} estrangulamentos). Sem sensor do dissipador para dizer onde está o gargalo.",
            r.estrangulamentos
        ),
    }
}

// ── O comando ────────────────────────────────────────────────────────────

fn num(v: Option<f64>, casas: usize) -> String {
    v.map_or_else(|| "—".into(), |x| format!("{x:.casas$}"))
}

fn linha(a: &Amostra) -> String {
    let ventoinhas = if a.ventoinhas.is_empty() {
        "—".to_string()
    } else {
        a.ventoinhas
            .iter()
            .map(|(r, _)| r.to_string())
            .collect::<Vec<_>>()
            .join("/")
    };
    let nucleos = if a.nucleos_c.is_empty() {
        "—".to_string()
    } else {
        juntar(&a.nucleos_c)
    };
    format!(
        "{:>5.0} {:>5} {:>12} {:>6} {:>6} {:>5} {:>5} {:>10} {:>6} {:>6}/{:<6} {:>7}",
        a.segundos,
        num(a.chip_c, 0),
        nucleos,
        num(a.perto_c, 0),
        num(a.dissipador_c, 0),
        num(a.potencia_w, 0),
        num(a.mhz, 0),
        ventoinhas,
        num(a.memoria_disponivel_pct, 0),
        format!("{:.0}", a.gravando_mb_s),
        format!("{:.0}", a.lendo_mb_s),
        a.estrangulamentos,
    )
}

fn juntar(v: &[f64]) -> String {
    v.iter()
        .map(|c| format!("{c:.0}"))
        .collect::<Vec<_>>()
        .join("/")
}

const CABECALHO: &str =
    "    s  chip      núcleos  perto dissip     W   MHz ventoinhas  mem%  grava/lê MB/s  estrang";

/// O que o teste vai fazer, antes de começar.
fn anunciar(op: &Opcoes, memoria_bytes: Option<u64>, leitor: &Leitor) {
    let mut partes = Vec::new();
    if op.cpu {
        partes.push(format!("CPU em {} threads", op.threads));
    }
    if let Some(b) = memoria_bytes {
        partes.push(format!(
            "memória: {:.1} GB ({:.0} % do disponível)",
            b as f64 / 1e9,
            op.memoria_pct
        ));
    }
    if op.disco {
        partes.push(format!(
            "disco: {} MB em {}",
            op.disco_mb,
            op.pasta.display()
        ));
    }
    println!(
        "🏋️ estresse por {} s · {} · Ctrl+C para parar",
        op.segundos,
        partes.join(" · ")
    );
    if let Some(pl1) = Rapl::achar(&op.sysfs).ok().and_then(|r| r.pl1_uw().ok()) {
        println!(
            "🎛️ o freio continua com o serviço: PL1 em vigor agora {:.0} W",
            pl1 as f64 / 1e6
        );
    }
    if leitor.sem_root {
        println!("⚠️ sem root a potência não é lida: rode com sudo para ver os watts");
    }
}

type Threads = (
    Vec<thread::JoinHandle<()>>,
    Option<thread::JoinHandle<Result<(), String>>>,
);

/// Solta as threads da carga: CPU, memória e disco.
fn soltar_carga(
    op: &Opcoes,
    memoria_bytes: Option<u64>,
    arquivo: &Path,
    carga: &Arc<AtomicBool>,
    devolver: &Arc<AtomicBool>,
    c: &Arc<Contadores>,
) -> Threads {
    let mut threads = Vec::new();
    if op.cpu {
        for i in 0..op.threads {
            let (carga, c) = (Arc::clone(carga), Arc::clone(c));
            threads.push(thread::spawn(move || {
                let v = girar(&carga, 0x1234_5678 + i as u64);
                c.cpu_voltas.fetch_add(v, Ordering::Relaxed);
            }));
        }
    }
    if let Some(bytes) = memoria_bytes {
        let (carga, devolver, c) = (Arc::clone(carga), Arc::clone(devolver), Arc::clone(c));
        threads.push(thread::spawn(move || {
            estressar_memoria(bytes, &carga, &devolver, &c);
        }));
    }
    let disco = op.disco.then(|| {
        let (carga, c, arquivo, mb) = (
            Arc::clone(carga),
            Arc::clone(c),
            arquivo.to_path_buf(),
            op.disco_mb,
        );
        thread::spawn(move || estressar_disco(&arquivo, mb, &carga, &c))
    });
    (threads, disco)
}

/// Roda o teste e devolve o resumo. As threads param e o arquivo é apagado em
/// qualquer saída.
pub fn executar(op: &Opcoes, parar: &Arc<AtomicBool>) -> Result<(Resumo, Motivo), String> {
    let mut leitor = Leitor::novo(&op.sysfs, &op.proc);
    let memoria_bytes = if op.memoria {
        let (total, disponivel) = leitor
            .memoria_kb()
            .ok_or("não consegui ler o /proc/meminfo")?;
        Some(
            fatia_de_memoria(total, disponivel, op.memoria_pct).ok_or_else(|| {
                format!(
                    "a memória já está apertada ({:.0} % disponível): não há o que estressar sem deixar menos de {RESERVA_PCT:.0} % livre",
                    disponivel / total * 100.0
                )
            })?,
        )
    } else {
        None
    };
    let arquivo = op
        .pasta
        .join(format!("termostato-estresse-{}", std::process::id()));
    if op.disco {
        checar_espaco(espaco_livre_mb(&op.pasta)?, op.disco_mb, &op.pasta)?;
    }
    anunciar(op, memoria_bytes, &leitor);

    let carga = Arc::new(AtomicBool::new(false));
    let devolver = Arc::new(AtomicBool::new(false));
    let c = Arc::new(Contadores::default());
    let (threads, disco) = soltar_carga(op, memoria_bytes, &arquivo, &carga, &devolver, &c);

    println!("{CABECALHO}");
    let largada = Instant::now();
    let mut amostras = Vec::new();
    let mut quente_desde: Option<Instant> = None;
    let mut anterior = (0u64, 0u64, Instant::now());
    let motivo = loop {
        let espera = Instant::now() + op.intervalo;
        while Instant::now() < espera {
            if parar.load(Ordering::Relaxed) {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        let agora = Instant::now();
        let mut a = leitor.ler(largada.elapsed().as_secs_f64());
        let (g, l) = (
            c.disco_gravado.load(Ordering::Relaxed),
            c.disco_lido.load(Ordering::Relaxed),
        );
        let dt = agora.duration_since(anterior.2).as_secs_f64().max(1e-3);
        a.gravando_mb_s = (g - anterior.0) as f64 / 1e6 / dt;
        a.lendo_mb_s = (l - anterior.1) as f64 / 1e6 / dt;
        anterior = (g, l, agora);
        if a.memoria_disponivel_pct.is_some_and(|p| p < DEVOLVER_PCT)
            && !devolver.swap(true, Ordering::Relaxed)
        {
            println!(
                "⚠️ memória disponível abaixo de {DEVOLVER_PCT:.0} %: devolvi a memória do teste"
            );
        }
        quente_desde = match a.chip_c {
            Some(t) if t >= TETO_C => quente_desde.or(Some(agora)),
            _ => None,
        };
        println!("{}", linha(&a));
        amostras.push(a);
        if parar.load(Ordering::Relaxed) {
            break Motivo::Interrompido;
        }
        if quente_desde.is_some_and(|q| agora.duration_since(q).as_secs_f64() >= TETO_S) {
            break Motivo::Teto;
        }
        if largada.elapsed().as_secs() >= op.segundos {
            break Motivo::Tempo;
        }
    };

    carga.store(true, Ordering::Relaxed);
    for t in threads {
        let _ = t.join();
    }
    let erro_disco = disco.and_then(|t| t.join().ok()).and_then(Result::err);
    let _ = fs::remove_file(&arquivo);

    let mut r = resumir(&amostras);
    r.memoria_erros = c.memoria_erros.load(Ordering::Relaxed);
    r.memoria_voltas = c.memoria_voltas.load(Ordering::Relaxed);
    r.memoria_devolvida = c.memoria_devolvida.load(Ordering::Relaxed);
    r.disco_erros = c.disco_erros.load(Ordering::Relaxed);
    if let Some(e) = erro_disco {
        println!("⚠️ o disco parou com erro: {e}");
    }
    Ok((r, motivo))
}

fn imprimir_resumo(op: &Opcoes, r: &Resumo, motivo: Motivo) {
    let por = match motivo {
        Motivo::Tempo => "tempo cumprido",
        Motivo::Interrompido => "interrompido",
        Motivo::Teto => "o chip ficou 30 s em 99 °C ou mais: limite achado",
    };
    println!();
    println!("📋 resumo ({:.0} s, {por})", r.segundos);
    println!(
        "   chip: pico {} °C, média da segunda metade {} °C · perto da CPU: pico {} °C",
        num(r.chip_pico_c, 0),
        num(r.chip_medio_c, 0),
        num(r.perto_pico_c, 0)
    );
    println!(
        "   dissipador: {} → {} °C · ventoinhas: pico {} % do máximo",
        num(r.dissipador_inicio_c, 0),
        num(r.dissipador_pico_c, 0),
        num(r.ventoinhas_pico_pct, 0)
    );
    println!(
        "   potência: média {} W, pico {} W · frequência média {} MHz · estrangulamentos: {}",
        num(r.potencia_media_w, 1),
        num(r.potencia_pico_w, 1),
        num(r.mhz_medio, 0),
        r.estrangulamentos
    );
    if op.memoria {
        let estado = if r.memoria_devolvida {
            " (devolvida antes do fim: a memória apertou)"
        } else {
            ""
        };
        println!(
            "   memória: {} voltas, {} erros{estado}",
            r.memoria_voltas, r.memoria_erros
        );
    }
    if op.disco {
        println!(
            "   disco: grava {} MB/s, lê {} MB/s, {} erros",
            num(r.gravacao_mb_s, 0),
            num(r.leitura_mb_s, 0),
            r.disco_erros
        );
    }
    if !r.nucleos_medio_c.is_empty() {
        println!(
            "   núcleos: médias {} °C, picos {} °C · estrangulamentos por núcleo: {} · tempo estrangulado: {:.1} s",
            juntar(&r.nucleos_medio_c),
            juntar(&r.nucleos_pico_c),
            r.estrangulamentos_nucleos
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("/"),
            r.estrangulado_ms as f64 / 1000.0
        );
    }
    println!("{}", diagnosticar(r));
    if let Some(n) = diagnosticar_nucleos(r) {
        println!("{n}");
    }
    if r.memoria_erros > 0 || r.disco_erros > 0 {
        println!("❌ houve erro de conferência: o hardware devolveu outro dado que o gravado");
    }
}

/// `termostato estresse …`
pub fn principal(args: &[String]) -> Result<(), String> {
    let op = interpretar(args)?;
    let parar = Arc::new(AtomicBool::new(false));
    for sinal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        signal_hook::flag::register(sinal, Arc::clone(&parar))
            .map_err(|e| format!("sinal {sinal}: {e}"))?;
    }
    let (r, motivo) = executar(&op, &parar)?;
    imprimir_resumo(&op, &r, motivo);
    Ok(())
}

#[cfg(test)]
mod testes {
    use super::*;
    use crate::hardware::sysfs_de_teste;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn sem_alvo_estressa_os_tres() {
        let op = interpretar(&args(&[])).unwrap();
        assert!(op.cpu && op.memoria && op.disco);
        assert_eq!(op.segundos, 60);
    }

    #[test]
    fn alvos_e_opcoes() {
        let d = tempfile::tempdir().unwrap();
        let pasta = format!("pasta={}", d.path().display());
        let op = interpretar(&args(&[
            "cpu",
            "disco",
            "segundos=120",
            "threads=3",
            "disco_mb=64",
            &pasta,
        ]))
        .unwrap();
        assert!(op.cpu && !op.memoria && op.disco);
        assert_eq!((op.segundos, op.threads, op.disco_mb), (120, 3, 64));
        assert_eq!(op.pasta, d.path());
        let op = interpretar(&args(&["memória", "memoria_pct=70,5"])).unwrap();
        assert!(!op.cpu && op.memoria && !op.disco);
        assert!((op.memoria_pct - 70.5).abs() < 1e-9);
    }

    #[test]
    fn recusa_o_que_nao_entende() {
        for a in [
            &["gpu"][..],
            &["segundos=2"],
            &["segundos=5000"],
            &["memoria_pct=95"],
            &["memoria_pct=0"],
            &["cor=azul"],
            &["disco", "pasta=/nao/existe/mesmo"],
        ] {
            assert!(interpretar(&args(a)).is_err(), "{a:?}");
        }
        let e = interpretar(&args(&["gpu"])).unwrap_err();
        assert!(e.contains("cpu, memoria e disco"), "{e}");
    }

    #[test]
    fn a_cpu_para_quando_a_bandeira_cai() {
        let parar = Arc::new(AtomicBool::new(false));
        let p = Arc::clone(&parar);
        let t = thread::spawn(move || girar(&p, 7));
        thread::sleep(Duration::from_millis(50));
        parar.store(true, Ordering::Relaxed);
        assert!(t.join().unwrap() > 0);
    }

    #[test]
    fn o_memtest_acha_um_erro_plantado() {
        let mut buf = vec![0u8; 4096];
        escrever_padrao(&mut buf, 0xDEAD_BEEF, 10);
        assert_eq!(conferir_padrao(&buf, 0xDEAD_BEEF, 10), 0);
        buf[1000] ^= 0x10;
        assert_eq!(conferir_padrao(&buf, 0xDEAD_BEEF, 10), 1);
        assert_eq!(conferir_padrao(&buf, 0xDEAD_BEEF, 11), 512);
    }

    #[test]
    fn a_fatia_de_memoria_guarda_a_reserva() {
        // 16 GB no total, 8 GB disponíveis, 50 % → 4 GB.
        let b = fatia_de_memoria(16e6, 8e6, 50.0).unwrap();
        assert_eq!(b, (4e6 * 1024.0) as u64);
        // 90 % de 2 GB deixaria menos de 10 % de 16 GB: corta para 0,4 GB.
        let b = fatia_de_memoria(16e6, 2e6, 90.0).unwrap();
        assert_eq!(b, (0.4e6 * 1024.0) as u64);
        // Abaixo da reserva, nada.
        assert_eq!(fatia_de_memoria(16e6, 1.5e6, 50.0), None);
    }

    #[test]
    fn o_disco_recusa_sem_espaco() {
        let p = Path::new("/var/tmp");
        assert!(checar_espaco(10_000, 1024, p).is_ok());
        let e = checar_espaco(2_000, 1024, p).unwrap_err();
        assert!(e.contains("2000 MB livres") && e.contains("3072 MB"), "{e}");
    }

    #[test]
    fn o_disco_grava_confere_e_apaga() {
        let d = tempfile::tempdir().unwrap();
        let caminho = d.path().join("arquivo");
        let parar = Arc::new(AtomicBool::new(false));
        let c = Arc::new(Contadores::default());
        let (p, c2, cam) = (Arc::clone(&parar), Arc::clone(&c), caminho.clone());
        let t = thread::spawn(move || estressar_disco(&cam, 8, &p, &c2));
        let limite = Instant::now() + Duration::from_secs(20);
        while c.disco_lido.load(Ordering::Relaxed) < 8 << 20 && Instant::now() < limite {
            thread::sleep(Duration::from_millis(10));
        }
        parar.store(true, Ordering::Relaxed);
        t.join().unwrap().unwrap();
        assert!(c.disco_lido.load(Ordering::Relaxed) >= 8 << 20);
        assert_eq!(c.disco_erros.load(Ordering::Relaxed), 0);
        assert!(!caminho.exists(), "o arquivo precisa sumir");
    }

    fn resumo_quente(estrang: u64, ventoinhas: f64, dissip: (f64, f64)) -> Resumo {
        Resumo {
            chip_pico_c: Some(100.0),
            estrangulamentos: estrang,
            ventoinhas_pico_pct: Some(ventoinhas),
            dissipador_inicio_c: Some(dissip.0),
            dissipador_pico_c: Some(dissip.1),
            ..Resumo::default()
        }
    }

    #[test]
    fn as_quatro_leituras_do_diagnostico() {
        let r = Resumo {
            chip_pico_c: Some(80.0),
            ..Resumo::default()
        };
        assert!(diagnosticar(&r).starts_with("✅"));
        assert!(diagnosticar(&resumo_quente(6261, 99.0, (42.0, 42.0))).starts_with("🧱"));
        assert!(diagnosticar(&resumo_quente(10, 60.0, (42.0, 50.0))).starts_with("🌀"));
        assert!(diagnosticar(&resumo_quente(10, 99.0, (42.0, 60.0))).starts_with("🔥"));
    }

    #[test]
    fn nucleos_desiguais_ou_parelhos() {
        let a = |nucleos: Vec<f64>| Amostra {
            nucleos_c: nucleos,
            ..Amostra::default()
        };
        let r = resumir(&[
            a(vec![60.0, 60.0, 60.0, 60.0]),
            a(vec![88.0, 99.0, 90.0, 89.0]),
        ]);
        assert_eq!(r.nucleos_medio_c, vec![88.0, 99.0, 90.0, 89.0]);
        assert_eq!(r.nucleos_pico_c, vec![88.0, 99.0, 90.0, 89.0]);
        let t = diagnosticar_nucleos(&r).unwrap();
        assert!(
            t.contains("desiguais") && t.contains("núcleo 1") && t.contains("núcleo 0"),
            "{t}"
        );
        let r = resumir(&[a(vec![90.0, 93.0, 91.0, 92.0])]);
        assert!(diagnosticar_nucleos(&r).unwrap().contains("parelhos"));
        assert_eq!(diagnosticar_nucleos(&Resumo::default()), None);
    }

    #[test]
    fn o_resumo_tira_picos_e_medias() {
        let a = |s, chip, w, est| Amostra {
            segundos: s,
            chip_c: Some(chip),
            potencia_w: Some(w),
            ventoinhas: vec![(6000, 6000), (3000, 6000)],
            estrangulamentos: est,
            gravando_mb_s: 100.0,
            ..Amostra::default()
        };
        let r = resumir(&[
            a(2.0, 60.0, 10.0, 0),
            a(4.0, 100.0, 60.0, 5),
            a(6.0, 90.0, 50.0, 9),
        ]);
        assert_eq!(r.chip_pico_c, Some(100.0));
        assert_eq!(r.chip_medio_c, Some(95.0));
        assert_eq!(r.potencia_pico_w, Some(60.0));
        assert_eq!(r.potencia_media_w, Some(40.0));
        assert_eq!(r.ventoinhas_pico_pct, Some(75.0));
        assert_eq!(r.estrangulamentos, 9);
        assert_eq!(r.gravacao_mb_s, Some(100.0));
        assert_eq!(r.leitura_mb_s, None);
    }

    #[test]
    fn de_ponta_a_ponta_no_sysfs_de_mentira() {
        let d = tempfile::tempdir().unwrap();
        let sysfs = d.path().join("sys");
        let proc = d.path().join("proc");
        let pasta = d.path().join("tmp");
        sysfs_de_teste::montar(&sysfs, 70);
        fs::create_dir_all(&proc).unwrap();
        fs::create_dir_all(&pasta).unwrap();
        fs::write(
            proc.join("meminfo"),
            "MemTotal: 100000 kB\nMemAvailable: 80000 kB\n",
        )
        .unwrap();
        let op = Opcoes {
            segundos: 1,
            threads: 2,
            memoria_pct: 10.0,
            disco_mb: 4,
            pasta: pasta.clone(),
            sysfs,
            proc,
            intervalo: Duration::from_millis(200),
            ..Opcoes::default()
        };
        let parar = Arc::new(AtomicBool::new(false));
        let (r, motivo) = executar(&op, &parar).unwrap();
        assert_eq!(motivo, Motivo::Tempo);
        assert_eq!(r.chip_pico_c, Some(70.0));
        assert_eq!(r.memoria_erros, 0);
        assert_eq!(r.disco_erros, 0);
        assert!(diagnosticar(&r).starts_with("✅"));
        assert_eq!(fs::read_dir(&pasta).unwrap().count(), 0, "sobrou arquivo");
    }
}

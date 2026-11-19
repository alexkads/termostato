//! O termostato da fedora (MacBookPro11,2, i7-4770HQ).
//!
//! Um serviço só, dono de toda a parte térmica, para ninguém mexer no sistema
//! operacional: limite de potência sustentada (RAPL PL1), freio de calor,
//! piso das ventoinhas e as métricas que o Prometheus lê em `/metrics`.
//!
//! Ao parar (SIGTERM, SIGINT), devolve o PL1 e os pisos das ventoinhas que
//! encontrou ao subir.

mod config;
mod freio;
mod hardware;
mod metricas;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use config::{Config, Ventoinhas};
use freio::{Freio, Mudanca};
use hardware::{Rapl, Sensor, Ventoinha};
use metricas::{Retrato, VentoinhaLida};

const CONFIG_PADRAO: &str = "/etc/termostato.toml";

fn w_para_uw(w: f64) -> u64 {
    (w * 1_000_000.0).round() as u64
}

fn uw_para_w(uw: u64) -> f64 {
    uw as f64 / 1_000_000.0
}

/// O que estava no hardware antes de o termostato subir.
struct Original {
    pl1_uw: Option<u64>,
    pisos: Vec<(usize, u64)>,
}

struct Estado {
    retrato: Retrato,
    ultima_leitura: Option<Instant>,
}

fn main() {
    let caminho = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from(CONFIG_PADRAO), PathBuf::from);
    if let Err(e) = rodar(&caminho) {
        eprintln!("❌ {e}");
        std::process::exit(1);
    }
}

/// A primeira linha do journal: o que este termostato vai fazer.
fn anunciar(cfg: &Config, original: &Original, ventoinhas: usize) {
    let achado = original
        .pl1_uw
        .map_or_else(|| "?".into(), |u| uw_para_w(u).to_string());
    let modo = match cfg.ventoinhas {
        Ventoinhas::Maximo => "no máximo",
        Ventoinhas::Automatico => "automático",
    };
    println!(
        "🌡️ termostato {} · PL1 {} W (achado {achado} W), freio {} W a partir de {} °C por {} leituras, solta abaixo de {} °C · ventoinhas: {modo} ({ventoinhas}) · métricas em http://{}/metrics",
        env!("CARGO_PKG_VERSION"),
        cfg.pl1_normal_w,
        cfg.pl1_freio_w,
        cfg.freio_acima_c,
        cfg.leituras_para_frear,
        cfg.solta_abaixo_c,
        cfg.escuta,
    );
}

fn rodar(caminho: &Path) -> Result<(), String> {
    let cfg = Config::ler(caminho)?;
    let sysfs = PathBuf::from(&cfg.sysfs);
    let sensor = Sensor::achar(&sysfs)?;
    let mut rapl = match Rapl::achar(&sysfs) {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("⚠️ {e}: sigo sem limite de potência, só medindo");
            None
        }
    };
    let ventoinhas = hardware::ventoinhas(&sysfs);

    let original = Original {
        pl1_uw: rapl.as_ref().and_then(|r| r.pl1_uw().ok()),
        pisos: ventoinhas
            .iter()
            .enumerate()
            .filter_map(|(i, v)| v.piso().ok().map(|p| (i, p)))
            .collect(),
    };
    anunciar(&cfg, &original, ventoinhas.len());

    let parar = Arc::new(AtomicBool::new(false));
    for sinal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT] {
        signal_hook::flag::register(sinal, Arc::clone(&parar))
            .map_err(|e| format!("sinal {sinal}: {e}"))?;
    }

    let estado = Arc::new(Mutex::new(Estado {
        retrato: Retrato::default(),
        ultima_leitura: None,
    }));
    let ouvinte = TcpListener::bind(&cfg.escuta)
        .map_err(|e| format!("não escutei em {}: {e}", cfg.escuta))?;
    {
        let estado = Arc::clone(&estado);
        let validade = Duration::from_secs_f64(cfg.intervalo_s * 3.0);
        std::thread::spawn(move || servir(&ouvinte, &estado, validade));
    }

    let mut freio = Freio::default();
    let mut retrato = Retrato::default();
    let intervalo = Duration::from_secs_f64(cfg.intervalo_s);
    let mut anterior = Instant::now();

    while !parar.load(Ordering::Relaxed) {
        let agora = Instant::now();
        let dt = agora.duration_since(anterior).as_secs_f64();
        anterior = agora;
        retrato.leituras += 1;

        ler_a_temperatura(&sensor, &mut freio, &cfg, &mut retrato);
        if freio.puxado() {
            retrato.segundos_freado += dt;
        }
        retrato.freio_puxado = freio.puxado();
        retrato.pl1_alvo_w = freio.pl1_w(&cfg);

        if let Some(r) = rapl.as_mut() {
            cuidar_do_rapl(r, w_para_uw(retrato.pl1_alvo_w), &mut retrato);
        }
        if cfg.ventoinhas == Ventoinhas::Maximo {
            for v in &ventoinhas {
                cuidar_da_ventoinha(v, &mut retrato);
            }
        }
        retrato.ventoinhas = ventoinhas
            .iter()
            .map(|v| VentoinhaLida {
                numero: v.numero.clone(),
                rpm: v.rpm().ok(),
                piso: v.piso().ok(),
                maximo: v.maximo().ok(),
            })
            .collect();
        retrato.frequencia_mhz = hardware::frequencia_mhz(&sysfs);

        if let Ok(mut e) = estado.lock() {
            e.retrato = retrato.clone();
            e.ultima_leitura = Some(Instant::now());
        }
        esperar(intervalo, &parar);
    }

    devolver(rapl.as_ref(), &ventoinhas, &original);
    Ok(())
}

/// Uma leitura do sensor, passada ao freio.
fn ler_a_temperatura(sensor: &Sensor, freio: &mut Freio, cfg: &Config, retrato: &mut Retrato) {
    match sensor.celsius() {
        Ok(t) => {
            retrato.temperatura_c = Some(t);
            match freio.ler(t, cfg) {
                Some(Mudanca::Puxou) => {
                    retrato.freadas += 1;
                    println!(
                        "🔥 {t} °C por {} leituras: freio puxado, PL1 em {} W",
                        cfg.leituras_para_frear, cfg.pl1_freio_w
                    );
                }
                Some(Mudanca::Soltou) => println!(
                    "❄️ {t} °C: freio solto, PL1 de volta a {} W",
                    cfg.pl1_normal_w
                ),
                None => {}
            }
        }
        Err(e) => {
            retrato.falhas_de_leitura += 1;
            retrato.temperatura_c = None;
            eprintln!("⚠️ temperatura: {e}");
        }
    }
}

/// Grava o PL1 quando ele não é o alvo: na subida, a cada mudança do freio, e
/// quando outro programa (o `tuned`, um script) o mudou por fora.
fn cuidar_do_rapl(r: &mut Rapl, alvo_uw: u64, retrato: &mut Retrato) {
    match r.pl1_uw() {
        Ok(atual) if atual.abs_diff(alvo_uw) > 500_000 => {
            if let Err(e) = r.definir_pl1_uw(alvo_uw) {
                retrato.falhas_de_leitura += 1;
                eprintln!("⚠️ PL1: {e}");
            } else if retrato
                .pl1_w
                .is_some_and(|antes| w_para_uw(antes).abs_diff(atual) < 500_000)
            {
                // O PL1 lido na volta anterior era o nosso: quem mudou foi o freio.
            } else {
                println!(
                    "🔧 PL1 estava em {} W: reaplicado {} W",
                    uw_para_w(atual),
                    uw_para_w(alvo_uw)
                );
            }
        }
        Ok(_) => {}
        Err(e) => {
            retrato.falhas_de_leitura += 1;
            eprintln!("⚠️ PL1: {e}");
        }
    }
    retrato.pl1_w = r.pl1_uw().ok().map(uw_para_w);
    retrato.pl2_w = r.pl2_uw().ok().map(uw_para_w);
    match r.potencia_w() {
        Ok(p) => retrato.potencia_w = p.or(retrato.potencia_w),
        Err(e) => {
            retrato.falhas_de_leitura += 1;
            eprintln!("⚠️ potência: {e}");
        }
    }
}

fn cuidar_da_ventoinha(v: &Ventoinha, retrato: &mut Retrato) {
    match (v.piso(), v.maximo()) {
        (Ok(piso), Ok(max)) if piso != max => match v.definir_piso(max) {
            Ok(()) => println!("🌀 ventoinha {}: piso {piso} → {max} RPM", v.numero),
            Err(e) => {
                retrato.falhas_de_leitura += 1;
                eprintln!("⚠️ ventoinha {}: {e}", v.numero);
            }
        },
        (Ok(_), Ok(_)) => {}
        (Err(e), _) | (_, Err(e)) => {
            retrato.falhas_de_leitura += 1;
            eprintln!("⚠️ ventoinha {}: {e}", v.numero);
        }
    }
}

/// Dorme o intervalo em fatias curtas, para o SIGTERM não esperar a volta toda.
fn esperar(intervalo: Duration, parar: &AtomicBool) {
    let fim = Instant::now() + intervalo;
    while !parar.load(Ordering::Relaxed) {
        let falta = fim.saturating_duration_since(Instant::now());
        if falta.is_zero() {
            break;
        }
        std::thread::sleep(falta.min(Duration::from_millis(200)));
    }
}

fn devolver(rapl: Option<&Rapl>, ventoinhas: &[Ventoinha], original: &Original) {
    if let (Some(r), Some(uw)) = (rapl, original.pl1_uw) {
        match r.definir_pl1_uw(uw) {
            Ok(()) => println!("↩️ PL1 devolvido a {} W", uw_para_w(uw)),
            Err(e) => eprintln!("⚠️ não devolvi o PL1: {e}"),
        }
    }
    for &(i, piso) in &original.pisos {
        if let Some(v) = ventoinhas.get(i)
            && let Err(e) = v.definir_piso(piso)
        {
            eprintln!("⚠️ não devolvi o piso da ventoinha {}: {e}", v.numero);
        }
    }
    println!("👋 termostato parado");
}

fn servir(ouvinte: &TcpListener, estado: &Mutex<Estado>, validade: Duration) {
    for conexao in ouvinte.incoming().flatten() {
        let _ = responder(conexao, estado, validade);
    }
}

fn responder(
    mut conexao: TcpStream,
    estado: &Mutex<Estado>,
    validade: Duration,
) -> std::io::Result<()> {
    conexao.set_read_timeout(Some(Duration::from_secs(2)))?;
    // Lê até o fim do cabeçalho: responder no primeiro pedaço e fechar com o
    // resto por ler faz o kernel mandar RST, e o cliente vê "connection reset".
    let mut pedido = Vec::with_capacity(512);
    let mut pedaco = [0u8; 512];
    while !pedido.windows(4).any(|j| j == b"\r\n\r\n") && pedido.len() < 8192 {
        let n = conexao.read(&mut pedaco)?;
        if n == 0 {
            break;
        }
        pedido.extend_from_slice(&pedaco[..n]);
    }
    let linha = String::from_utf8_lossy(&pedido);
    let caminho = linha.split_whitespace().nth(1).unwrap_or("/");
    let (retrato, fresca) = {
        let e = estado
            .lock()
            .map_err(|_| std::io::Error::other("estado envenenado"))?;
        (
            e.retrato.clone(),
            e.ultima_leitura.is_some_and(|t| t.elapsed() <= validade),
        )
    };
    let (status, tipo, corpo) = match caminho.split('?').next().unwrap_or("/") {
        "/metrics" => (
            "200 OK",
            "text/plain; version=0.0.4; charset=utf-8",
            retrato.em_texto(),
        ),
        "/saude" if fresca => ("200 OK", "text/plain; charset=utf-8", "ok\n".into()),
        "/saude" => (
            "503 Service Unavailable",
            "text/plain; charset=utf-8",
            "sem leitura recente\n".into(),
        ),
        _ => (
            "404 Not Found",
            "text/plain; charset=utf-8",
            "use /metrics ou /saude\n".into(),
        ),
    };
    write!(
        conexao,
        "HTTP/1.1 {status}\r\nContent-Type: {tipo}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{corpo}",
        corpo.len()
    )
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn watts_e_microwatts() {
        assert_eq!(w_para_uw(38.0), 38_000_000);
        assert!((uw_para_w(25_000_000) - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn reaplica_o_pl1_mudado_por_fora_e_mede() {
        let d = tempfile::tempdir().unwrap();
        hardware::sysfs_de_teste::montar(d.path(), 70);
        let mut r = Rapl::achar(d.path()).unwrap();
        let mut retrato = Retrato::default();
        cuidar_do_rapl(&mut r, w_para_uw(38.0), &mut retrato);
        assert_eq!(r.pl1_uw().unwrap(), 38_000_000);
        assert_eq!(retrato.pl1_w, Some(38.0));
        assert_eq!(retrato.pl2_w, Some(125.0));
        // Alguém (o tuned) volta o PL1 a 100 W: a próxima volta corrige.
        r.definir_pl1_uw(100_000_000).unwrap();
        cuidar_do_rapl(&mut r, w_para_uw(38.0), &mut retrato);
        assert_eq!(r.pl1_uw().unwrap(), 38_000_000);
        assert_eq!(retrato.falhas_de_leitura, 0);
    }

    #[test]
    fn ventoinha_vai_ao_maximo_e_volta_ao_original() {
        let d = tempfile::tempdir().unwrap();
        hardware::sysfs_de_teste::montar(d.path(), 70);
        let vs = hardware::ventoinhas(d.path());
        let original = Original {
            pl1_uw: None,
            pisos: vec![(0, 2000), (1, 2000)],
        };
        let mut retrato = Retrato::default();
        for v in &vs {
            cuidar_da_ventoinha(v, &mut retrato);
        }
        assert_eq!(vs[0].piso().unwrap(), 6156);
        assert_eq!(vs[1].piso().unwrap(), 5700);
        devolver(None, &vs, &original);
        assert_eq!(vs[0].piso().unwrap(), 2000);
    }

    #[test]
    fn metrics_e_saude_por_http() {
        let ouvinte = TcpListener::bind("127.0.0.1:0").unwrap();
        let endereco = ouvinte.local_addr().unwrap();
        let estado = Arc::new(Mutex::new(Estado {
            retrato: Retrato {
                temperatura_c: Some(81.0),
                ..Retrato::default()
            },
            ultima_leitura: Some(Instant::now()),
        }));
        let e2 = Arc::clone(&estado);
        std::thread::spawn(move || servir(&ouvinte, &e2, Duration::from_secs(6)));
        let pedir = |caminho: &str| {
            let mut c = TcpStream::connect(endereco).unwrap();
            write!(c, "GET {caminho} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut s = String::new();
            c.read_to_string(&mut s).unwrap();
            s
        };
        let m = pedir("/metrics");
        assert!(m.starts_with("HTTP/1.1 200"));
        assert!(m.contains("termostato_temperatura_celsius 81"));
        assert!(pedir("/saude").starts_with("HTTP/1.1 200"));
        estado.lock().unwrap().ultima_leitura = None;
        assert!(pedir("/saude").starts_with("HTTP/1.1 503"));
        assert!(pedir("/outra").starts_with("HTTP/1.1 404"));
    }
}

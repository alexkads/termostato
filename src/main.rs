//! O termostato da fedora (MacBookPro11,2, i7-4770HQ).
//!
//! Um serviço só, dono de toda a parte térmica, para ninguém mexer no sistema
//! operacional: limite de potência sustentada (RAPL PL1), freio de calor,
//! piso das ventoinhas e as métricas que o Prometheus lê em `/metrics`.
//!
//! Ao parar (SIGTERM, SIGINT, crash), o PL1 fica no valor seguro: um reinício
//! nunca deixa a máquina sem limite. Os valores de fábrica só voltam com
//! `termostato --desinstalar` (ver `fabrica.rs`).
//!
//! Os superpoderes (0.5): o guardião vê os processos, aprende quem esquenta e
//! freia antes, lê a pressão de memória, segura os mal-educados no cgroup deles
//! (no modo observar, só mostra o que faria) e avisa o Grafana de tudo.
//!
//!   termostato [config]                 # o serviço
//!   termostato --desinstalar [config]   # devolve a fábrica e apaga o estado
//!   sudo termostato CPU50C              # ajustes com o serviço rodando
//!   sudo termostato chave=valor …       # (ver `ajustes.rs`)
//!   termostato ajustes                  # o que está em vigor

mod ajustes;
mod avisos;
mod config;
mod fabrica;
mod freio;
mod gatilhos;
mod guardiao;
mod hardware;
mod historico;
mod kernel;
mod maos;
mod metricas;
mod pressao;
mod processos;
mod tcc;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use avisos::{Avisador, Aviso};
use config::{Config, Ventoinhas};
use freio::{Freio, Mudanca};
use gatilhos::Alarme;
use guardiao::Guardiao;
use hardware::{Rapl, Sensor, Ventoinha};
use kernel::Kernel;
use metricas::{Retrato, VentoinhaLida};

const CONFIG_PADRAO: &str = "/etc/termostato.toml";

fn w_para_uw(w: f64) -> u64 {
    (w * 1_000_000.0).round() as u64
}

fn uw_para_w(uw: u64) -> f64 {
    uw as f64 / 1_000_000.0
}

use fabrica::{Fabrica, Piso};

struct Estado {
    retrato: Retrato,
    ultima_leitura: Option<Instant>,
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(pedido) = ajustes::interpretar(&args) {
        let caminho = std::env::var_os("TERMOSTATO_CONFIG")
            .map_or_else(|| PathBuf::from(CONFIG_PADRAO), PathBuf::from);
        if let Err(e) = pedido.and_then(|p| atender(&caminho, p)) {
            eprintln!("❌ {e}");
            std::process::exit(1);
        }
        return;
    }
    let desinstalar = args.first().is_some_and(|a| a == "--desinstalar");
    if desinstalar {
        args.remove(0);
    }
    let caminho = args
        .first()
        .map_or_else(|| PathBuf::from(CONFIG_PADRAO), PathBuf::from);
    let resultado = if desinstalar {
        devolver_a_fabrica(&caminho)
    } else {
        rodar(&caminho)
    };
    if let Err(e) = resultado {
        eprintln!("❌ {e}");
        std::process::exit(1);
    }
}

/// A primeira linha do journal: o que este termostato vai fazer.
fn anunciar(cfg: &Config, achado: &Fabrica, ventoinhas: usize) {
    let achado = achado
        .pl1_uw
        .map_or_else(|| "?".into(), |u| uw_para_w(u).to_string());
    let modo = match cfg.ventoinhas {
        Ventoinhas::Maximo => "no máximo",
        Ventoinhas::Automatico => "automático",
    };
    println!(
        "🌡️ termostato {} · limite normal PL1 {} W (achado {achado} W) · curva de freio: {} °C → {} W até {} °C → {} W, previsão de {} s sobre {} leituras, solta a {} W/s · ventoinhas: {modo} ({ventoinhas}) · métricas em http://{}/metrics",
        env!("CARGO_PKG_VERSION"),
        cfg.pl1_normal_w,
        cfg.freio_comeca_c,
        cfg.pl1_normal_w,
        cfg.freio_acima_c,
        cfg.pl1_freio_w,
        cfg.previsao_s,
        cfg.janela_leituras,
        cfg.subida_w_por_s,
        cfg.escuta,
    );
}

// A subida e o laço em sequência: dividir espalharia a ordem que importa
// (fábrica, sinais, HTTP, guardião, laço, parada segura).
#[allow(clippy::too_many_lines)]
fn rodar(caminho: &Path) -> Result<(), String> {
    let base = Config::ler(caminho)?;
    let estado_dir = PathBuf::from(&base.estado);
    let mut ajustes_lidos = ajustes::texto(&estado_dir);
    let mut ajustes_em_vigor = toml::Table::new();
    let mut cfg = match ajustes::tabela(&ajustes_lidos)
        .and_then(|t| ajustes::config_com(caminho, &t).map(|c| (t, c)))
    {
        Ok((t, c)) => {
            for (k, v) in ajustes::achatar(&t) {
                println!("🎛️ ajuste em vigor: {k} = {v}");
            }
            ajustes_em_vigor = t;
            c
        }
        Err(e) => {
            eprintln!("⚠️ ajustes ignorados: {e}");
            base.clone()
        }
    };
    let inicial = cfg.clone();
    let sysfs = PathBuf::from(&cfg.sysfs);
    let sensor = esperar_o_sensor(&sysfs)?;
    let mut rapl = match Rapl::achar(&sysfs) {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("⚠️ {e}: sigo sem limite de potência, só medindo");
            None
        }
    };
    // Subindo cedo no boot, o `applesmc` pode ainda não ter carregado: o laço
    // procura as ventoinhas de novo enquanto não as encontrar.
    let mut ventoinhas = hardware::ventoinhas(&sysfs);

    let achado = Fabrica {
        pl1_uw: rapl.as_ref().and_then(|r| r.pl1_uw().ok()),
        pisos: ventoinhas
            .iter()
            .filter_map(|v| {
                v.piso().ok().map(|rpm| Piso {
                    ventoinha: v.numero.clone(),
                    rpm,
                })
            })
            .collect(),
        tcc_offset_c: None,
    };
    anunciar(&cfg, &achado, ventoinhas.len());
    match fabrica::guardar_se_faltar(Path::new(&cfg.estado), &achado) {
        Ok(true) => println!(
            "💾 valores de fábrica guardados em {} (só o --desinstalar os devolve)",
            fabrica::arquivo(Path::new(&cfg.estado)).display()
        ),
        Ok(false) => {}
        Err(e) => eprintln!("⚠️ não guardei os valores de fábrica: {e}"),
    }

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

    let (avisador, sem_grafana) = Avisador::novo(&cfg.grafana);
    if let Some(motivo) = sem_grafana {
        eprintln!("⚠️ {motivo}");
    }
    let (mut kernel, tcc_de_antes) = Kernel::novo(&cfg, &avisador);
    // O TCC offset de fábrica entra no arquivo só se ele ainda não existia.
    if tcc_de_antes.is_some() {
        let estado_dir = Path::new(&cfg.estado);
        if let Ok(Some(mut f)) = fabrica::ler(estado_dir)
            && f.tcc_offset_c.is_none()
        {
            f.tcc_offset_c = Some(0);
            let _ = fabrica::regravar(estado_dir, &f);
        }
    }
    let mut guardiao = Guardiao::novo(&cfg, avisador);
    guardiao.avisador().avisar(Aviso::novo(
        format!(
            "🌡️ termostato {} subiu (mãos: {:?})",
            env!("CARGO_PKG_VERSION"),
            cfg.maos.modo
        ),
        &["subiu"],
    ));

    let mut freio = Freio::default();
    let mut retrato = Retrato::default();
    let mut anterior = Instant::now();
    let inicio = anterior;

    while !parar.load(Ordering::Relaxed) {
        let agora = Instant::now();
        let dt = agora.duration_since(anterior).as_secs_f64();
        anterior = agora;
        retrato.leituras += 1;

        let agora_s = agora.duration_since(inicio).as_secs_f64();
        let texto = ajustes::texto(&estado_dir);
        if texto != ajustes_lidos {
            ajustes_lidos = texto;
            recarregar(
                caminho,
                &ajustes_lidos,
                &inicial,
                &mut ajustes_em_vigor,
                &mut cfg,
                &mut guardiao,
                &mut retrato,
            );
        }
        retrato.alvo_c = cfg.alvo_c;
        retrato.ajustes = ajustes::achatar(&ajustes_em_vigor).len();
        match sensor.celsius() {
            Ok(t) => {
                let prevista_processos = guardiao.antes_do_freio(agora_s, t, &cfg);
                let d = freio.ler(agora_s, t, prevista_processos, &cfg);
                registrar_o_freio(t, &d, &cfg, &mut retrato, guardiao.avisador());
                guardiao.depois_do_freio(agora_s, t, &d, &cfg, &mut retrato);
            }
            Err(e) => {
                retrato.falhas_de_leitura += 1;
                retrato.temperatura_c = None;
                eprintln!("⚠️ temperatura: {e}");
            }
        }
        if freio.acionado() {
            retrato.segundos_acionado += dt;
        }
        retrato.freio_acionado = freio.acionado();
        retrato.pl1_alvo_w = freio.pl1_w(&cfg);

        if let Some(r) = rapl.as_mut() {
            cuidar_do_rapl(
                r,
                w_para_uw(retrato.pl1_alvo_w),
                &mut retrato,
                guardiao.avisador(),
            );
        }
        if cfg.ventoinhas == Ventoinhas::Maximo {
            if ventoinhas.is_empty() {
                ventoinhas = hardware::ventoinhas(&sysfs);
                if !ventoinhas.is_empty() {
                    println!(
                        "🌀 {} ventoinhas apareceram (applesmc carregou)",
                        ventoinhas.len()
                    );
                }
            }
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
        kernel.a_cada_volta(agora_s, guardiao.avisador(), &mut retrato);

        if let Ok(mut e) = estado.lock() {
            e.retrato = retrato.clone();
            e.ultima_leitura = Some(Instant::now());
        }
        esperar(
            Duration::from_secs_f64(cfg.intervalo_s),
            &parar,
            kernel.alarme(),
        );
    }

    guardiao.parar();
    parar_seguro(rapl.as_ref(), &cfg);
    Ok(())
}

/// O arquivo de ajustes mudou: confere, troca a config e avisa o que mudou.
/// Ajuste inválido (alguém editou à mão) fica de fora, e vale o de antes.
fn recarregar(
    caminho: &Path,
    texto: &str,
    inicial: &Config,
    em_vigor: &mut toml::Table,
    cfg: &mut Config,
    guardiao: &mut Guardiao,
    retrato: &mut Retrato,
) {
    let lido =
        ajustes::tabela(texto).and_then(|t| ajustes::config_com(caminho, &t).map(|c| (t, c)));
    let (tabela, mut nova) = match lido {
        Ok(par) => par,
        Err(e) => {
            let texto = format!("⚠️ ajustes recusados, sigo com os de antes: {e}");
            eprintln!("{texto}");
            retrato.contar("ajuste_recusado");
            guardiao
                .avisador()
                .avisar(Aviso::novo(texto, &["ajuste", "recusado"]));
            return;
        }
    };
    for campo in ajustes::manter_os_da_subida(inicial, &mut nova) {
        eprintln!("⚠️ {campo} só muda reiniciando o serviço: ignorado");
    }
    let mudou = ajustes::diferencas(em_vigor, &tabela);
    if cfg.maos.modo == config::Modo::Agir && nova.maos.modo != config::Modo::Agir {
        guardiao.soltar_as_maos();
    }
    *cfg = nova;
    *em_vigor = tabela;
    if mudou.is_empty() {
        return;
    }
    retrato.contar("ajuste");
    let texto = format!("🎛️ ajuste: {}", mudou.join(", "));
    println!("{texto}");
    guardiao.avisador().avisar(Aviso::novo(texto, &["ajuste"]));
}

/// `termostato CPU50C`, `chave=valor`, `ajustes`, `padrao`: grava o pedido no
/// estado, para o serviço aplicar na próxima volta.
fn atender(caminho: &Path, pedido: ajustes::Pedido) -> Result<(), String> {
    let base = Config::ler(caminho)?;
    let estado = PathBuf::from(&base.estado);
    let atual = ajustes::tabela(&ajustes::texto(&estado))?;
    let novo = match pedido {
        ajustes::Pedido::Mostrar => {
            let cfg = ajustes::config_com(caminho, &atual)?;
            let lista = ajustes::achatar(&atual);
            if lista.is_empty() {
                println!("sem ajustes: vale só {}", caminho.display());
            }
            for (k, v) in lista {
                println!("🎛️ {k} = {v}");
            }
            if cfg.alvo_c > 0.0 {
                println!(
                    "🎯 alvo: CPU em {} °C (PL1 entre {} e {} W)",
                    cfg.alvo_c, cfg.pl1_minimo_w, cfg.pl1_normal_w
                );
            } else {
                println!(
                    "🎯 sem alvo: só a curva ({} °C → {} °C)",
                    cfg.freio_comeca_c, cfg.freio_acima_c
                );
            }
            return Ok(());
        }
        ajustes::Pedido::Limpar => toml::Table::new(),
        ajustes::Pedido::Ajustar(pares) => {
            let mut t = atual.clone();
            for (k, v) in pares {
                ajustes::aplicar(&mut t, &k, v);
            }
            t
        }
    };
    let cfg = ajustes::config_com(caminho, &novo)?;
    ajustes::gravar(&estado, &novo)?;
    let mudou = ajustes::diferencas(&atual, &novo);
    if mudou.is_empty() {
        println!("nada mudou");
    }
    for linha in &mudou {
        println!("🎛️ {linha}");
    }
    if cfg.alvo_c > 0.0 {
        println!("🎯 alvo: CPU em {} °C", cfg.alvo_c);
    }
    println!(
        "✅ gravado em {}: o serviço aplica em até {} s",
        ajustes::arquivo(&estado).display(),
        cfg.intervalo_s
    );
    Ok(())
}

/// Subindo cedo no boot, o `coretemp` pode ainda não ter carregado: espera
/// até 30 s por ele antes de desistir (e o systemd tenta de novo em 3 s).
fn esperar_o_sensor(sysfs: &Path) -> Result<Sensor, String> {
    let mut ultimo = String::new();
    for tentativa in 0..30 {
        match Sensor::achar(sysfs) {
            Ok(s) => {
                if tentativa > 0 {
                    println!("🌡️ sensor encontrado depois de {tentativa} s");
                }
                return Ok(s);
            }
            Err(e) => ultimo = e,
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    Err(ultimo)
}

/// O que a curva decidiu: retrato, journal e Grafana.
fn registrar_o_freio(
    t: f64,
    d: &freio::Decisao,
    cfg: &Config,
    retrato: &mut Retrato,
    avisador: &Avisador,
) {
    retrato.temperatura_c = Some(t);
    retrato.temperatura_prevista_c = Some(d.prevista_c);
    retrato.tendencia_c_s = Some(d.tendencia_c_s);
    retrato.freio_intensidade = d.intensidade;
    let texto = match d.mudanca {
        Some(Mudanca::Acionou) if d.pelo_alvo => {
            retrato.acionamentos += 1;
            retrato.contar("freio_acionado");
            format!(
                "🎯 {t:.0} °C, acima do alvo de {} °C: freio acionado, PL1 em {:.1} W",
                cfg.alvo_c, d.pl1_w
            )
        }
        Some(Mudanca::Acionou) => {
            retrato.acionamentos += 1;
            retrato.contar("freio_acionado");
            format!(
                "🔥 {t:.0} °C agora, {:.0} °C previstos em {} s ({:+.1} °C/s): freio de calor acionado, PL1 em {:.1} W",
                d.prevista_c, cfg.previsao_s, d.tendencia_c_s, d.pl1_w
            )
        }
        Some(Mudanca::Desligou) => {
            retrato.contar("freio_desligado");
            format!(
                "❄️ {t:.0} °C: freio de calor desligado, de volta ao limite normal de {} W",
                cfg.pl1_normal_w
            )
        }
        None => return,
    };
    println!("{texto}");
    let tag = if d.mudanca == Some(Mudanca::Acionou) {
        "acionado"
    } else {
        "desligado"
    };
    avisador.avisar(Aviso::novo(texto, &["freio", tag]));
}

/// Grava o PL1 quando ele não é o alvo: na subida, a cada mudança do freio, e
/// quando outro programa (o `tuned`, um script) o mudou por fora.
fn cuidar_do_rapl(r: &mut Rapl, alvo_uw: u64, retrato: &mut Retrato, avisador: &Avisador) {
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
                let texto = format!(
                    "🔧 PL1 estava em {} W: reaplicado {} W (outro programa mexeu)",
                    uw_para_w(atual),
                    uw_para_w(alvo_uw)
                );
                println!("{texto}");
                retrato.contar("pl1_reaplicado");
                avisador.avisar(Aviso::novo(texto, &["pl1"]));
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
fn esperar(intervalo: Duration, parar: &AtomicBool, alarme: Option<&Alarme>) {
    let fim = Instant::now() + intervalo;
    while !parar.load(Ordering::Relaxed) {
        // O kernel avisou que a memória apertou: a próxima volta é agora.
        if alarme.is_some_and(|a| a.tocou.load(Ordering::Relaxed)) {
            break;
        }
        let falta = fim.saturating_duration_since(Instant::now());
        if falta.is_zero() {
            break;
        }
        std::thread::sleep(falta.min(Duration::from_millis(100)));
    }
}

/// Ao parar: o PL1 vai ao valor seguro (se o freio estava acionado, sai dos
/// 25 W) e as ventoinhas ficam como estão. Nada volta à fábrica aqui.
fn parar_seguro(rapl: Option<&Rapl>, cfg: &Config) {
    if let Some(r) = rapl
        && let Err(e) = r.definir_pl1_uw(w_para_uw(cfg.pl1_normal_w))
    {
        eprintln!("⚠️ PL1 ao parar: {e}");
    }
    println!(
        "👋 termostato parado com o PL1 em {} W (a fábrica só volta com --desinstalar)",
        cfg.pl1_normal_w
    );
}

/// `--desinstalar`: devolve o PL1 e os pisos guardados na primeira subida e
/// apaga o estado. Sem estado, não há o que devolver, e o hardware fica.
fn devolver_a_fabrica(caminho: &Path) -> Result<(), String> {
    let cfg = Config::ler(caminho)?;
    let estado = PathBuf::from(&cfg.estado);
    let Some(f) = fabrica::ler(&estado)? else {
        println!("sem valores de fábrica guardados: nada a devolver");
        return Ok(());
    };
    devolver(&cfg, &f)?;
    ajustes::apagar(&estado)?;
    fabrica::apagar(&estado)
}

fn devolver(cfg: &Config, f: &Fabrica) -> Result<(), String> {
    let sysfs = &PathBuf::from(&cfg.sysfs);
    if let Some(o) = f.tcc_offset_c {
        match tcc::Msr::achar(Path::new(&cfg.dev_cpu)).and_then(|m| m.definir_offset(o)) {
            Ok(tjmax) => println!(
                "↩️ TCC offset devolvido a {o} °C: o chip volta a se frear só a {} °C",
                tjmax.saturating_sub(o)
            ),
            Err(e) => eprintln!("⚠️ não devolvi o TCC offset: {e} (ele volta a 0 no próximo boot)"),
        }
    }
    if let Some(uw) = f.pl1_uw {
        Rapl::achar(sysfs)?.definir_pl1_uw(uw)?;
        println!("↩️ PL1 devolvido a {} W", uw_para_w(uw));
    }
    let ventoinhas = hardware::ventoinhas(sysfs);
    for p in &f.pisos {
        if let Some(v) = ventoinhas.iter().find(|v| v.numero == p.ventoinha) {
            v.definir_piso(p.rpm)?;
            println!("↩️ ventoinha {}: piso devolvido a {} RPM", v.numero, p.rpm);
        }
    }
    Ok(())
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
        cuidar_do_rapl(&mut r, w_para_uw(38.0), &mut retrato, &Avisador::mudo());
        assert_eq!(r.pl1_uw().unwrap(), 38_000_000);
        assert_eq!(retrato.pl1_w, Some(38.0));
        assert_eq!(retrato.pl2_w, Some(125.0));
        // Alguém (o tuned) volta o PL1 a 100 W: a próxima volta corrige.
        r.definir_pl1_uw(100_000_000).unwrap();
        cuidar_do_rapl(&mut r, w_para_uw(38.0), &mut retrato, &Avisador::mudo());
        assert_eq!(r.pl1_uw().unwrap(), 38_000_000);
        assert_eq!(retrato.falhas_de_leitura, 0);
    }

    #[test]
    fn parar_deixa_o_valor_seguro_e_nao_a_fabrica() {
        let d = tempfile::tempdir().unwrap();
        hardware::sysfs_de_teste::montar(d.path(), 95);
        let r = Rapl::achar(d.path()).unwrap();
        // O freio estava acionado (25 W) quando o SIGTERM chegou.
        r.definir_pl1_uw(25_000_000).unwrap();
        parar_seguro(Some(&r), &Config::default());
        assert_eq!(
            r.pl1_uw().unwrap(),
            38_000_000,
            "nem 25 W nem os 100 W de fábrica"
        );
    }

    #[test]
    fn desinstalar_devolve_a_fabrica_e_apaga_o_estado() {
        let sys = tempfile::tempdir().unwrap();
        let est = tempfile::tempdir().unwrap();
        hardware::sysfs_de_teste::montar(sys.path(), 70);
        let vs = hardware::ventoinhas(sys.path());
        let mut retrato = Retrato::default();
        for v in &vs {
            cuidar_da_ventoinha(v, &mut retrato);
        }
        assert_eq!(vs[0].piso().unwrap(), 6156);
        let f = Fabrica {
            pl1_uw: Some(100_000_000),
            pisos: vec![Piso {
                ventoinha: "1".into(),
                rpm: 2000,
            }],
            tcc_offset_c: None,
        };
        fabrica::guardar_se_faltar(est.path(), &f).unwrap();
        let toml = est.path().join("t.toml");
        std::fs::write(
            &toml,
            format!(
                "sysfs = {:?}\nestado = {:?}\n",
                sys.path().display().to_string(),
                est.path().display().to_string()
            ),
        )
        .unwrap();
        devolver_a_fabrica(&toml).unwrap();
        assert_eq!(
            Rapl::achar(sys.path()).unwrap().pl1_uw().unwrap(),
            100_000_000
        );
        assert_eq!(vs[0].piso().unwrap(), 2000);
        assert_eq!(vs[1].piso().unwrap(), 5700, "sem piso guardado, fica");
        assert_eq!(fabrica::ler(est.path()).unwrap(), None);
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

//! A caixa-preta: o que aconteceu até o último segundo antes de a máquina
//! parar, e a explicação no boot seguinte.
//!
//! A cada leitura, uma linha JSON vai para um arquivo por hora em
//! `<estado>/caixa-preta/<hora unix>.jsonl`, com `fsync` a cada 10 s. Ficam as
//! últimas `horas`, até `mb_max`. O journal não basta: quando a máquina trava,
//! o que ainda não foi gravado no disco se perde, e o journal do kernel só
//! conta o OOM, não quem estava esquentando, trocando swap ou drenando a
//! bateria.
//!
//! Ao parar limpo, o serviço grava o marcador `parada-limpa`. Se ele não está lá
//! na subida seguinte, houve queda: o journal recebe um 💥 com a hora e a última
//! leitura, o Grafana recebe a anotação, e o último minuto da caixa-preta vai
//! para `<estado>/quedas/`. O `boot_id` em cada linha separa a máquina que
//! travou (outro boot) do serviço que foi morto (o mesmo boot).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::metricas::Retrato;

const MARCADOR: &str = "parada-limpa";
const SINCRONIZAR_S: f64 = 10.0;
/// Quantas linhas do fim vão para o arquivo da queda (30 × 2 s = 1 min).
const LINHAS_DA_QUEDA: usize = 30;

pub struct CaixaPreta {
    pasta: PathBuf,
    horas: u64,
    mb_max: u64,
    aberto: Option<(u64, File)>,
    ultimo_sync: f64,
}

impl CaixaPreta {
    pub fn abrir(estado: &Path, horas: u64, mb_max: u64) -> Result<Self, String> {
        let pasta = estado.join("caixa-preta");
        fs::create_dir_all(&pasta).map_err(|e| format!("{}: {e}", pasta.display()))?;
        Ok(Self {
            pasta,
            horas,
            mb_max,
            aberto: None,
            ultimo_sync: 0.0,
        })
    }

    /// Grava uma linha. Troca de arquivo a cada hora e poda os antigos.
    pub fn gravar(&mut self, agora: f64, linha: &str) -> Result<(), String> {
        let hora = (agora / 3600.0) as u64;
        if self.aberto.as_ref().is_none_or(|(h, _)| *h != hora) {
            self.sincronizar();
            let caminho = self.pasta.join(format!("{hora}.jsonl"));
            let f = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&caminho)
                .map_err(|e| format!("{}: {e}", caminho.display()))?;
            self.aberto = Some((hora, f));
            self.podar(hora);
        }
        if let Some((_, f)) = self.aberto.as_mut() {
            writeln!(f, "{linha}").map_err(|e| format!("caixa-preta: {e}"))?;
        }
        if agora - self.ultimo_sync >= SINCRONIZAR_S {
            self.sincronizar();
            self.ultimo_sync = agora;
        }
        Ok(())
    }

    pub fn sincronizar(&mut self) {
        if let Some((_, f)) = self.aberto.as_ref() {
            let _ = f.sync_data();
        }
    }

    /// Tira as horas velhas e, se passar do teto, as mais antigas. A hora atual
    /// nunca sai.
    fn podar(&self, hora_atual: u64) {
        let mut arquivos = arquivos(&self.pasta);
        arquivos.retain(|(h, p)| {
            let velho = *h + self.horas <= hora_atual;
            if velho {
                let _ = fs::remove_file(p);
            }
            !velho
        });
        let teto = self.mb_max * 1024 * 1024;
        let mut total: u64 = arquivos
            .iter()
            .filter_map(|(_, p)| fs::metadata(p).ok())
            .map(|m| m.len())
            .sum();
        for (h, p) in &arquivos {
            if total <= teto || *h == hora_atual {
                break;
            }
            total = total.saturating_sub(fs::metadata(p).map_or(0, |m| m.len()));
            let _ = fs::remove_file(p);
        }
    }
}

/// Os arquivos da caixa-preta, do mais antigo ao mais novo.
fn arquivos(pasta: &Path) -> Vec<(u64, PathBuf)> {
    let mut v: Vec<(u64, PathBuf)> = fs::read_dir(pasta)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| {
            let nome = e.file_name().into_string().ok()?;
            let hora = nome.strip_suffix(".jsonl")?.parse().ok()?;
            Some((hora, e.path()))
        })
        .collect();
    v.sort_by_key(|(h, _)| *h);
    v
}

/// As últimas `n` linhas válidas, da mais antiga à mais nova.
pub fn ultimas(estado: &Path, n: usize) -> Vec<Value> {
    let mut v: Vec<Value> = Vec::new();
    for (_, p) in arquivos(&estado.join("caixa-preta")).iter().rev() {
        let texto = fs::read_to_string(p).unwrap_or_default();
        let mut daqui: Vec<Value> = texto
            .lines()
            .rev()
            .filter_map(|l| serde_json::from_str(l).ok())
            .take(n - v.len())
            .collect();
        daqui.reverse();
        daqui.extend(v);
        v = daqui;
        if v.len() >= n {
            break;
        }
    }
    v
}

pub fn marcar_parada_limpa(estado: &Path) {
    let _ = fs::write(estado.join(MARCADOR), "ok\n");
}

/// O que se sabe da queda.
#[derive(Debug, Clone, PartialEq)]
pub struct Queda {
    pub em: f64,
    /// O serviço foi morto, e a máquina seguiu ligada.
    pub mesmo_boot: bool,
    pub ultimas: Vec<Value>,
}

/// Na subida: houve queda desde a última parada? Tira o marcador, para a
/// próxima queda ser vista. Sem caixa-preta gravada (primeira subida), não.
pub fn verificar_queda(estado: &Path, boot_atual: &str) -> Option<Queda> {
    let marcador = estado.join(MARCADOR);
    let limpa = marcador.exists();
    let _ = fs::remove_file(&marcador);
    if limpa {
        return None;
    }
    let ultimas = ultimas(estado, LINHAS_DA_QUEDA);
    let ultima = ultimas.last()?;
    Some(Queda {
        em: ultima["t"].as_f64().unwrap_or(0.0),
        mesmo_boot: !boot_atual.is_empty() && ultima["boot"].as_str() == Some(boot_atual),
        ultimas,
    })
}

/// Guarda o último minuto em `<estado>/quedas/<hora>.json`.
pub fn guardar_queda(estado: &Path, q: &Queda, texto: &str) -> Result<PathBuf, String> {
    let pasta = estado.join("quedas");
    fs::create_dir_all(&pasta).map_err(|e| format!("{}: {e}", pasta.display()))?;
    let caminho = pasta.join(format!("{}.json", q.em as u64));
    let corpo = json!({ "texto": texto, "mesmo_boot": q.mesmo_boot, "ultimas": q.ultimas });
    fs::write(&caminho, format!("{corpo:#}\n"))
        .map_err(|e| format!("{}: {e}", caminho.display()))?;
    Ok(caminho)
}

fn arredondar(v: f64, casas: i32) -> f64 {
    let f = 10f64.powi(casas);
    (v * f).round() / f
}

/// Uma linha da caixa-preta, a partir do retrato da volta.
pub fn linha(r: &Retrato, agora: f64, boot: &str) -> String {
    let a = |v: Option<f64>| v.map(|x| arredondar(x, 1));
    let p = r.pressao.as_ref();
    let motivos = r
        .motivos
        .as_ref()
        .map(|m| json!({ "agora": m.agora(), "desde_o_boot": m.desde_o_boot() }));
    let bateria = r.bateria.as_ref().map(|b| {
        json!({
            "carregador": b.carregador_ligado,
            "estado": b.estado,
            "a": b.amperes.map(|x| arredondar(x, 2)),
            "pct": b.carga_pct,
        })
    });
    json!({
        "t": arredondar(agora, 1),
        "boot": boot,
        "chip": a(r.temperatura_c),
        "nucleos": r.nucleos_c.iter().map(|(_, c)| arredondar(*c, 0)).collect::<Vec<_>>(),
        "pl1_w": arredondar(r.pl1_alvo_w, 1),
        "w": a(r.potencia_w),
        "mhz": r.frequencia_mhz.map(f64::round),
        "ventoinhas": r.ventoinhas.iter().map(|v| v.rpm).collect::<Vec<_>>(),
        "freio": arredondar(r.freio_intensidade, 2),
        "estrangulamento": r.estrangulamento.map(|e| e.0),
        "motivos": motivos,
        "bateria": bateria,
        "psi": p.map(|p| json!({
            "mem": p.memoria_some_pct,
            "mem_full": p.memoria_full_pct,
            "cpu": p.cpu_some_pct,
            "io": p.io_some_pct,
        })),
        "mem_disp_pct": p.map(|p| arredondar(p.memoria_disponivel_pct, 1)),
        "swap_usado_mb": p.map(|p| p.swap_usado_mb.round()),
        "swap_mb_s": r.swap_mb_s.map(|(e, s)| json!({ "entra": arredondar(e, 1), "sai": arredondar(s, 1) })),
        "oom": r.oom_mortes,
        "cpu_top": r.processos.iter().take(3).map(|p| json!([p.nome, arredondar(p.fatia, 2)])).collect::<Vec<_>>(),
        "mem_top": r.memoria_maiores.iter().take(3).map(|m| json!([m.nome, m.copias, m.rss_mb.round(), m.swap_mb.round()])).collect::<Vec<_>>(),
    })
    .to_string()
}

/// O fuso local em segundos, pelo rodapé POSIX do `/etc/localtime` (TZif v2+),
/// como `<-03>3` ou `EST5EDT,…`. Sem ele, UTC. O horário de verão é ignorado.
pub fn fuso_s(tzif: &[u8]) -> i64 {
    let texto = String::from_utf8_lossy(tzif);
    let Some(rodape) = texto.trim_end_matches('\n').rsplit('\n').next() else {
        return 0;
    };
    let resto = if let Some(r) = rodape.strip_prefix('<') {
        r.split_once('>').map_or("", |(_, r)| r)
    } else {
        rodape.trim_start_matches(|c: char| c.is_ascii_alphabetic())
    };
    let fim = resto
        .char_indices()
        .find(|(i, c)| !(c.is_ascii_digit() || ((*c == '-' || *c == '+') && *i == 0)))
        .map_or(resto.len(), |(i, _)| i);
    // No POSIX, "3" quer dizer 3 h a OESTE de Greenwich: UTC−3.
    resto[..fim].parse::<i64>().map_or(0, |h| -h * 3600)
}

/// `AAAA-MM-DD HH:MM:SS` de um instante unix, já no fuso.
pub fn data_hora(unix: i64) -> String {
    let dias = unix.div_euclid(86_400);
    let s = unix.rem_euclid(86_400);
    // Civil a partir de dias (Howard Hinnant).
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let dia = doy - (153 * mp + 2) / 5 + 1;
    let mes = if mp < 10 { mp + 3 } else { mp - 9 };
    let ano = yoe + era * 400 + i64::from(mes <= 2);
    format!(
        "{ano:04}-{mes:02}-{dia:02} {:02}:{:02}:{:02}",
        s / 3600,
        s % 3600 / 60,
        s % 60
    )
}

fn um(v: &Value, casas: usize) -> String {
    v.as_f64()
        .map_or_else(|| "—".into(), |x| format!("{x:.casas$}"))
}

/// A frase do 💥.
pub fn descrever(q: &Queda, fuso: i64) -> String {
    let quando = data_hora(q.em as i64 + fuso);
    let o_que = if q.mesmo_boot {
        format!(
            "o termostato parou às {quando} sem desligar limpo (foi morto; a máquina seguiu ligada)"
        )
    } else {
        format!("a fedora parou às {quando} sem desligar (travou ou perdeu energia)")
    };
    let Some(u) = q.ultimas.last() else {
        return format!("💥 {o_que}");
    };
    let mem_top = u["mem_top"]
        .as_array()
        .map(|v| {
            v.iter()
                .map(|m| {
                    format!(
                        "{} ×{} ({:.1} GB + {:.1} GB de swap)",
                        m[0].as_str().unwrap_or("?"),
                        m[1].as_u64().unwrap_or(0),
                        m[2].as_f64().unwrap_or(0.0) / 1024.0,
                        m[3].as_f64().unwrap_or(0.0) / 1024.0
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "—".into());
    let mut partes = vec![
        format!("chip {} °C", um(&u["chip"], 0)),
        format!("memória disponível {} %", um(&u["mem_disp_pct"], 0)),
        format!("PSI de memória {} %", um(&u["psi"]["mem"], 0)),
        format!(
            "swap entrando {} MB/s e saindo {} MB/s",
            um(&u["swap_mb_s"]["entra"], 0),
            um(&u["swap_mb_s"]["sai"], 0)
        ),
    ];
    if let Some(a) = u["bateria"]["a"].as_f64() {
        partes.push(format!("bateria {a:+.2} A"));
    }
    format!(
        "💥 {o_que}. A última leitura: {}. Quem segurava a memória: {mem_top}",
        partes.join(", ")
    )
}

#[cfg(test)]
mod testes {
    use super::*;

    fn caixa(d: &Path, horas: u64, mb: u64) -> CaixaPreta {
        CaixaPreta::abrir(d, horas, mb).unwrap()
    }

    #[test]
    fn grava_uma_linha_por_volta_e_le_o_fim() {
        let d = tempfile::tempdir().unwrap();
        let mut c = caixa(d.path(), 24, 50);
        for i in 0..40 {
            c.gravar(1_000_000.0 + f64::from(i) * 2.0, &format!("{{\"t\":{i}}}"))
                .unwrap();
        }
        let u = ultimas(d.path(), 30);
        assert_eq!(u.len(), 30);
        assert_eq!(u[0]["t"], 10);
        assert_eq!(u[29]["t"], 39);
    }

    #[test]
    fn o_anel_tira_as_horas_velhas_e_respeita_o_teto() {
        let d = tempfile::tempdir().unwrap();
        let mut c = caixa(d.path(), 3, 50);
        for h in 0..6u32 {
            c.gravar(f64::from(h) * 3600.0 + 10.0, "{}").unwrap();
        }
        let horas: Vec<u64> = arquivos(&d.path().join("caixa-preta"))
            .iter()
            .map(|a| a.0)
            .collect();
        assert_eq!(horas, vec![3, 4, 5]);
        // Teto de 1 MB: ~0,5 MB por hora, só cabem duas.
        let d = tempfile::tempdir().unwrap();
        let mut c = caixa(d.path(), 24, 1);
        let grande = format!("\"{}\"", "x".repeat(512 * 1024));
        for h in 0..4u32 {
            c.gravar(f64::from(h) * 3600.0, &grande).unwrap();
        }
        let horas: Vec<u64> = arquivos(&d.path().join("caixa-preta"))
            .iter()
            .map(|a| a.0)
            .collect();
        assert_eq!(horas, vec![2, 3]);
    }

    #[test]
    fn parada_limpa_nao_e_queda_e_sem_marcador_e() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(
            verificar_queda(d.path(), "boot-a"),
            None,
            "sem caixa-preta, não"
        );
        let mut c = caixa(d.path(), 24, 50);
        c.gravar(1_000.0, r#"{"t":1000,"boot":"boot-a","chip":71}"#)
            .unwrap();
        marcar_parada_limpa(d.path());
        assert_eq!(verificar_queda(d.path(), "boot-a"), None);
        // O marcador saiu: a próxima subida sem ele é queda.
        let q = verificar_queda(d.path(), "boot-b").unwrap();
        assert!((q.em - 1000.0).abs() < 1e-9);
        assert!(!q.mesmo_boot);
        let q = verificar_queda(d.path(), "boot-a").unwrap();
        assert!(q.mesmo_boot);
        let arquivo = guardar_queda(d.path(), &q, "texto").unwrap();
        assert!(
            fs::read_to_string(arquivo)
                .unwrap()
                .contains("\"chip\": 71")
        );
    }

    #[test]
    fn descreve_o_travamento_de_19_11() {
        let u: Value = serde_json::from_str(
            r#"{"t":1795123936,"boot":"a","chip":97,"mem_disp_pct":0.4,"psi":{"mem":88.5},
               "swap_mb_s":{"entra":41.2,"sai":63.0},"bateria":{"a":-1.31},
               "mem_top":[["rustc",8,12288,11264],["qemu-system-x86",1,3072,0]]}"#,
        )
        .unwrap();
        let q = Queda {
            em: 1_795_123_936.0,
            mesmo_boot: false,
            ultimas: vec![u],
        };
        let t = descrever(&q, -3 * 3600);
        assert!(
            t.starts_with("💥 a fedora parou às 2026-11-19 18:32:16"),
            "{t}"
        );
        assert!(t.contains("memória disponível 0 %"), "{t}");
        assert!(t.contains("swap entrando 41 MB/s e saindo 63 MB/s"), "{t}");
        assert!(t.contains("bateria -1.31 A"), "{t}");
        assert!(t.contains("rustc ×8 (12.0 GB + 11.0 GB de swap)"), "{t}");
        let q = Queda {
            mesmo_boot: true,
            ..q
        };
        assert!(descrever(&q, 0).contains("foi morto"));
    }

    #[test]
    fn le_o_fuso_do_localtime() {
        assert_eq!(fuso_s(b"TZif2...\n<-03>3\n"), -3 * 3600);
        assert_eq!(fuso_s(b"TZif2...\nEST5EDT,M3.2.0,M11.1.0\n"), -5 * 3600);
        assert_eq!(fuso_s(b"TZif2...\n<+0530>-5:30\n"), 5 * 3600);
        assert_eq!(fuso_s(b"TZif2...\nUTC0\n"), 0);
        assert_eq!(fuso_s(b""), 0);
        assert_eq!(data_hora(0), "1970-01-01 00:00:00");
        assert_eq!(data_hora(1_795_134_736), "2026-11-20 00:32:16");
    }

    #[test]
    fn a_linha_tem_o_que_explica_um_travamento() {
        let mut r = Retrato {
            temperatura_c: Some(97.25),
            ..Retrato::default()
        };
        r.nucleos_c = vec![(0, 96.0), (1, 99.0)];
        r.swap_mb_s = Some((41.24, 0.0));
        r.oom_mortes = Some(4);
        let v: Value = serde_json::from_str(&linha(&r, 1000.04, "boot-a")).unwrap();
        assert_eq!(v["t"], 1000.0);
        assert_eq!(v["boot"], "boot-a");
        assert_eq!(v["chip"], 97.3);
        assert_eq!(v["nucleos"], json!([96.0, 99.0]));
        assert_eq!(v["swap_mb_s"]["entra"], 41.2);
        assert_eq!(v["oom"], 4);
        assert!(v["bateria"].is_null());
    }
}

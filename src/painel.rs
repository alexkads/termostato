//! O painel: a página em `/`, feita em RavenRS (`painel/`), e a API que ela lê.
//!
//! - `GET /` e os arquivos do painel, embutidos no binário (`painel/dist/`): o RPM
//!   não compila wasm, e o painel nunca fica de versão diferente do termostato.
//! - `GET /api/estado`: tudo o que o painel desenha, em JSON.
//! - `POST /api/ajustes`: os mesmos ajustes do `sudo termostato chave=valor`,
//!   conferidos pelas mesmas regras, gravados no mesmo arquivo do estado.
//!
//! Quem pode ajustar: só um pedido que chega pelo `tailscale serve` com o
//! cabeçalho `Tailscale-User-Login` de um login de `[painel] quem_pode_ajustar`.
//! O corpo tem de ser JSON (o navegador faz o preflight de CORS, que este
//! servidor não responde) e o `Origin`, quando vem, tem de ser o próprio host:
//! outra página aberta no navegador do dono não consegue mudar nada.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::Path;

use serde_json::{Value, json};

use crate::ajustes;
use crate::avisos;
use crate::config::Config;
use crate::metricas::Retrato;

/// A página e o app, compilados por `painel/compilar.sh`.
pub const ARQUIVOS: &[(&str, &str, &[u8])] = &[
    (
        "/",
        "text/html; charset=utf-8",
        include_bytes!("../painel/dist/index.html"),
    ),
    (
        "/painel.js",
        "text/javascript; charset=utf-8",
        include_bytes!("../painel/dist/painel.js"),
    ),
    (
        "/painel_bg.wasm",
        "application/wasm",
        include_bytes!("../painel/dist/painel_bg.wasm"),
    ),
];

/// Uma leitura guardada para os gráficos: a cada 10 s, os últimos 10 min.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Amostra {
    pub unix_s: f64,
    pub temperatura_c: f64,
    pub potencia_w: f64,
    pub pl1_w: f64,
}

const INTERVALO_DA_SERIE_S: f64 = 10.0;
const AMOSTRAS: usize = 60;
const HORAS: u64 = 12;

/// O que o painel mostra do passado recente, guardado em memória.
#[derive(Debug, Clone, Default)]
pub struct Historia {
    pub amostras: VecDeque<Amostra>,
    /// Segundos com o freio acionado em cada hora (hora = unix / 3600).
    pub freio_por_hora: BTreeMap<u64, f64>,
}

impl Historia {
    /// A cada volta: guarda uma amostra a cada 10 s e soma o tempo de freio.
    pub fn registrar(&mut self, unix_s: f64, dt: f64, retrato: &Retrato) {
        if retrato.freio_acionado {
            let hora = (unix_s / 3600.0) as u64;
            *self.freio_por_hora.entry(hora).or_default() += dt;
            while self.freio_por_hora.len() as u64 > HORAS {
                self.freio_por_hora.pop_first();
            }
        }
        let vez = self
            .amostras
            .back()
            .is_none_or(|a| unix_s - a.unix_s >= INTERVALO_DA_SERIE_S - 0.5);
        if let (true, Some(t)) = (vez, retrato.temperatura_c) {
            self.amostras.push_back(Amostra {
                unix_s,
                temperatura_c: t,
                potencia_w: retrato.potencia_w.unwrap_or(0.0),
                pl1_w: retrato.pl1_w.unwrap_or(retrato.pl1_alvo_w),
            });
            while self.amostras.len() > AMOSTRAS {
                self.amostras.pop_front();
            }
        }
    }

    /// Os minutos de freio das últimas 12 horas, da mais velha à de agora.
    fn minutos_por_hora(&self, unix_s: f64) -> Vec<(u64, f64)> {
        let agora = (unix_s / 3600.0) as u64;
        (0..HORAS)
            .rev()
            .map(|atras| {
                let h = agora.saturating_sub(atras);
                (
                    h,
                    self.freio_por_hora.get(&h).copied().unwrap_or(0.0) / 60.0,
                )
            })
            .collect()
    }
}

/// A hora do dia (0–23) no fuso local, pelo deslocamento de `TZ`/`localtime`.
fn hora_local(hora_unix: u64, fuso_s: i64) -> u64 {
    (hora_unix.cast_signed() * 3600 + fuso_s)
        .rem_euclid(86_400)
        .cast_unsigned()
        / 3600
}

/// O deslocamento do fuso local em segundos (o Brasil, sem horário de verão,
/// é −3 h). Lido de `/etc/localtime` seria o certo; o `date +%z` resolve sem
/// dependência e roda uma vez só.
pub fn fuso_local_s() -> i64 {
    std::process::Command::new("date")
        .arg("+%z")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|z| {
            let z = z.trim();
            let sinal = if z.starts_with('-') { -1 } else { 1 };
            let h: i64 = z.get(1..3)?.parse().ok()?;
            let m: i64 = z.get(3..5)?.parse().ok()?;
            Some(sinal * (h * 3600 + m * 60))
        })
        .unwrap_or(0)
}

/// Quem vence agora: o alvo, a curva (com ou sem a antecipação) ou ninguém.
fn quem_vence(r: &Retrato, cfg: &Config) -> &'static str {
    if !r.freio_acionado && r.pl1_alvo_w >= cfg.pl1_normal_w - 0.05 {
        "normal"
    } else if r.decisao.pelo_alvo {
        "alvo"
    } else if r.decisao.antecipado {
        "antecipado"
    } else {
        "curva"
    }
}

/// Tudo o que o painel desenha.
pub fn estado_json(
    r: &Retrato,
    historia: &Historia,
    cfg: &Config,
    em_vigor: &toml::Table,
    unix_s: f64,
    fuso_s: i64,
    pode_ajustar: bool,
) -> String {
    let mut fichas: Vec<&crate::metricas::FichaLida> = r.fichas.iter().collect();
    fichas.sort_by(|a, b| b.nota_c_s.total_cmp(&a.nota_c_s));
    let mut sensores = r.sensores_do_mac.clone();
    sensores.sort_by(|a, b| b.1.total_cmp(&a.1));
    sensores.truncate(6);
    let modo = match cfg.maos.modo {
        crate::config::Modo::Observar => "observar",
        crate::config::Modo::Agir => "agir",
        crate::config::Modo::Desligado => "desligado",
    };
    json!({
        "versao": env!("CARGO_PKG_VERSION"),
        "maquina": std::fs::read_to_string("/etc/hostname").map_or_else(|_| String::new(), |h| h.trim().to_string()),
        "agora_unix": unix_s,
        "agora": {
            "temperatura": r.temperatura_c,
            "prevista": r.temperatura_prevista_c,
            "tendencia": r.tendencia_c_s,
            "pl1": r.pl1_w,
            "pl1_alvo": r.pl1_alvo_w,
            "pl1_normal": cfg.pl1_normal_w,
            "potencia": r.potencia_w,
            "frequencia_mhz": r.frequencia_mhz,
            "tcc_corte": r.corte_c(),
            "psi_memoria": r.pressao.as_ref().map(|p| p.memoria_some_pct),
            "freio_acionado": r.freio_acionado,
            "freio_intensidade": r.freio_intensidade,
            "segurados": r.segurados,
        },
        "ventoinhas": r.ventoinhas.iter().map(|v| json!({"numero": v.numero, "rpm": v.rpm, "maximo": v.maximo})).collect::<Vec<_>>(),
        "serie": historia.amostras.iter().map(|a| json!({"t": a.unix_s, "temperatura": a.temperatura_c, "potencia": a.potencia_w, "pl1": a.pl1_w})).collect::<Vec<_>>(),
        "freio_por_hora": historia.minutos_por_hora(unix_s).into_iter().map(|(h, m)| json!({"hora": hora_local(h, fuso_s), "minutos": m})).collect::<Vec<_>>(),
        "processos": r.processos.iter().map(|p| {
            let ficha = r.fichas.iter().find(|f| f.nome == p.nome);
            json!({"nome": p.nome, "fatia": p.fatia, "nota": ficha.map(|f| f.nota_c_s), "suspeito": ficha.is_some_and(|f| f.suspeito)})
        }).collect::<Vec<_>>(),
        "historico": fichas.iter().map(|f| json!({
            "nome": f.nome, "nota": f.nota_c_s, "pico": f.maior_tendencia_c_s,
            "observacoes": f.observacoes, "visto_em": f.visto_em, "suspeito": f.suspeito,
        })).collect::<Vec<_>>(),
        "suspeito_c_s": cfg.processos.suspeito_c_s,
        "sensores": sensores.iter().map(|(n, c)| json!({"nome": n, "celsius": c})).collect::<Vec<_>>(),
        "nucleos": r.nucleos_c.iter().map(|(n, c)| json!({
            "nucleo": n,
            "celsius": c,
            "estrangulamentos": r.nucleos_estrangulamento.iter().find(|(i, _)| i == n).map(|(_, v)| v),
        })).collect::<Vec<_>>(),
        "motivos": r.motivos.as_ref().map_or_else(Vec::new, |m| m.lista.iter().map(|mo| json!({
            "nome": mo.nome,
            "agora": mo.agora,
            "desde_o_boot": mo.desde_o_boot,
            "leituras": r.energia.motivo_leituras.iter().find(|(n, _)| *n == mo.nome).map_or(0, |(_, v)| *v),
        })).collect::<Vec<_>>()),
        "prochot": {"eventos": r.energia.prochot_eventos, "segundos": r.energia.prochot_s},
        "bateria": r.bateria.as_ref().map(|b| json!({
            "carregador": b.carregador_ligado,
            "estado": b.estado,
            "amperes": b.amperes,
            "volts": b.volts,
            "carga_pct": b.carga_pct,
            "descarga_s": r.energia.descarga_s,
            "descarga_wh": r.energia.descarga_wh,
            "descarga_episodios": r.energia.descarga_episodios,
        })),
        "feitos": avisos::diario(12).iter().map(|a| json!({"em_ms": a.em_ms as u64, "texto": a.texto, "tags": a.tags})).collect::<Vec<_>>(),
        "decisao": {
            "vence": quem_vence(r, cfg),
            "curva_w": r.decisao.pela_curva_w,
            "alvo_w": r.decisao.do_alvo_w,
            "prevista_processos": r.decisao.prevista_processos_c,
            "freio_comeca_c": cfg.freio_comeca_c,
            "freio_acima_c": cfg.freio_acima_c,
            "suspeitos_ativos": r.suspeitos.iter().map(|(n, _)| n).collect::<Vec<_>>(),
        },
        "ajustes": {
            "alvo_c": cfg.alvo_c,
            "maos_modo": modo,
            "pl1_normal_w": cfg.pl1_normal_w,
            "em_vigor": ajustes::achatar(em_vigor).into_iter().map(|(k, v)| json!({"chave": k, "valor": v})).collect::<Vec<_>>(),
            "pode_ajustar": pode_ajustar,
        },
    })
    .to_string()
}

/// Um pedido HTTP já lido.
#[derive(Debug, Default)]
pub struct Pedido {
    pub metodo: String,
    pub caminho: String,
    /// Nomes em minúsculas.
    pub cabecalhos: HashMap<String, String>,
    pub corpo: Vec<u8>,
}

impl Pedido {
    pub fn cabecalho(&self, nome: &str) -> Option<&str> {
        self.cabecalhos.get(nome).map(String::as_str)
    }
}

/// Lê o pedido inteiro: linha, cabeçalhos e o corpo do `Content-Length` (até 64 KiB).
pub fn ler_pedido(leitor: &mut impl std::io::Read) -> std::io::Result<Pedido> {
    let mut bruto = Vec::with_capacity(1024);
    let mut pedaco = [0u8; 1024];
    let fim = loop {
        if let Some(i) = bruto.windows(4).position(|j| j == b"\r\n\r\n") {
            break i;
        }
        if bruto.len() > 16 * 1024 {
            return Err(std::io::Error::other("cabeçalho grande demais"));
        }
        let n = leitor.read(&mut pedaco)?;
        if n == 0 {
            break bruto.len();
        }
        bruto.extend_from_slice(&pedaco[..n]);
    };
    let cabeca = String::from_utf8_lossy(&bruto[..fim]).to_string();
    let mut linhas = cabeca.split("\r\n");
    let primeira = linhas.next().unwrap_or_default();
    let mut partes = primeira.split_whitespace();
    let mut p = Pedido {
        metodo: partes.next().unwrap_or("GET").to_string(),
        caminho: partes
            .next()
            .unwrap_or("/")
            .split('?')
            .next()
            .unwrap_or("/")
            .to_string(),
        ..Pedido::default()
    };
    for l in linhas {
        if let Some((k, v)) = l.split_once(':') {
            p.cabecalhos
                .insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
        }
    }
    let tamanho: usize = p
        .cabecalho("content-length")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
        .min(64 * 1024);
    let mut corpo = bruto.get(fim + 4..).unwrap_or_default().to_vec();
    while corpo.len() < tamanho {
        let n = leitor.read(&mut pedaco)?;
        if n == 0 {
            break;
        }
        corpo.extend_from_slice(&pedaco[..n]);
    }
    corpo.truncate(tamanho);
    p.corpo = corpo;
    Ok(p)
}

/// Quem está pedindo pode mudar ajustes? `Ok(login)` ou a razão da recusa.
pub fn autorizar(p: &Pedido, cfg: &Config) -> Result<String, String> {
    let login = p
        .cabecalho("tailscale-user-login")
        .filter(|l| !l.is_empty())
        .ok_or("o painel só ajusta pela tailnet (tailscale serve)")?;
    if !cfg.painel.quem_pode_ajustar.iter().any(|q| q == login) {
        return Err(format!("{login} não está em [painel] quem_pode_ajustar"));
    }
    Ok(login.to_string())
}

/// O `Origin`, quando o navegador manda, tem de ser o próprio host.
fn mesma_origem(p: &Pedido) -> bool {
    match (p.cabecalho("origin"), p.cabecalho("host")) {
        (None, _) => true,
        (Some(origem), Some(host)) => {
            let sem_esquema = origem.split("://").nth(1).unwrap_or(origem);
            sem_esquema == host || p.cabecalho("x-forwarded-host") == Some(sem_esquema)
        }
        (Some(_), None) => false,
    }
}

/// Uma resposta: status, tipo e corpo.
pub type Resposta = (&'static str, &'static str, Vec<u8>);

fn erro(status: &'static str, motivo: impl Into<String>) -> Resposta {
    (
        status,
        "application/json",
        json!({"erro": motivo.into()}).to_string().into_bytes(),
    )
}

/// `POST /api/ajustes`: `{"ajustes": {"alvo_c": 65, "maos.modo": "agir"}}`,
/// `{"ajustes": {"alvo_c": null}}` (volta ao toml) ou `{"padrao": true}`.
pub fn ajustar(p: &Pedido, cfg: &Config, caminho_config: &Path) -> Resposta {
    if !p
        .cabecalho("content-type")
        .is_some_and(|t| t.starts_with("application/json"))
    {
        return erro("415 Unsupported Media Type", "o corpo tem de ser JSON");
    }
    if !mesma_origem(p) {
        return erro("403 Forbidden", "pedido de outra página");
    }
    let login = match autorizar(p, cfg) {
        Ok(l) => l,
        Err(e) => return erro("403 Forbidden", e),
    };
    let Ok(corpo) = serde_json::from_slice::<Value>(&p.corpo) else {
        return erro("400 Bad Request", "JSON inválido");
    };
    let estado = Path::new(&cfg.estado);
    let atual = match ajustes::tabela(&ajustes::texto(estado)) {
        Ok(t) => t,
        Err(e) => return erro("500 Internal Server Error", e),
    };
    let novo = if corpo.get("padrao") == Some(&Value::Bool(true)) {
        toml::Table::new()
    } else {
        let Some(pedidos) = corpo.get("ajustes").and_then(Value::as_object) else {
            return erro("400 Bad Request", "falta \"ajustes\"");
        };
        let mut t = atual.clone();
        for (chave, valor) in pedidos {
            let raiz = chave.split('.').next().unwrap_or(chave);
            if ajustes::SO_NA_SUBIDA.contains(&raiz) {
                return erro(
                    "400 Bad Request",
                    format!("{chave} só muda reiniciando o serviço"),
                );
            }
            let valor = match valor {
                Value::Null => None,
                v => match toml::Value::try_from(v) {
                    Ok(tv) => Some(tv),
                    Err(e) => return erro("400 Bad Request", format!("{chave}: {e}")),
                },
            };
            ajustes::aplicar(&mut t, chave, valor);
        }
        t
    };
    if let Err(e) = ajustes::config_com(caminho_config, &novo) {
        return erro("400 Bad Request", e);
    }
    if let Err(e) = ajustes::gravar(estado, &novo) {
        return erro("500 Internal Server Error", e);
    }
    let mudou = ajustes::diferencas(&atual, &novo);
    println!(
        "🎛️ painel ({login}): {}",
        if mudou.is_empty() {
            "nada mudou".into()
        } else {
            mudou.join(", ")
        }
    );
    (
        "200 OK",
        "application/json",
        json!({"mudou": mudou}).to_string().into_bytes(),
    )
}

#[cfg(test)]
mod testes {
    use super::*;

    fn pedido(texto: &str) -> Pedido {
        ler_pedido(&mut texto.as_bytes()).unwrap()
    }

    fn cfg_em(dir: &Path) -> Config {
        Config {
            estado: dir.to_string_lossy().into(),
            painel: crate::config::ConfigPainel {
                quem_pode_ajustar: vec!["dono@exemplo.dev".into()],
            },
            ..Config::default()
        }
    }

    fn post(corpo: &str, extras: &str) -> Pedido {
        pedido(&format!(
            "POST /api/ajustes HTTP/1.1\r\nHost: fedora.tail:8443\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extras}\r\n{corpo}",
            corpo.len()
        ))
    }

    #[test]
    fn le_o_pedido_com_corpo() {
        let p = post(
            "{\"padrao\":true}",
            "Tailscale-User-Login: dono@exemplo.dev\r\n",
        );
        assert_eq!(p.metodo, "POST");
        assert_eq!(p.caminho, "/api/ajustes");
        assert_eq!(
            p.cabecalho("tailscale-user-login"),
            Some("dono@exemplo.dev")
        );
        assert_eq!(p.corpo, b"{\"padrao\":true}");
        let g = pedido("GET /api/estado?x=1 HTTP/1.1\r\nHost: a\r\n\r\n");
        assert_eq!(g.caminho, "/api/estado");
        assert!(g.corpo.is_empty());
    }

    #[test]
    fn ajusta_quem_pode_e_grava() {
        let d = tempfile::tempdir().unwrap();
        let cfg = cfg_em(d.path());
        let base = d.path().join("termostato.toml");
        let p = post(
            "{\"ajustes\":{\"alvo_c\":65,\"maos.modo\":\"agir\"}}",
            "Tailscale-User-Login: dono@exemplo.dev\r\nOrigin: https://fedora.tail:8443\r\n",
        );
        let (status, _, corpo) = ajustar(&p, &cfg, &base);
        assert_eq!(status, "200 OK", "{}", String::from_utf8_lossy(&corpo));
        let t = ajustes::tabela(&ajustes::texto(d.path())).unwrap();
        let c = ajustes::config_com(&base, &t).unwrap();
        assert!((c.alvo_c - 65.0).abs() < 1e-9);
        assert_eq!(c.maos.modo, crate::config::Modo::Agir);

        // null tira o ajuste; padrao tira todos.
        let p = post(
            "{\"ajustes\":{\"alvo_c\":null}}",
            "Tailscale-User-Login: dono@exemplo.dev\r\n",
        );
        assert_eq!(ajustar(&p, &cfg, &base).0, "200 OK");
        let t = ajustes::tabela(&ajustes::texto(d.path())).unwrap();
        assert!(!t.contains_key("alvo_c"));
        let p = post(
            "{\"padrao\":true}",
            "Tailscale-User-Login: dono@exemplo.dev\r\n",
        );
        assert_eq!(ajustar(&p, &cfg, &base).0, "200 OK");
        assert!(!ajustes::arquivo(d.path()).exists());
    }

    #[test]
    fn recusa_sem_login_de_fora_e_de_outra_pagina() {
        let d = tempfile::tempdir().unwrap();
        let cfg = cfg_em(d.path());
        let base = d.path().join("termostato.toml");
        let corpo = "{\"ajustes\":{\"alvo_c\":65}}";
        for extras in [
            "",
            "Tailscale-User-Login: outro@exemplo.dev\r\n",
            "Tailscale-User-Login: dono@exemplo.dev\r\nOrigin: https://site-mau.example\r\n",
        ] {
            assert_eq!(
                ajustar(&post(corpo, extras), &cfg, &base).0,
                "403 Forbidden",
                "{extras}"
            );
        }
        let sem_json = pedido(&format!(
            "POST /api/ajustes HTTP/1.1\r\nHost: a\r\nContent-Type: text/plain\r\nTailscale-User-Login: dono@exemplo.dev\r\nContent-Length: {}\r\n\r\n{corpo}",
            corpo.len()
        ));
        assert_eq!(
            ajustar(&sem_json, &cfg, &base).0,
            "415 Unsupported Media Type"
        );
        assert!(!ajustes::arquivo(d.path()).exists(), "nada gravado");
    }

    #[test]
    fn recusa_o_invalido_e_o_da_subida() {
        let d = tempfile::tempdir().unwrap();
        let cfg = cfg_em(d.path());
        let base = d.path().join("termostato.toml");
        let login = "Tailscale-User-Login: dono@exemplo.dev\r\n";
        for corpo in [
            "{\"ajustes\":{\"alvo_c\":30}}",
            "{\"ajustes\":{\"pl1_normla_w\":30}}",
            "{\"ajustes\":{\"painel.quem_pode_ajustar\":[\"eu\"]}}",
            "{\"ajustes\":{\"escuta\":\"0.0.0.0:1\"}}",
            "não é json",
        ] {
            assert_eq!(
                ajustar(&post(corpo, login), &cfg, &base).0,
                "400 Bad Request",
                "{corpo}"
            );
        }
        assert!(!ajustes::arquivo(d.path()).exists(), "nada gravado");
    }

    #[test]
    fn a_historia_guarda_a_cada_10_s_e_conta_o_freio() {
        let mut h = Historia::default();
        let mut r = Retrato {
            temperatura_c: Some(60.0),
            potencia_w: Some(15.0),
            pl1_w: Some(38.0),
            ..Retrato::default()
        };
        for i in 0..100 {
            r.freio_acionado = i >= 70;
            h.registrar(36_000.0 + f64::from(i) * 2.0, 2.0, &r);
        }
        assert_eq!(h.amostras.len(), 20, "200 s a cada 10 s");
        let minutos = h.minutos_por_hora(36_000.0 + 200.0);
        assert_eq!(minutos.len(), 12);
        assert!(
            (minutos[11].1 - 1.0).abs() < 1e-9,
            "30 voltas de 2 s = 1 min"
        );
    }

    #[test]
    fn o_estado_tem_o_que_o_painel_le() {
        let r = Retrato {
            temperatura_c: Some(64.0),
            freio_acionado: true,
            pl1_alvo_w: 18.2,
            decisao: crate::metricas::DecisaoLida {
                pela_curva_w: 38.0,
                do_alvo_w: Some(18.2),
                pelo_alvo: true,
                ..Default::default()
            },
            nucleos_c: vec![(0, 97.0), (1, 92.0)],
            nucleos_estrangulamento: vec![(0, 765)],
            motivos: Some(crate::vigia::Motivos::de(1 << 10, 1 << 16)),
            bateria: Some(crate::vigia::Bateria {
                carregador_ligado: Some(true),
                estado: "Discharging".into(),
                amperes: Some(-1.2),
                carga_pct: Some(90.0),
                volts: Some(12.6),
            }),
            energia: crate::energia::EnergiaLida {
                motivo_leituras: vec![("pl1", 20)],
                prochot_eventos: 1,
                prochot_s: 2.0,
                descarga_s: 21.0,
                descarga_wh: 0.05,
                descarga_episodios: 2,
            },
            ..Retrato::default()
        };
        let cfg = Config {
            alvo_c: 65.0,
            ..Config::default()
        };
        let v: Value = serde_json::from_str(&estado_json(
            &r,
            &Historia::default(),
            &cfg,
            &toml::Table::new(),
            0.0,
            -10_800,
            false,
        ))
        .unwrap();
        assert_eq!(v["decisao"]["vence"], "alvo");
        assert_eq!(v["agora"]["temperatura"], 64.0);
        assert_eq!(v["ajustes"]["alvo_c"], 65.0);
        assert_eq!(v["ajustes"]["pode_ajustar"], false);
        assert_eq!(v["freio_por_hora"].as_array().unwrap().len(), 12);
        assert_eq!(hora_local(10, -10_800), 7);
        assert_eq!(v["nucleos"][0]["celsius"], 97.0);
        assert_eq!(v["nucleos"][0]["estrangulamentos"], 765);
        assert!(v["nucleos"][1]["estrangulamentos"].is_null());
        let pl1 = v["motivos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["nome"] == "pl1")
            .unwrap();
        assert_eq!(pl1["agora"], true);
        assert_eq!(pl1["leituras"], 20);
        let prochot = v["motivos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["nome"] == "prochot")
            .unwrap();
        assert_eq!(prochot["desde_o_boot"], true);
        assert_eq!(v["prochot"]["eventos"], 1);
        assert_eq!(v["bateria"]["amperes"], -1.2);
        assert_eq!(v["bateria"]["descarga_episodios"], 2);
    }
}

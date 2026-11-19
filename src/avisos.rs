//! Os avisos: cada coisa que o termostato faz vira uma anotação no Grafana.
//!
//! O dono: *"termostato deve notificar o grafana o que ele está fazendo, pois
//! pode ser nocivo para o ambiente"*. Cada freio, cada processo segurado, cada
//! limite devolvido (e, no modo observar, cada coisa que **faria**) vira uma
//! marca vertical no painel, com o texto do journal e tags para filtrar.
//!
//! As anotações vão por `POST /api/annotations`, com o token de uma conta de
//! serviço guardado num arquivo (modo 600, fora do repositório). O envio fica
//! numa thread própria com uma fila: um Grafana fora do ar nunca atrasa o
//! freio, só gera um ⚠️ no journal. Sem token, os avisos ficam só no journal e
//! nas métricas (`termostato_acoes_total`).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::config::ConfigGrafana;

#[derive(Debug, Clone, PartialEq)]
pub struct Aviso {
    pub texto: String,
    pub tags: Vec<String>,
    /// Milissegundos desde 1970.
    pub em_ms: u128,
}

impl Aviso {
    pub fn novo(texto: impl Into<String>, tags: &[&str]) -> Self {
        let mut t: Vec<String> = vec!["termostato".into()];
        t.extend(tags.iter().map(|s| (*s).to_string()));
        Self {
            texto: texto.into(),
            tags: t,
            em_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis()),
        }
    }

    pub fn json(&self) -> String {
        serde_json::json!({ "time": self.em_ms, "text": self.texto, "tags": self.tags }).to_string()
    }
}

/// Quem manda os avisos: o laço só empurra na fila.
#[derive(Clone)]
pub struct Avisador {
    fila: Option<Sender<Aviso>>,
}

impl Avisador {
    /// Sem URL ou sem token legível, só journal e métricas.
    pub fn novo(cfg: &ConfigGrafana) -> (Self, Option<String>) {
        if cfg.url.is_empty() {
            return (
                Self::mudo(),
                Some("sem grafana.url: avisos só no journal".into()),
            );
        }
        let token = match std::fs::read_to_string(&cfg.token_arquivo) {
            Ok(t) if !t.trim().is_empty() => t.trim().to_string(),
            _ => {
                return (
                    Self { fila: None },
                    Some(format!(
                        "sem token em {}: avisos só no journal e nas métricas",
                        cfg.token_arquivo
                    )),
                );
            }
        };
        let (tx, rx) = channel();
        let url = cfg.url.clone();
        std::thread::spawn(move || enviar_sempre(&rx, &url, &token));
        (Self { fila: Some(tx) }, None)
    }

    /// Um avisador que não manda nada (testes e sem Grafana).
    pub fn mudo() -> Self {
        Self { fila: None }
    }

    pub fn avisar(&self, aviso: Aviso) {
        if let Some(f) = &self.fila {
            let _ = f.send(aviso);
        }
    }
}

/// Esperas entre as tentativas de um aviso. O termostato sobe cedo no boot,
/// antes da rede: os primeiros avisos esperam ela subir em vez de se perder
/// (visto no primeiro boot do Fedora 45, em 19/11/2026: "Network is
/// unreachable").
const ESPERAS_S: [u64; 4] = [5, 15, 30, 60];

fn enviar_sempre(fila: &Receiver<Aviso>, url: &str, token: &str) {
    for aviso in fila {
        let mut tentativa = 0;
        loop {
            match enviar(url, token, &aviso) {
                Ok(()) => break,
                Err(e) if tentativa < ESPERAS_S.len() => {
                    std::thread::sleep(std::time::Duration::from_secs(ESPERAS_S[tentativa]));
                    tentativa += 1;
                    let _ = e;
                }
                Err(e) => {
                    eprintln!("⚠️ Grafana: {e} (o aviso fica só no journal)");
                    break;
                }
            }
        }
    }
}

/// `http://host:porta` → `host:porta`. Só HTTP: o Grafana está na tailnet.
pub fn endereco(url: &str) -> Result<String, String> {
    let resto = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("{url}: só http:// é aceito"))?;
    let host = resto.split('/').next().unwrap_or(resto);
    Ok(if host.contains(':') {
        host.to_string()
    } else {
        format!("{host}:80")
    })
}

pub fn enviar(url: &str, token: &str, aviso: &Aviso) -> Result<(), String> {
    let endereco = endereco(url)?;
    let alvo = std::net::ToSocketAddrs::to_socket_addrs(&endereco)
        .map_err(|e| format!("{endereco}: {e}"))?
        .next()
        .ok_or_else(|| format!("{endereco}: sem endereço"))?;
    let mut s = TcpStream::connect_timeout(&alvo, Duration::from_secs(2))
        .map_err(|e| format!("{endereco}: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(3))).ok();
    s.set_write_timeout(Some(Duration::from_secs(3))).ok();
    let corpo = aviso.json();
    write!(
        s,
        "POST /api/annotations HTTP/1.0\r\nHost: {endereco}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{corpo}",
        corpo.len()
    )
    .map_err(|e| e.to_string())?;
    let mut resposta = String::new();
    s.read_to_string(&mut resposta).map_err(|e| e.to_string())?;
    let status = resposta.split_whitespace().nth(1).unwrap_or("?");
    if status == "200" {
        Ok(())
    } else {
        Err(format!("respondeu {status}"))
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn manda_a_anotacao_com_token_texto_e_tags() {
        let ouvinte = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", ouvinte.local_addr().unwrap());
        let t = std::thread::spawn(move || {
            let (mut c, _) = ouvinte.accept().unwrap();
            let mut buf = vec![0u8; 8192];
            let mut lido = 0;
            // Lê até o corpo inteiro chegar.
            loop {
                let n = c.read(&mut buf[lido..]).unwrap();
                lido += n;
                let s = String::from_utf8_lossy(&buf[..lido]).to_string();
                if n == 0 || s.contains("\"tags\"") {
                    break;
                }
            }
            c.write_all(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\n{}")
                .unwrap();
            String::from_utf8_lossy(&buf[..lido]).to_string()
        });
        let a = Aviso::novo("🤚 segurando rustc", &["segurar", "observar"]);
        enviar(&url, "segredo", &a).unwrap();
        let pedido = t.join().unwrap();
        assert!(pedido.starts_with("POST /api/annotations"));
        assert!(pedido.contains("Authorization: Bearer segredo"));
        assert!(pedido.contains(r#""text":"🤚 segurando rustc""#));
        assert!(pedido.contains(r#""tags":["termostato","segurar","observar"]"#));
    }

    #[test]
    fn grafana_fora_do_ar_e_erro_rapido_e_nao_trava() {
        let ouvinte = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", ouvinte.local_addr().unwrap());
        drop(ouvinte);
        let inicio = std::time::Instant::now();
        assert!(enviar(&url, "x", &Aviso::novo("t", &[])).is_err());
        assert!(inicio.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn sem_token_os_avisos_ficam_no_journal() {
        let cfg = ConfigGrafana {
            url: "http://127.0.0.1:1".into(),
            token_arquivo: "/nao/existe".into(),
        };
        let (a, motivo) = Avisador::novo(&cfg);
        assert!(motivo.unwrap().contains("sem token"));
        a.avisar(Aviso::novo("nada acontece", &[]));
    }

    #[test]
    fn endereco_da_url() {
        assert_eq!(
            endereco("http://100.64.0.10:3000").unwrap(),
            "100.64.0.10:3000"
        );
        assert_eq!(endereco("http://grafana").unwrap(), "grafana:80");
        assert!(endereco("https://x").is_err());
    }
}

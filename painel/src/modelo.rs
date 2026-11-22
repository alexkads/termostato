//! O que `GET /api/estado` do termostato manda. Tudo com padrão: um campo que
//! falta (um termostato mais velho, um sensor que não leu) vira vazio, nunca erro.

use serde::Deserialize;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Estado {
    pub versao: String,
    pub maquina: String,
    pub agora_unix: f64,
    pub agora: Agora,
    pub ventoinhas: Vec<Ventoinha>,
    pub serie: Vec<Amostra>,
    pub freio_por_hora: Vec<HoraDeFreio>,
    pub processos: Vec<Processo>,
    pub historico: Vec<Ficha>,
    pub suspeito_c_s: f64,
    pub sensores: Vec<Sensor>,
    pub feitos: Vec<Feito>,
    pub decisao: Decisao,
    pub ajustes: Ajustes,
    pub nucleos: Vec<Nucleo>,
    pub motivos: Vec<MotivoDoFreio>,
    pub prochot: Prochot,
    pub bateria: Option<Bateria>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Nucleo {
    pub nucleo: u32,
    pub celsius: f64,
    pub estrangulamentos: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct MotivoDoFreio {
    pub nome: String,
    pub agora: bool,
    pub desde_o_boot: bool,
    pub leituras: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Prochot {
    pub eventos: u64,
    pub segundos: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Bateria {
    pub carregador: Option<bool>,
    pub estado: String,
    pub amperes: Option<f64>,
    pub volts: Option<f64>,
    pub carga_pct: Option<f64>,
    pub descarga_s: f64,
    pub descarga_wh: f64,
    pub descarga_episodios: u64,
}

/// O nome de um motivo do freio do chip (`MSR 0x690`) em português.
pub fn nome_do_motivo(codigo: &str) -> String {
    match codigo {
        "prochot" => "PROCHOT (freio de fora)",
        "termico" => "Calor no chip",
        "pl1" => "Limite PL1",
        "pl2" => "Limite PL2",
        "corrente" => "Corrente elétrica",
        "vr" => "Regulador de tensão quente",
        "turbo_maximo" => "Teto do turbo",
        "utilizacao_autonoma" => "Pouco uso (o chip baixou sozinho)",
        "atenuacao_turbo" => "Trocas do turbo seguradas",
        outro => outro,
    }
    .to_string()
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Agora {
    pub temperatura: Option<f64>,
    pub prevista: Option<f64>,
    pub tendencia: Option<f64>,
    pub pl1: Option<f64>,
    pub pl1_alvo: f64,
    pub pl1_normal: f64,
    pub potencia: Option<f64>,
    pub frequencia_mhz: Option<f64>,
    pub tcc_corte: Option<f64>,
    pub psi_memoria: Option<f64>,
    pub freio_acionado: bool,
    pub freio_intensidade: f64,
    pub segurados: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Ventoinha {
    pub numero: String,
    pub rpm: Option<f64>,
    pub maximo: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Amostra {
    pub t: f64,
    pub temperatura: f64,
    pub potencia: f64,
    pub pl1: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct HoraDeFreio {
    pub hora: u32,
    pub minutos: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Processo {
    pub nome: String,
    pub fatia: f64,
    pub nota: Option<f64>,
    pub suspeito: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Ficha {
    pub nome: String,
    pub nota: f64,
    pub pico: f64,
    pub observacoes: u32,
    pub visto_em: f64,
    pub suspeito: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Sensor {
    pub nome: String,
    pub celsius: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Feito {
    pub em_ms: f64,
    pub texto: String,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Decisao {
    /// `normal`, `curva`, `alvo` ou `antecipado`.
    pub vence: String,
    pub curva_w: f64,
    pub alvo_w: Option<f64>,
    pub prevista_processos: Option<f64>,
    pub freio_comeca_c: f64,
    pub freio_acima_c: f64,
    pub suspeitos_ativos: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Ajustes {
    pub alvo_c: f64,
    pub maos_modo: String,
    pub pl1_normal_w: f64,
    pub em_vigor: Vec<AjusteEmVigor>,
    pub pode_ajustar: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct AjusteEmVigor {
    pub chave: String,
    pub valor: String,
}

/// `64,0` em vez de `64.0`.
pub fn num(v: f64, casas: usize) -> String {
    format!("{v:.casas$}").replace('.', ",")
}

/// `6156` → `6.156`.
pub fn milhar(v: f64) -> String {
    let s = format!("{:.0}", v.round());
    let (sinal, digitos) = s.strip_prefix('-').map_or(("", s.as_str()), |d| ("-", d));
    let mut saida = String::new();
    for (i, c) in digitos.chars().enumerate() {
        if i > 0 && (digitos.len() - i) % 3 == 0 {
            saida.push('.');
        }
        saida.push(c);
    }
    format!("{sinal}{saida}")
}

/// `há 25 min`, `há 2 h 5 min`, `agora`.
pub fn ha(agora_s: f64, antes_s: f64) -> String {
    let min = ((agora_s - antes_s) / 60.0).round().max(0.0) as u64;
    match min {
        0 => "agora".into(),
        1..=59 => format!("há {min} min"),
        _ if min < 24 * 60 => format!("há {} h {} min", min / 60, min % 60),
        _ => format!("há {} dias", min / (24 * 60)),
    }
}

/// O nome de um sensor do SMC da Apple em português, quando ele é conhecido.
pub fn nome_do_sensor(codigo: &str) -> String {
    let nome = match codigo {
        "TC0P" | "TC0D" | "TC0E" | "TC0F" | "TCXC" => "CPU",
        "TG0P" | "TG0D" | "TGDD" => "GPU",
        "TB0T" | "TB1T" | "TB2T" => "Bateria",
        "TM0P" | "TM0S" => "Memória",
        "Tm0P" | "Tp0P" => "Placa",
        "TA0P" | "Ta0P" | "TaLC" | "TaRC" => "Ar interno",
        "TH0P" | "TH0a" | "TH0b" => "Disco",
        "TW0P" => "Wi-Fi",
        "Ts0P" | "Ts0S" => "Apoio das mãos",
        _ => return codigo.to_string(),
    };
    format!("{nome} ({codigo})")
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn numeros_em_portugues() {
        assert_eq!(num(64.04, 1), "64,0");
        assert_eq!(milhar(6156.0), "6.156");
        assert_eq!(milhar(999.0), "999");
        assert_eq!(milhar(1_234_567.0), "1.234.567");
        assert_eq!(ha(1000.0, 1000.0), "agora");
        assert_eq!(ha(3600.0, 0.0), "há 1 h 0 min");
        assert_eq!(ha(1500.0, 0.0), "há 25 min");
        assert_eq!(nome_do_sensor("TB0T"), "Bateria (TB0T)");
        assert_eq!(nome_do_sensor("XYZ1"), "XYZ1");
        assert_eq!(nome_do_motivo("prochot"), "PROCHOT (freio de fora)");
        assert_eq!(nome_do_motivo("novo"), "novo");
    }

    #[test]
    fn le_o_json_do_termostato_mesmo_incompleto() {
        let e: Estado = serde_json::from_str(
            r#"{"versao":"0.8.0","agora":{"temperatura":64.0},"decisao":{"vence":"alvo"}}"#,
        )
        .unwrap();
        assert_eq!(e.versao, "0.8.0");
        assert_eq!(e.agora.temperatura, Some(64.0));
        assert_eq!(e.decisao.vence, "alvo");
        assert!(e.serie.is_empty());
        assert!(e.nucleos.is_empty() && e.motivos.is_empty() && e.bateria.is_none());
        let e: Estado = serde_json::from_str(
            r#"{"nucleos":[{"nucleo":2,"celsius":99.0,"estrangulamentos":9719}],
                "motivos":[{"nome":"prochot","agora":false,"desde_o_boot":true,"leituras":1}],
                "prochot":{"eventos":1,"segundos":2.0},
                "bateria":{"carregador":true,"amperes":-1.21,"carga_pct":90.0}}"#,
        )
        .unwrap();
        assert_eq!(e.nucleos[0].estrangulamentos, Some(9719));
        assert!(e.motivos[0].desde_o_boot);
        assert_eq!(e.prochot.eventos, 1);
        assert_eq!(e.bateria.unwrap().amperes, Some(-1.21));
    }
}

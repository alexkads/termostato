//! O freio de calor: a curva, com previsão do pico. Pura, sem hardware.
//!
//! A CPU nunca fica solta: sem calor, vale o `pl1_normal_w`. O freio age antes
//! de o pico chegar, em três passos a cada leitura:
//!
//! 1. **Previsão.** Uma reta ajustada às últimas `janela_leituras` temperaturas
//!    (mínimos quadrados) dá a tendência em °C/s. Se ela sobe, a temperatura
//!    prevista é a de agora mais a tendência vezes `previsao_s`. Se desce, vale
//!    a de agora: esfriar não é motivo para soltar mais cedo.
//!    Uma **terceira fonte** vem dos processos (`historico.rs`): quando um nome
//!    do histórico negativo começa a gastar CPU, a temperatura prevista por ele
//!    entra na conta, e a curva freia antes de o sensor reagir (o freio
//!    **antecipado**).
//! 2. **Curva.** O PL1 desejado sai de uma reta entre dois pontos: até
//!    `freio_comeca_c` é o `pl1_normal_w`; em `freio_acima_c` ou mais, o
//!    `pl1_freio_w`. A curva é aplicada à maior entre a temperatura de agora e
//!    a prevista.
//! 3. **Freia rápido, solta devagar.** Se o desejado é menor que o PL1 atual, ele
//!    vale na hora. Se é maior, o PL1 sobe no máximo `subida_w_por_s` por
//!    segundo. É isso que substitui a histerese: sem calor novo, o freio vai
//!    soltando aos poucos, sem o efeito sanfona.

use std::collections::VecDeque;

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mudanca {
    /// O PL1 saiu do limite normal.
    Acionou,
    /// O PL1 voltou ao limite normal.
    Desligou,
}

/// O que uma leitura decidiu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Decisao {
    /// Tendência da temperatura, em °C/s (negativa esfriando).
    pub tendencia_c_s: f64,
    /// A temperatura prevista pela tendência, para daqui a `previsao_s`.
    pub prevista_c: f64,
    /// A prevista pelos processos do histórico negativo, quando houver.
    pub prevista_processos_c: Option<f64>,
    /// A previsão pelos processos foi a maior das três: o freio veio antes.
    pub antecipado: bool,
    /// O PL1 que vale agora, em watts.
    pub pl1_w: f64,
    /// 0 = limite normal; 1 = freio no máximo (`pl1_freio_w`).
    pub intensidade: f64,
    pub mudanca: Option<Mudanca>,
}

#[derive(Debug, Default, Clone)]
pub struct Freio {
    /// (segundos, °C) das últimas leituras.
    historico: VecDeque<(f64, f64)>,
    pl1_w: Option<f64>,
    acionado: bool,
}

/// O PL1 desejado para uma temperatura, pela curva.
pub fn curva(temperatura: f64, cfg: &Config) -> f64 {
    if temperatura <= cfg.freio_comeca_c {
        return cfg.pl1_normal_w;
    }
    if temperatura >= cfg.freio_acima_c {
        return cfg.pl1_freio_w;
    }
    let fracao = (temperatura - cfg.freio_comeca_c) / (cfg.freio_acima_c - cfg.freio_comeca_c);
    cfg.pl1_normal_w - fracao * (cfg.pl1_normal_w - cfg.pl1_freio_w)
}

/// A inclinação (°C/s) da reta de mínimos quadrados pelos pontos.
pub fn tendencia(pontos: &VecDeque<(f64, f64)>) -> f64 {
    let n = pontos.len() as f64;
    if n < 2.0 {
        return 0.0;
    }
    let media_t = pontos.iter().map(|p| p.0).sum::<f64>() / n;
    let media_c = pontos.iter().map(|p| p.1).sum::<f64>() / n;
    let (mut cov, mut var) = (0.0, 0.0);
    for &(t, c) in pontos {
        cov += (t - media_t) * (c - media_c);
        var += (t - media_t) * (t - media_t);
    }
    if var == 0.0 { 0.0 } else { cov / var }
}

impl Freio {
    pub fn acionado(&self) -> bool {
        self.acionado
    }

    /// O PL1 de agora; antes da primeira leitura, o normal.
    pub fn pl1_w(&self, cfg: &Config) -> f64 {
        self.pl1_w.unwrap_or(cfg.pl1_normal_w)
    }

    /// Uma leitura: `agora_s` em segundos desde a subida, `temperatura` em °C.
    pub fn ler(
        &mut self,
        agora_s: f64,
        temperatura: f64,
        prevista_processos_c: Option<f64>,
        cfg: &Config,
    ) -> Decisao {
        let anterior = self.historico.back().map(|p| p.0);
        self.historico.push_back((agora_s, temperatura));
        while self.historico.len() > cfg.janela_leituras as usize {
            self.historico.pop_front();
        }

        let tendencia_c_s = tendencia(&self.historico);
        // Só a subida projeta: esfriando, vale a temperatura de agora.
        let prevista_c = temperatura + tendencia_c_s.max(0.0) * cfg.previsao_s;
        let pela_temperatura = temperatura.max(prevista_c);
        let maior = prevista_processos_c.map_or(pela_temperatura, |p| p.max(pela_temperatura));
        let antecipado = prevista_processos_c.is_some_and(|p| p > pela_temperatura)
            && curva(maior, cfg) < curva(pela_temperatura, cfg);
        let desejado = curva(maior, cfg);

        let pl1_w = match (self.pl1_w, anterior) {
            (Some(atual), _) if desejado <= atual => desejado,
            (Some(atual), Some(antes)) => {
                let dt = (agora_s - antes).max(0.0);
                desejado.min(atual + cfg.subida_w_por_s * dt)
            }
            _ => desejado,
        };
        self.pl1_w = Some(pl1_w);

        let acionado = pl1_w < cfg.pl1_normal_w - 0.05;
        let mudanca = match (self.acionado, acionado) {
            (false, true) => Some(Mudanca::Acionou),
            (true, false) => Some(Mudanca::Desligou),
            _ => None,
        };
        self.acionado = acionado;

        let faixa = cfg.pl1_normal_w - cfg.pl1_freio_w;
        let intensidade = if faixa > 0.0 {
            ((cfg.pl1_normal_w - pl1_w) / faixa).clamp(0.0, 1.0)
        } else {
            0.0
        };
        Decisao {
            tendencia_c_s,
            prevista_c,
            prevista_processos_c,
            antecipado,
            pl1_w,
            intensidade,
            mudanca,
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn cfg() -> Config {
        Config::default()
    }

    fn quase(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn a_curva_vai_do_normal_ao_freio_em_linha_reta() {
        let c = cfg();
        assert!(quase(curva(60.0, &c), 38.0));
        assert!(quase(curva(80.0, &c), 38.0));
        assert!(
            quase(curva(86.0, &c), 31.5),
            "metade do caminho: {}",
            curva(86.0, &c)
        );
        assert!(quase(curva(92.0, &c), 25.0));
        assert!(quase(curva(99.0, &c), 25.0));
    }

    #[test]
    fn tendencia_e_a_inclinacao_da_reta() {
        let p: VecDeque<_> = [(0.0, 70.0), (2.0, 72.0), (4.0, 74.0), (6.0, 76.0)].into();
        assert!(quase(tendencia(&p), 1.0));
        let q: VecDeque<_> = [(0.0, 80.0)].into();
        assert!(quase(tendencia(&q), 0.0));
    }

    #[test]
    fn freia_antes_do_pico_pela_previsao() {
        // Subindo 1,2 °C/s e ainda em 78 °C: a previsão (90 °C em 10 s) já
        // freia, quando a regra antiga só agiria depois de 5 leituras a 92 °C.
        let c = cfg();
        let mut f = Freio::default();
        let mut d = f.ler(0.0, 68.4, None, &c);
        for (i, t) in [70.8, 73.2, 75.6, 78.0].into_iter().enumerate() {
            d = f.ler(2.0 * (i + 1) as f64, t, None, &c);
        }
        assert!(quase(d.tendencia_c_s, 1.2));
        assert!(quase(d.prevista_c, 90.0));
        assert!(d.pl1_w < 30.0, "freou antes do pico: {} W", d.pl1_w);
        assert!(f.acionado());
    }

    #[test]
    fn temperatura_estavel_nao_freia() {
        let c = cfg();
        let mut f = Freio::default();
        for i in 0..10 {
            let d = f.ler(2.0 * f64::from(i), 75.0, None, &c);
            assert!(quase(d.pl1_w, 38.0));
            assert_eq!(d.mudanca, None);
        }
    }

    #[test]
    fn freia_na_hora_e_solta_devagar() {
        let c = cfg();
        let mut f = Freio::default();
        let d = f.ler(0.0, 95.0, None, &c);
        assert!(quase(d.pl1_w, 25.0));
        assert_eq!(d.mudanca, Some(Mudanca::Acionou));
        assert!(quase(d.intensidade, 1.0));
        // Esfriou de vez: o PL1 sobe só 0,5 W/s (1 W a cada leitura de 2 s).
        let mut t = 0.0;
        let mut pl1 = d.pl1_w;
        let mut desligou_em = None;
        for _ in 0..20 {
            t += 2.0;
            let d = f.ler(t, 60.0, None, &c);
            assert!(d.pl1_w <= pl1 + 1.0 + 1e-9, "subiu rápido demais");
            pl1 = d.pl1_w;
            if d.mudanca == Some(Mudanca::Desligou) {
                desligou_em = Some(t);
            }
        }
        assert_eq!(desligou_em, Some(26.0), "13 W a 0,5 W/s = 26 s");
        assert!(quase(pl1, 38.0));
    }

    #[test]
    fn calor_de_novo_no_meio_da_volta_freia_outra_vez() {
        let c = cfg();
        let mut f = Freio::default();
        f.ler(0.0, 95.0, None, &c);
        f.ler(2.0, 60.0, None, &c);
        f.ler(4.0, 60.0, None, &c);
        let d = f.ler(6.0, 93.0, None, &c);
        assert!(quase(d.pl1_w, 25.0));
    }

    #[test]
    fn processo_suspeito_freia_antes_de_o_sensor_reagir() {
        let c = cfg();
        let mut f = Freio::default();
        for i in 0..4 {
            f.ler(2.0 * f64::from(i), 60.0, None, &c);
        }
        // 60 °C e parada, mas o histórico diz que o rustc leva a 90 °C.
        let d = f.ler(8.0, 60.0, Some(90.0), &c);
        assert!(d.antecipado);
        assert!(d.pl1_w < 30.0, "{}", d.pl1_w);
        assert_eq!(d.mudanca, Some(Mudanca::Acionou));
    }

    #[test]
    fn previsao_por_processos_mais_fria_nao_antecipa() {
        let c = cfg();
        let mut f = Freio::default();
        let d = f.ler(0.0, 85.0, Some(70.0), &c);
        assert!(!d.antecipado);
    }

    #[test]
    fn esfriando_nao_preve_para_baixo() {
        let c = cfg();
        let mut f = Freio::default();
        let mut d = f.ler(0.0, 90.0, None, &c);
        for (i, t) in [88.0, 86.0, 84.0].into_iter().enumerate() {
            d = f.ler(2.0 * (i + 1) as f64, t, None, &c);
        }
        assert!(d.tendencia_c_s < 0.0);
        assert!(quase(d.prevista_c, 84.0));
    }
}

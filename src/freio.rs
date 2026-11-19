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
//!
//! **O alvo** (`alvo_c`, ou `sudo termostato CPU50C`): além da curva, um
//! controle integral segura a CPU numa temperatura. Acima do alvo (a de agora:
//! a previsão é da curva), o PL1 do alvo desce `alvo_ganho_w_por_c_s` × °C de erro por
//! segundo; abaixo, sobe no máximo `subida_w_por_s`. Fica entre o
//! `pl1_minimo_w` e o `pl1_normal_w`, e vale o menor entre ele e a curva.

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
    /// O alvo de temperatura (e não a curva) é quem limita o PL1 agora.
    pub pelo_alvo: bool,
    pub mudanca: Option<Mudanca>,
}

#[derive(Debug, Default, Clone)]
pub struct Freio {
    /// (segundos, °C) das últimas leituras.
    historico: VecDeque<(f64, f64)>,
    pl1_w: Option<f64>,
    acionado: bool,
    /// O PL1 que o controle do alvo quer; `None` sem alvo.
    pl1_do_alvo: Option<f64>,
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
        let pela_curva = curva(maior, cfg);
        let dt = anterior.map_or(0.0, |antes| (agora_s - antes).max(0.0));
        // O alvo olha a temperatura de agora: a previsão fica para a curva, a
        // guarda do pico. Com ela, os picos curtos do Haswell assustavam o
        // controle (visto em 19/11/2026: com previsão, 57–66 °C e o PL1 caindo
        // a 8 W num tranco; sem, 60–69 °C em volta de 65 e o PL1 em 14–21 W).
        let do_alvo = self.seguir_o_alvo(temperatura, dt, cfg);
        let pelo_alvo = do_alvo.is_some_and(|p| p < pela_curva);
        let desejado = do_alvo.map_or(pela_curva, |p| p.min(pela_curva));

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
            pelo_alvo,
            mudanca,
        }
    }

    /// O controle integral do alvo: o PL1 que segura a temperatura nele.
    fn seguir_o_alvo(&mut self, referencia_c: f64, dt: f64, cfg: &Config) -> Option<f64> {
        if cfg.alvo_c <= 0.0 {
            self.pl1_do_alvo = None;
            return None;
        }
        let atual = self
            .pl1_do_alvo
            .or(self.pl1_w)
            .unwrap_or(cfg.pl1_normal_w)
            .min(cfg.pl1_normal_w);
        let erro = referencia_c - cfg.alvo_c;
        let passo = cfg.alvo_ganho_w_por_c_s * erro.abs() * dt;
        let novo = if erro > 0.0 {
            atual - passo
        } else {
            atual + passo.min(cfg.subida_w_por_s * dt)
        };
        let novo = novo.clamp(cfg.pl1_minimo_w, cfg.pl1_normal_w);
        self.pl1_do_alvo = Some(novo);
        Some(novo)
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

    fn com_alvo(alvo: f64) -> Config {
        Config {
            alvo_c: alvo,
            ..Config::default()
        }
    }

    #[test]
    fn sem_alvo_nada_muda() {
        let c = cfg();
        let mut f = Freio::default();
        for i in 0..10 {
            let d = f.ler(2.0 * f64::from(i), 60.0, None, &c);
            assert!(quase(d.pl1_w, 38.0));
            assert!(!d.pelo_alvo);
        }
    }

    #[test]
    fn acima_do_alvo_o_pl1_desce_ate_o_minimo() {
        // 60 °C com alvo de 50: 10 °C de erro × 0,5 W/(°C·s) = 5 W/s.
        let c = com_alvo(50.0);
        let mut f = Freio::default();
        let d0 = f.ler(0.0, 60.0, None, &c);
        assert!(
            quase(d0.pl1_w, 38.0),
            "primeira leitura sem dt: {}",
            d0.pl1_w
        );
        let d1 = f.ler(2.0, 60.0, None, &c);
        assert!(quase(d1.pl1_w, 28.0), "{}", d1.pl1_w);
        assert!(d1.pelo_alvo);
        assert_eq!(d1.mudanca, Some(Mudanca::Acionou));
        let mut d = d1;
        for i in 2..20 {
            d = f.ler(2.0 * f64::from(i), 60.0, None, &c);
        }
        assert!(quase(d.pl1_w, c.pl1_minimo_w), "piso: {}", d.pl1_w);
    }

    #[test]
    fn abaixo_do_alvo_sobe_devagar_e_volta_ao_normal() {
        let c = com_alvo(50.0);
        let mut f = Freio::default();
        for i in 0..20 {
            f.ler(2.0 * f64::from(i), 60.0, None, &c);
        }
        // Esfriou para 40 °C: o erro pediria 5 W/s, mas a subida é de 0,5 W/s.
        let d = f.ler(40.0, 40.0, None, &c);
        assert!(quase(d.pl1_w, c.pl1_minimo_w + 1.0), "{}", d.pl1_w);
        let mut d = d;
        for i in 21..80 {
            d = f.ler(2.0 * f64::from(i), 40.0, None, &c);
        }
        assert!(quase(d.pl1_w, 38.0));
        assert!(!f.acionado());
    }

    #[test]
    fn no_alvo_o_pl1_para() {
        let c = com_alvo(50.0);
        let mut f = Freio::default();
        f.ler(0.0, 60.0, None, &c);
        let a = f.ler(2.0, 60.0, None, &c).pl1_w;
        let b = f.ler(4.0, 50.0, None, &c).pl1_w;
        // Tendência de descida não projeta: a 50 °C, erro zero, o PL1 fica.
        assert!(quase(a, b), "{a} → {b}");
    }

    #[test]
    fn a_curva_ainda_manda_quando_freia_mais() {
        // Alvo alto (90 °C), mas 92 °C agora: a curva leva a 25 W de uma vez.
        let c = com_alvo(90.0);
        let mut f = Freio::default();
        let d = f.ler(0.0, 92.0, None, &c);
        assert!(quase(d.pl1_w, 25.0));
        assert!(!d.pelo_alvo);
    }

    #[test]
    fn tirar_o_alvo_solta_devagar() {
        let mut c = com_alvo(50.0);
        let mut f = Freio::default();
        for i in 0..20 {
            f.ler(2.0 * f64::from(i), 60.0, None, &c);
        }
        c.alvo_c = 0.0;
        let d = f.ler(40.0, 60.0, None, &c);
        assert!(quase(d.pl1_w, c.pl1_minimo_w + 1.0), "{}", d.pl1_w);
        assert!(!d.pelo_alvo);
    }

    #[test]
    fn o_alvo_nao_se_assusta_com_a_previsao() {
        // Subindo rápido mas ainda abaixo do alvo: a curva não freia (abaixo de
        // 80 °C mesmo prevista) e o alvo olha a de agora, então o PL1 fica.
        let c = com_alvo(65.0);
        let mut f = Freio::default();
        let mut d = f.ler(0.0, 56.0, None, &c);
        for (i, t) in [58.0, 60.0, 62.0, 64.0].into_iter().enumerate() {
            d = f.ler(2.0 * (i + 1) as f64, t, None, &c);
        }
        assert!(d.prevista_c > 70.0, "a previsão sobe: {}", d.prevista_c);
        assert!(quase(d.pl1_w, 38.0), "{}", d.pl1_w);
    }
}

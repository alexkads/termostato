//! O freio de calor: a regra, sem tocar em hardware.
//!
//! Com `leituras_para_frear` leituras seguidas em `freio_acima_c` ou mais, o
//! freio puxa (o PL1 cai para `pl1_freio_w`). Ele só solta abaixo de
//! `solta_abaixo_c`. Entre as duas temperaturas nada muda: é a histerese que
//! impede o freio de ligar e desligar a cada leitura.

use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mudanca {
    Puxou,
    Soltou,
}

#[derive(Debug, Default, Clone)]
pub struct Freio {
    quentes_seguidas: u32,
    puxado: bool,
}

impl Freio {
    pub fn puxado(&self) -> bool {
        self.puxado
    }

    /// Uma leitura. Devolve a mudança, quando houver.
    pub fn ler(&mut self, temperatura: f64, cfg: &Config) -> Option<Mudanca> {
        if temperatura >= cfg.freio_acima_c {
            self.quentes_seguidas = self.quentes_seguidas.saturating_add(1);
        } else {
            self.quentes_seguidas = 0;
        }
        if !self.puxado && self.quentes_seguidas >= cfg.leituras_para_frear {
            self.puxado = true;
            return Some(Mudanca::Puxou);
        }
        if self.puxado && temperatura < cfg.solta_abaixo_c {
            self.puxado = false;
            self.quentes_seguidas = 0;
            return Some(Mudanca::Soltou);
        }
        None
    }

    /// O PL1 que vale agora, em watts.
    pub fn pl1_w(&self, cfg: &Config) -> f64 {
        if self.puxado {
            cfg.pl1_freio_w
        } else {
            cfg.pl1_normal_w
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn cfg() -> Config {
        Config::default()
    }

    #[test]
    fn so_puxa_depois_de_cinco_leituras_quentes_seguidas() {
        let c = cfg();
        let mut f = Freio::default();
        for _ in 0..4 {
            assert_eq!(f.ler(93.0, &c), None);
        }
        assert_eq!(f.ler(93.0, &c), Some(Mudanca::Puxou));
        assert!(f.puxado());
        assert!((f.pl1_w(&c) - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn uma_leitura_fria_no_meio_zera_a_contagem() {
        let c = cfg();
        let mut f = Freio::default();
        for _ in 0..4 {
            f.ler(93.0, &c);
        }
        f.ler(91.0, &c);
        for _ in 0..4 {
            assert_eq!(f.ler(93.0, &c), None);
        }
        assert_eq!(f.ler(93.0, &c), Some(Mudanca::Puxou));
    }

    #[test]
    fn entre_80_e_92_o_freio_continua_puxado() {
        let c = cfg();
        let mut f = Freio::default();
        for _ in 0..5 {
            f.ler(95.0, &c);
        }
        for t in [91.0, 88.0, 84.0, 80.0] {
            assert_eq!(f.ler(t, &c), None, "{t} °C não solta");
            assert!(f.puxado());
        }
        assert_eq!(f.ler(79.0, &c), Some(Mudanca::Soltou));
        assert!((f.pl1_w(&c) - 38.0).abs() < f64::EPSILON);
    }

    #[test]
    fn depois_de_soltar_precisa_de_cinco_quentes_de_novo() {
        let c = cfg();
        let mut f = Freio::default();
        for _ in 0..5 {
            f.ler(95.0, &c);
        }
        f.ler(70.0, &c);
        for _ in 0..4 {
            assert_eq!(f.ler(95.0, &c), None);
        }
        assert_eq!(f.ler(95.0, &c), Some(Mudanca::Puxou));
    }
}

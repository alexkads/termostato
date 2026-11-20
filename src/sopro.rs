//! A curva das ventoinhas: soprar só quando precisa.
//!
//! O dono, 20/11/2026: *"Eu gostaria que o cooler não ficasse acelerado sem
//! necessidade"*. No modo `"curva"` o piso de cada ventoinha acompanha a
//! temperatura: fica no mínimo até `ventoinhas_calma_ate_c`, sobe em linha
//! reta e chega ao máximo em `ventoinhas_maximo_em_c`.
//!
//! A curva olha a temperatura **suavizada** (média móvel exponencial de
//! `ventoinhas_suavizacao_s`), e não a de agora nem a previsão do freio. Na
//! fedora, cada arrancada curta do turbo leva a CPU a 70–77 °C por poucos
//! segundos, e o freio aciona pela previsão (visto em 20/11/2026): seguir o
//! freio punha as ventoinhas no máximo a cada clique. Quem segura o pico é o
//! freio, cortando a potência na hora; a ventoinha cuida do calor que fica.
//!
//! O piso desce devagar (`ventoinhas_descida_rpm_por_s`), para não virar
//! sanfona. Sem leitura de temperatura, vai ao máximo. O SMC continua livre
//! para soprar acima do piso, se ele quiser.

use crate::config::Config;

/// Degraus do piso: só se grava quando o piso muda de degrau, e não a cada
/// RPM, para não escrever no SMC a cada leitura enquanto ele desce.
pub const DEGRAU_RPM: u64 = 100;

/// O piso que a curva pede para a temperatura `t`, entre `minimo` e `maximo`.
pub fn pela_curva(t: f64, minimo: u64, maximo: u64, cfg: &Config) -> u64 {
    let minimo = minimo.min(maximo);
    let fracao = ((t - cfg.ventoinhas_calma_ate_c)
        / (cfg.ventoinhas_maximo_em_c - cfg.ventoinhas_calma_ate_c))
        .clamp(0.0, 1.0);
    minimo + ((maximo - minimo) as f64 * fracao).round() as u64
}

/// O piso de uma ventoinha, com a temperatura suavizada e a memória de onde
/// o piso estava, para descer devagar.
#[derive(Debug, Default, Clone)]
pub struct Sopro {
    suavizada: Option<f64>,
    atual: Option<f64>,
}

impl Sopro {
    /// A temperatura que a curva usa, depois da leitura `t`.
    pub fn suavizada(&self) -> Option<f64> {
        self.suavizada
    }

    /// O piso a gravar agora, já em degraus de [`DEGRAU_RPM`]. `t` não finito
    /// (sem leitura) pede o máximo e não entra na média.
    pub fn decidir(&mut self, t: f64, dt: f64, minimo: u64, maximo: u64, cfg: &Config) -> u64 {
        let minimo = minimo.min(maximo);
        let pedido = if t.is_finite() {
            let tau = cfg.ventoinhas_suavizacao_s;
            let media = match self.suavizada {
                Some(antes) if tau > 0.0 => {
                    antes + (t - antes) * (1.0 - (-dt.max(0.0) / tau).exp())
                }
                _ => t,
            };
            self.suavizada = Some(media);
            pela_curva(media, minimo, maximo, cfg)
        } else {
            maximo
        } as f64;
        let novo = match self.atual {
            // Sobe junto com a média; desce no máximo `descida` por segundo.
            Some(antes) if pedido < antes => {
                (antes - cfg.ventoinhas_descida_rpm_por_s * dt.max(0.0)).max(pedido)
            }
            _ => pedido,
        };
        self.atual = Some(novo);
        let degrau = ((novo / DEGRAU_RPM as f64).round() as u64) * DEGRAU_RPM;
        degrau.clamp(minimo, maximo)
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn cfg() -> Config {
        Config::default()
    }

    #[test]
    fn fria_fica_no_minimo_e_quente_no_maximo() {
        let c = cfg();
        assert_eq!(pela_curva(45.0, 2000, 6156, &c), 2000);
        assert_eq!(pela_curva(c.ventoinhas_calma_ate_c, 2000, 6156, &c), 2000);
        assert_eq!(pela_curva(c.ventoinhas_maximo_em_c, 2000, 6156, &c), 6156);
        assert_eq!(pela_curva(95.0, 2000, 6156, &c), 6156);
        let meio = f64::midpoint(c.ventoinhas_calma_ate_c, c.ventoinhas_maximo_em_c);
        assert_eq!(pela_curva(meio, 2000, 6156, &c), 4078);
    }

    #[test]
    fn arrancada_curta_do_turbo_quase_nao_mexe() {
        // O que a fedora fez em 20/11/2026: 50 °C, um pico de 4 s a 77 °C, volta.
        let c = cfg();
        let mut s = Sopro::default();
        for _ in 0..30 {
            s.decidir(50.0, 2.0, 2000, 6156, &c);
        }
        let mut maior = 0;
        for t in [77.0, 77.0, 55.0, 50.0] {
            maior = maior.max(s.decidir(t, 2.0, 2000, 6156, &c));
        }
        assert_eq!(maior, 2000, "a média nem chegou aos 60 °C");
    }

    #[test]
    fn carga_longa_leva_ao_maximo_em_menos_de_um_minuto() {
        let c = cfg();
        let mut s = Sopro::default();
        s.decidir(50.0, 2.0, 2000, 6156, &c);
        let mut segundos = 0.0;
        while s.decidir(90.0, 2.0, 2000, 6156, &c) < 6156 {
            segundos += 2.0;
            assert!(segundos < 60.0, "demorou demais: {segundos} s");
        }
    }

    #[test]
    fn desce_devagar() {
        let c = Config {
            ventoinhas_suavizacao_s: 0.0,
            ..cfg()
        };
        let mut s = Sopro::default();
        assert_eq!(s.decidir(85.0, 2.0, 2000, 6000, &c), 6000);
        let passo = (c.ventoinhas_descida_rpm_por_s * 2.0) as u64;
        assert_eq!(s.decidir(45.0, 2.0, 2000, 6000, &c), 6000 - passo);
        let mut piso = 0;
        for _ in 0..200 {
            piso = s.decidir(45.0, 2.0, 2000, 6000, &c);
        }
        assert_eq!(piso, 2000, "acaba no mínimo");
    }

    #[test]
    fn sem_leitura_vai_ao_maximo_e_nao_estraga_a_media() {
        let c = cfg();
        let mut s = Sopro::default();
        s.decidir(50.0, 2.0, 2000, 6156, &c);
        assert_eq!(s.decidir(f64::INFINITY, 2.0, 2000, 6156, &c), 6156);
        assert_eq!(s.suavizada(), Some(50.0));
    }

    #[test]
    fn grava_em_degraus_e_aguenta_minimo_acima_do_maximo() {
        let c = cfg();
        let mut s = Sopro::default();
        assert_eq!(s.decidir(61.0, 2.0, 2000, 6156, &c) % DEGRAU_RPM, 0);
        let mut s = Sopro::default();
        assert_eq!(s.decidir(40.0, 2.0, 9000, 6156, &c), 6156);
    }
}

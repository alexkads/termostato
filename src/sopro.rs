//! A curva das ventoinhas: soprar só quando precisa.
//!
//! O dono, 20/11/2026: *"Eu gostaria que o cooler não ficasse acelerado sem
//! necessidade"*. No modo `"curva"` o piso de cada ventoinha acompanha a
//! temperatura: fica no mínimo até `ventoinhas_calma_ate_c`, sobe em linha
//! reta e chega ao máximo em `ventoinhas_maximo_em_c` (o padrão é o mesmo
//! ponto em que o freio começa, para o MacBook já estar soprando tudo quando
//! a CPU chega lá). Com o freio acionado, vai ao máximo de qualquer jeito.
//!
//! Como o freio, **sobe na hora e desce devagar** (`ventoinhas_descida_rpm_por_s`):
//! uma arrancada curta do turbo não faz a ventoinha subir e descer como
//! sanfona. O SMC continua livre para soprar acima do piso, se ele quiser.

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

/// O piso de uma ventoinha, com a memória de onde estava para descer devagar.
#[derive(Debug, Default, Clone)]
pub struct Sopro {
    atual: Option<f64>,
}

impl Sopro {
    /// O piso a gravar agora, já em degraus de [`DEGRAU_RPM`].
    pub fn decidir(
        &mut self,
        t: f64,
        freio_acionado: bool,
        dt: f64,
        minimo: u64,
        maximo: u64,
        cfg: &Config,
    ) -> u64 {
        let pedido = if freio_acionado {
            maximo
        } else {
            pela_curva(t, minimo, maximo, cfg)
        } as f64;
        let novo = match self.atual {
            // Sobe na hora; desce no máximo `descida` por segundo.
            Some(antes) if pedido < antes => {
                (antes - cfg.ventoinhas_descida_rpm_por_s * dt.max(0.0)).max(pedido)
            }
            _ => pedido,
        };
        self.atual = Some(novo);
        let degrau = ((novo / DEGRAU_RPM as f64).round() as u64) * DEGRAU_RPM;
        degrau.clamp(minimo.min(maximo), maximo)
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
        // No meio da reta, no meio do caminho.
        let meio = f64::midpoint(c.ventoinhas_calma_ate_c, c.ventoinhas_maximo_em_c);
        assert_eq!(pela_curva(meio, 2000, 6156, &c), 4078);
    }

    #[test]
    fn sobe_na_hora_e_desce_devagar() {
        let c = cfg();
        let mut s = Sopro::default();
        assert_eq!(s.decidir(50.0, false, 2.0, 2000, 6000, &c), 2000);
        // Arrancada: vai ao máximo na mesma leitura.
        assert_eq!(s.decidir(85.0, false, 2.0, 2000, 6000, &c), 6000);
        // Esfriou de uma vez: desce só `descida * dt` por leitura.
        let passo = (c.ventoinhas_descida_rpm_por_s * 2.0) as u64;
        assert_eq!(s.decidir(45.0, false, 2.0, 2000, 6000, &c), 6000 - passo);
        let mut piso = 0;
        for _ in 0..200 {
            piso = s.decidir(45.0, false, 2.0, 2000, 6000, &c);
        }
        assert_eq!(piso, 2000, "acaba no mínimo");
    }

    #[test]
    fn freio_acionado_e_sempre_o_maximo() {
        let mut s = Sopro::default();
        assert_eq!(s.decidir(50.0, true, 2.0, 2000, 6156, &cfg()), 6156);
    }

    #[test]
    fn grava_em_degraus() {
        let c = cfg();
        let mut s = Sopro::default();
        let t = c.ventoinhas_calma_ate_c + 1.0;
        assert_eq!(s.decidir(t, false, 2.0, 2000, 6156, &c) % DEGRAU_RPM, 0);
    }

    #[test]
    fn minimo_acima_do_maximo_nao_quebra() {
        let mut s = Sopro::default();
        assert_eq!(s.decidir(40.0, false, 2.0, 9000, 6156, &cfg()), 6156);
    }
}

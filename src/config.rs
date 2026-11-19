//! A configuração: um arquivo TOML só, `/etc/termostato.toml`.
//!
//! Tudo o que o dono ajusta mora aqui, e nada no sistema operacional: os watts,
//! as temperaturas do freio e as ventoinhas. Campo ausente usa o padrão abaixo,
//! então um arquivo vazio (ou nenhum arquivo) já é uma configuração válida.

use std::path::Path;

use serde::Deserialize;

/// O que fazer com as ventoinhas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Ventoinhas {
    /// O piso de cada ventoinha vai ao máximo dela: o MacBook não espera
    /// esquentar para soprar (a saída de ar é pela dobradiça).
    Maximo,
    /// O termostato não mexe nelas: o `applesmc` decide.
    Automatico,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Limite de potência sustentada (RAPL PL1) no dia a dia, em watts. O
    /// firmware da Apple deixa 100 W, que na prática é sem limite.
    pub pl1_normal_w: f64,
    /// O PL1 enquanto o freio estiver puxado.
    pub pl1_freio_w: f64,
    /// Temperatura do pacote que conta como "quente" para o freio.
    pub freio_acima_c: f64,
    /// Abaixo desta, o freio solta. A distância evita ligar e desligar a cada
    /// leitura em volta de um limite só.
    pub solta_abaixo_c: f64,
    /// Quantas leituras quentes seguidas puxam o freio.
    pub leituras_para_frear: u32,
    /// Intervalo entre leituras, em segundos.
    pub intervalo_s: f64,
    pub ventoinhas: Ventoinhas,
    /// Onde o `/metrics` escuta. Só no loopback: o serviço roda como root.
    pub escuta: String,
    /// A raiz do sysfs. Só os testes mudam isto.
    pub sysfs: String,
    /// Onde ficam os valores de fábrica (`fabrica.toml`), para o
    /// `--desinstalar`. É o `StateDirectory` do serviço.
    pub estado: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            pl1_normal_w: 38.0,
            pl1_freio_w: 25.0,
            freio_acima_c: 92.0,
            solta_abaixo_c: 80.0,
            leituras_para_frear: 5,
            intervalo_s: 2.0,
            ventoinhas: Ventoinhas::Maximo,
            escuta: "127.0.0.1:9101".into(),
            sysfs: "/sys".into(),
            estado: "/var/lib/termostato".into(),
        }
    }
}

impl Config {
    /// Lê o arquivo; sem arquivo, os padrões.
    pub fn ler(caminho: &Path) -> Result<Self, String> {
        let config = match std::fs::read_to_string(caminho) {
            Ok(texto) => toml::from_str(&texto)
                .map_err(|e| format!("{} não é uma configuração válida: {e}", caminho.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(format!("não li {}: {e}", caminho.display())),
        };
        config.validar()?;
        Ok(config)
    }

    /// Recusa o que deixaria a máquina sem proteção ou o freio sem sentido.
    pub fn validar(&self) -> Result<(), String> {
        if !(5.0..=150.0).contains(&self.pl1_normal_w) {
            return Err(format!(
                "pl1_normal_w = {} fora de 5 a 150 W",
                self.pl1_normal_w
            ));
        }
        if !(5.0..=self.pl1_normal_w).contains(&self.pl1_freio_w) {
            return Err(format!(
                "pl1_freio_w = {} precisa ficar entre 5 W e o pl1_normal_w ({})",
                self.pl1_freio_w, self.pl1_normal_w
            ));
        }
        if self.solta_abaixo_c >= self.freio_acima_c {
            return Err(format!(
                "solta_abaixo_c ({}) precisa ser menor que freio_acima_c ({})",
                self.solta_abaixo_c, self.freio_acima_c
            ));
        }
        if self.freio_acima_c > 99.0 {
            return Err(format!(
                "freio_acima_c = {} passa do crítico (100 °C)",
                self.freio_acima_c
            ));
        }
        if self.leituras_para_frear == 0 {
            return Err("leituras_para_frear precisa ser pelo menos 1".into());
        }
        if !(0.5..=60.0).contains(&self.intervalo_s) {
            return Err(format!(
                "intervalo_s = {} fora de 0,5 a 60 s",
                self.intervalo_s
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn arquivo_vazio_vale_os_padroes() {
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c, Config::default());
        c.validar().unwrap();
    }

    #[test]
    fn so_o_que_mudou() {
        let c: Config = toml::from_str("pl1_normal_w = 42\nventoinhas = \"automatico\"").unwrap();
        assert!((c.pl1_normal_w - 42.0).abs() < f64::EPSILON);
        assert_eq!(c.ventoinhas, Ventoinhas::Automatico);
        assert!((c.pl1_freio_w - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn campo_com_erro_de_digitacao_e_recusado() {
        assert!(toml::from_str::<Config>("pl1_normal = 42").is_err());
    }

    #[test]
    fn freio_acima_do_normal_e_recusado() {
        let c = Config {
            pl1_freio_w: 50.0,
            ..Config::default()
        };
        assert!(c.validar().is_err());
    }

    #[test]
    fn histerese_invertida_e_recusada() {
        let c = Config {
            solta_abaixo_c: 95.0,
            ..Config::default()
        };
        assert!(c.validar().is_err());
    }

    #[test]
    fn sem_arquivo_sao_os_padroes() {
        let c = Config::ler(Path::new("/nao/existe/termostato.toml")).unwrap();
        assert_eq!(c, Config::default());
    }
}

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
    /// O PL1 com o freio no máximo: o fim da curva.
    pub pl1_freio_w: f64,
    /// Temperatura (de agora ou prevista) em que a curva começa a frear.
    // Os nomes da 0.3 continuam aceitos: o /etc/termostato.toml instalado os usa.
    #[serde(alias = "desliga_abaixo_c", alias = "solta_abaixo_c")]
    pub freio_comeca_c: f64,
    /// Temperatura (de agora ou prevista) em que o freio chega ao máximo.
    pub freio_acima_c: f64,
    /// Quantas leituras entram na reta da tendência.
    #[serde(alias = "leituras_para_frear")]
    pub janela_leituras: u32,
    /// Quantos segundos à frente a previsão olha.
    pub previsao_s: f64,
    /// Quanto o PL1 sobe por segundo ao soltar o freio. Frear é na hora.
    pub subida_w_por_s: f64,
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
            freio_comeca_c: 80.0,
            freio_acima_c: 92.0,
            janela_leituras: 5,
            previsao_s: 10.0,
            subida_w_por_s: 0.5,
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
        if self.freio_comeca_c >= self.freio_acima_c {
            return Err(format!(
                "freio_comeca_c ({}) precisa ser menor que freio_acima_c ({})",
                self.freio_comeca_c, self.freio_acima_c
            ));
        }
        if self.freio_acima_c > 99.0 {
            return Err(format!(
                "freio_acima_c = {} passa do crítico (100 °C)",
                self.freio_acima_c
            ));
        }
        if !(2..=60).contains(&self.janela_leituras) {
            return Err(format!(
                "janela_leituras = {} fora de 2 a 60",
                self.janela_leituras
            ));
        }
        if !(0.0..=120.0).contains(&self.previsao_s) {
            return Err(format!(
                "previsao_s = {} fora de 0 a 120 s",
                self.previsao_s
            ));
        }
        if !(0.05..=50.0).contains(&self.subida_w_por_s) {
            return Err(format!(
                "subida_w_por_s = {} fora de 0,05 a 50 W/s",
                self.subida_w_por_s
            ));
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
    fn curva_invertida_e_recusada() {
        let c = Config {
            freio_comeca_c: 95.0,
            ..Config::default()
        };
        assert!(c.validar().is_err());
    }

    #[test]
    fn arquivo_da_versao_anterior_continua_valendo() {
        let c: Config = toml::from_str(
            "pl1_normal_w = 38\npl1_freio_w = 25\nfreio_acima_c = 92\nleituras_para_frear = 5\nsolta_abaixo_c = 80",
        )
        .unwrap();
        assert_eq!(c.janela_leituras, 5);
        assert!((c.freio_comeca_c - 80.0).abs() < f64::EPSILON);
        c.validar().unwrap();
    }

    #[test]
    fn sem_arquivo_sao_os_padroes() {
        let c = Config::ler(Path::new("/nao/existe/termostato.toml")).unwrap();
        assert_eq!(c, Config::default());
    }
}

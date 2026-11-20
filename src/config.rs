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

/// O que as mãos do termostato fazem com os processos e contêineres.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Modo {
    /// Só registra o que faria: journal, métricas e anotação no Grafana.
    Observar,
    /// Segura de verdade, e devolve quando a máquina se acalma.
    Agir,
    /// Nem observa.
    Desligado,
}

/// Os processos: observar quem gasta, aprender quem esquenta, antecipar.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConfigProcessos {
    /// Aprender quem esquenta e guardar no histórico.
    pub aprender: bool,
    /// Usar o histórico para frear antes de a temperatura reagir.
    pub antecipar: bool,
    /// Nota de calor (°C/s) a partir da qual o processo é suspeito.
    pub suspeito_c_s: f64,
    /// Observações de calor necessárias antes de virar suspeito.
    pub minimo_de_observacoes: u32,
    /// Fatia mínima da CPU da máquina (0 a 1) para contar.
    pub fatia_minima: f64,
    /// Nomes não vistos há mais dias que isto saem do histórico.
    pub esquecer_dias: u64,
    /// Nomes que nunca entram no histórico.
    pub ignorar: Vec<String>,
}

impl Default for ConfigProcessos {
    fn default() -> Self {
        Self {
            aprender: true,
            antecipar: true,
            suspeito_c_s: 1.0,
            minimo_de_observacoes: 3,
            fatia_minima: 0.05,
            esquecer_dias: 30,
            ignorar: vec!["termostato".into()],
        }
    }
}

/// As mãos: segurar CPU e memória dos mal-educados, no cgroup deles.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConfigMaos {
    pub modo: Modo,
    /// Intensidade do freio de calor (0 a 1) a partir da qual segura a CPU.
    pub cpu_a_partir_de: f64,
    /// Nenhum cgroup fica com menos núcleos que isto.
    pub minimo_cpus: f64,
    /// Pressão de memória (PSI some avg10, %) que começa a segurar memória.
    pub memoria_aperto_pct: f64,
    /// Abaixo disto, por `calmo_s`, os limites voltam ao original.
    pub memoria_folga_pct: f64,
    /// Segundos de calma antes de devolver os limites.
    pub calmo_s: f64,
    /// Fatia mínima da CPU (0 a 1) para um cgroup ser segurado.
    pub fatia_minima: f64,
    /// Começos de cgroup ou nomes de contêiner que nunca são segurados.
    pub nunca: Vec<String>,
}

impl Default for ConfigMaos {
    fn default() -> Self {
        Self {
            modo: Modo::Observar,
            cpu_a_partir_de: 0.3,
            minimo_cpus: 1.0,
            memoria_aperto_pct: 10.0,
            memoria_folga_pct: 2.0,
            calmo_s: 60.0,
            fatia_minima: 0.2,
            nunca: vec![
                "/init.scope".into(),
                "/system.slice/termostato.service".into(),
                "/system.slice/sshd.service".into(),
                "/system.slice/systemd-".into(),
                "/system.slice/tailscaled.service".into(),
                "/system.slice/docker.service".into(),
                "/system.slice/containerd.service".into(),
                "monitoramento-".into(),
                "testcontainers-ryuk-".into(),
            ],
        }
    }
}

/// O painel em `/` e a API dele.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConfigPainel {
    /// Logins da tailnet que podem mudar ajustes pelo painel (o cabeçalho
    /// `Tailscale-User-Login` que o `tailscale serve` põe). Vazio: só mostra.
    pub quem_pode_ajustar: Vec<String>,
}

/// Os avisos ao Grafana: uma anotação por intervenção.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConfigGrafana {
    /// Vazio = sem anotações (as métricas continuam).
    pub url: String,
    /// Arquivo com o token de uma conta de serviço do Grafana (modo 600).
    pub token_arquivo: String,
}

impl Default for ConfigGrafana {
    fn default() -> Self {
        Self {
            url: String::new(),
            token_arquivo: "/etc/termostato/grafana.token".into(),
        }
    }
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
    /// Alvo de temperatura (°C): o termostato segura a CPU nele, baixando o
    /// PL1 até o `pl1_minimo_w`. 0 = sem alvo, só a curva. Atalho na linha de
    /// comando: `sudo termostato CPU50C` (ver `ajustes.rs`).
    pub alvo_c: f64,
    /// O PL1 mais baixo a que o alvo pode levar.
    pub pl1_minimo_w: f64,
    /// Quanto o PL1 desce por segundo para cada °C acima do alvo.
    pub alvo_ganho_w_por_c_s: f64,
    /// Intervalo entre leituras, em segundos.
    pub intervalo_s: f64,
    pub ventoinhas: Ventoinhas,
    /// Onde o `/metrics` escuta. Só no loopback: o serviço roda como root.
    pub escuta: String,
    /// A raiz do sysfs. Só os testes mudam isto.
    pub sysfs: String,
    /// Onde ficam os valores de fábrica (`fabrica.toml`), o histórico e os
    /// segurados. É o `StateDirectory` do serviço.
    pub estado: String,
    /// A raiz do procfs. Só os testes mudam isto.
    pub proc: String,
    /// O socket do Docker, para saber o nome dos contêineres e pedir que maneirem.
    pub docker_socket: String,
    /// Onde o driver `msr` do kernel expõe os registros de cada CPU.
    pub dev_cpu: String,
    /// O TCC offset gravado no chip (°C abaixo do TjMax em que a própria CPU
    /// se freia; 8 = 92 °C no i7-4770HQ). 0 deixa o de fábrica.
    pub tcc_offset_c: u8,
    /// O gatilho de pressão de memória no kernel: tempo parado (ms) dentro da
    /// janela (ms) que acorda o termostato. 0 desliga o gatilho. O kernel aceita
    /// janelas de 500 ms a 10 s. O kernel 7.3
    /// do Fedora só aceita janelas em múltiplos de 2 s (1 s é recusado com
    /// EINVAL mesmo como root, visto em 19/11/2026).
    pub psi_parado_ms: u64,
    pub psi_janela_ms: u64,
    pub processos: ConfigProcessos,
    pub maos: ConfigMaos,
    pub grafana: ConfigGrafana,
    pub painel: ConfigPainel,
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
            alvo_c: 0.0,
            pl1_minimo_w: 8.0,
            alvo_ganho_w_por_c_s: 0.5,
            intervalo_s: 2.0,
            ventoinhas: Ventoinhas::Maximo,
            escuta: "127.0.0.1:9101".into(),
            sysfs: "/sys".into(),
            estado: "/var/lib/termostato".into(),
            proc: "/proc".into(),
            docker_socket: "/var/run/docker.sock".into(),
            dev_cpu: "/dev/cpu".into(),
            tcc_offset_c: 8,
            psi_parado_ms: 150,
            psi_janela_ms: 1000,
            processos: ConfigProcessos::default(),
            maos: ConfigMaos::default(),
            grafana: ConfigGrafana::default(),
            painel: ConfigPainel::default(),
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

    /// O alvo de temperatura e o piso a que ele pode levar o PL1.
    fn validar_alvo(&self) -> Result<(), String> {
        if self.alvo_c != 0.0
            && !(crate::ajustes::ALVO_MINIMO_C..=crate::ajustes::ALVO_MAXIMO_C)
                .contains(&self.alvo_c)
        {
            return Err(format!(
                "alvo_c = {} fora de {} a {} °C (0 = sem alvo)",
                self.alvo_c,
                crate::ajustes::ALVO_MINIMO_C,
                crate::ajustes::ALVO_MAXIMO_C
            ));
        }
        if !(3.0..=self.pl1_freio_w).contains(&self.pl1_minimo_w) {
            return Err(format!(
                "pl1_minimo_w = {} precisa ficar entre 3 W e o pl1_freio_w ({})",
                self.pl1_minimo_w, self.pl1_freio_w
            ));
        }
        if !(0.01..=10.0).contains(&self.alvo_ganho_w_por_c_s) {
            return Err(format!(
                "alvo_ganho_w_por_c_s = {} fora de 0,01 a 10",
                self.alvo_ganho_w_por_c_s
            ));
        }
        Ok(())
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
        self.validar_alvo()?;
        if !(0.5..=60.0).contains(&self.intervalo_s) {
            return Err(format!(
                "intervalo_s = {} fora de 0,5 a 60 s",
                self.intervalo_s
            ));
        }
        if self.tcc_offset_c > crate::tcc::OFFSET_MAXIMO_C {
            return Err(format!(
                "tcc_offset_c = {} acima de {} °C",
                self.tcc_offset_c,
                crate::tcc::OFFSET_MAXIMO_C
            ));
        }
        if self.psi_parado_ms > 0
            && (self.psi_parado_ms >= self.psi_janela_ms
                || !(500..=10_000).contains(&self.psi_janela_ms))
        {
            return Err(format!(
                "psi_parado_ms ({}) precisa ser menor que psi_janela_ms ({}), e a janela entre 500 e 10.000 ms (o kernel recusa outras)",
                self.psi_parado_ms, self.psi_janela_ms
            ));
        }
        let p = &self.processos;
        if p.suspeito_c_s <= 0.0 {
            return Err(format!(
                "processos.suspeito_c_s = {} precisa ser maior que 0",
                p.suspeito_c_s
            ));
        }
        if !(0.0..=1.0).contains(&p.fatia_minima) {
            return Err(format!(
                "processos.fatia_minima = {} fora de 0 a 1",
                p.fatia_minima
            ));
        }
        if p.minimo_de_observacoes == 0 {
            return Err("processos.minimo_de_observacoes precisa ser pelo menos 1".into());
        }
        let m = &self.maos;
        if !(0.0..=1.0).contains(&m.cpu_a_partir_de) || !(0.0..=1.0).contains(&m.fatia_minima) {
            return Err("maos.cpu_a_partir_de e maos.fatia_minima ficam entre 0 e 1".into());
        }
        if m.minimo_cpus < 0.1 {
            return Err(format!(
                "maos.minimo_cpus = {} abaixo de 0,1 núcleo",
                m.minimo_cpus
            ));
        }
        if m.memoria_folga_pct >= m.memoria_aperto_pct {
            return Err(format!(
                "maos.memoria_folga_pct ({}) precisa ser menor que maos.memoria_aperto_pct ({})",
                m.memoria_folga_pct, m.memoria_aperto_pct
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
    fn as_maos_comecam_observando() {
        let c: Config = toml::from_str("").unwrap();
        assert_eq!(c.maos.modo, Modo::Observar);
        let c: Config = toml::from_str("[maos]\nmodo = \"agir\"").unwrap();
        assert_eq!(c.maos.modo, Modo::Agir);
        assert!(toml::from_str::<Config>("[maos]\nmodo = \"matar\"").is_err());
    }

    #[test]
    fn superpoderes_invalidos_sao_recusados() {
        let mut c = Config::default();
        c.processos.suspeito_c_s = 0.0;
        assert!(c.validar().is_err());
        let mut c = Config::default();
        c.maos.minimo_cpus = 0.05;
        assert!(c.validar().is_err());
        let mut c = Config::default();
        c.maos.memoria_folga_pct = 20.0;
        assert!(c.validar().is_err());
    }

    #[test]
    fn sem_arquivo_sao_os_padroes() {
        let c = Config::ler(Path::new("/nao/existe/termostato.toml")).unwrap();
        assert_eq!(c, Config::default());
    }
}

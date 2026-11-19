//! O hardware, pelo sysfs: o sensor da CPU, o limite de potência (RAPL), as
//! ventoinhas do MacBook e a frequência.
//!
//! O número de cada `hwmon` muda entre boots, então tudo é achado pelo `name`
//! do chip, nunca por caminho fixo. A raiz do sysfs vem da configuração, para
//! os testes montarem um sysfs de mentira numa pasta temporária.

use std::fs;
use std::path::{Path, PathBuf};

fn ler_u64(caminho: &Path) -> Result<u64, String> {
    let texto = fs::read_to_string(caminho).map_err(|e| format!("{}: {e}", caminho.display()))?;
    texto
        .trim()
        .parse()
        .map_err(|e| format!("{}: {e} ({texto:?})", caminho.display()))
}

fn escrever_u64(caminho: &Path, valor: u64) -> Result<(), String> {
    fs::write(caminho, valor.to_string()).map_err(|e| format!("{}: {e}", caminho.display()))
}

/// A pasta `hwmon*` cujo `name` é `chip`.
fn hwmon(sysfs: &Path, chip: &str) -> Option<PathBuf> {
    let base = sysfs.join("class/hwmon");
    let mut pastas: Vec<_> = fs::read_dir(&base)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    pastas.sort();
    pastas
        .into_iter()
        .find(|p| fs::read_to_string(p.join("name")).is_ok_and(|n| n.trim() == chip))
}

/// A temperatura do pacote da CPU (`coretemp`, `temp1` = "Package id 0").
pub struct Sensor {
    arquivo: PathBuf,
}

impl Sensor {
    pub fn achar(sysfs: &Path) -> Result<Self, String> {
        let pasta =
            hwmon(sysfs, "coretemp").ok_or("não achei o sensor coretemp (módulo carregado?)")?;
        Ok(Self {
            arquivo: pasta.join("temp1_input"),
        })
    }

    pub fn celsius(&self) -> Result<f64, String> {
        Ok(ler_u64(&self.arquivo)? as f64 / 1000.0)
    }
}

/// O limite de potência do pacote (`intel-rapl:0`).
pub struct Rapl {
    pasta: PathBuf,
    /// Para a potência medida: o contador de energia anterior e quando.
    anterior: Option<(u64, std::time::Instant)>,
}

impl Rapl {
    pub fn achar(sysfs: &Path) -> Result<Self, String> {
        let pasta = sysfs.join("class/powercap/intel-rapl:0");
        if !pasta.join("constraint_0_power_limit_uw").exists() {
            return Err(format!("não achei o RAPL em {}", pasta.display()));
        }
        Ok(Self {
            pasta,
            anterior: None,
        })
    }

    fn arquivo(&self, nome: &str) -> PathBuf {
        self.pasta.join(nome)
    }

    /// PL1 (longo prazo), em µW.
    pub fn pl1_uw(&self) -> Result<u64, String> {
        ler_u64(&self.arquivo("constraint_0_power_limit_uw"))
    }

    pub fn definir_pl1_uw(&self, uw: u64) -> Result<(), String> {
        escrever_u64(&self.arquivo("constraint_0_power_limit_uw"), uw)
    }

    /// PL2 (curto prazo), em µW. Só lido: o turbo curto fica como o firmware deixou.
    pub fn pl2_uw(&self) -> Result<u64, String> {
        ler_u64(&self.arquivo("constraint_1_power_limit_uw"))
    }

    /// A potência média desde a leitura anterior, em watts. Na primeira
    /// leitura não há o que comparar.
    pub fn potencia_w(&mut self) -> Result<Option<f64>, String> {
        let agora = std::time::Instant::now();
        let energia = ler_u64(&self.arquivo("energy_uj"))?;
        let teto = ler_u64(&self.arquivo("max_energy_range_uj")).unwrap_or(u64::MAX);
        let anterior = self.anterior.replace((energia, agora));
        Ok(anterior.and_then(|(antes, quando)| {
            let segundos = agora.duration_since(quando).as_secs_f64();
            (segundos > 0.0).then(|| delta_de_energia(antes, energia, teto) as f64 / 1e6 / segundos)
        }))
    }
}

/// A energia gasta entre duas leituras, contando a volta do contador.
pub fn delta_de_energia(antes: u64, agora: u64, teto: u64) -> u64 {
    if agora >= antes {
        agora - antes
    } else {
        teto - antes + agora
    }
}

/// Uma ventoinha do `applesmc`.
pub struct Ventoinha {
    pub numero: String,
    base: PathBuf,
}

impl Ventoinha {
    fn ler(&self, sufixo: &str) -> Result<u64, String> {
        ler_u64(&PathBuf::from(format!("{}_{sufixo}", self.base.display())))
    }

    pub fn rpm(&self) -> Result<u64, String> {
        self.ler("input")
    }

    pub fn piso(&self) -> Result<u64, String> {
        self.ler("min")
    }

    pub fn maximo(&self) -> Result<u64, String> {
        self.ler("max")
    }

    pub fn definir_piso(&self, rpm: u64) -> Result<(), String> {
        escrever_u64(&PathBuf::from(format!("{}_min", self.base.display())), rpm)
    }
}

/// As ventoinhas do MacBook. Fora de um Mac, nenhuma, e o resto funciona.
pub fn ventoinhas(sysfs: &Path) -> Vec<Ventoinha> {
    let Some(pasta) = hwmon(sysfs, "applesmc") else {
        return Vec::new();
    };
    let mut lista: Vec<_> = fs::read_dir(&pasta)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| {
            let nome = e.file_name().into_string().ok()?;
            let numero = nome.strip_prefix("fan")?.strip_suffix("_max")?.to_string();
            Some(Ventoinha {
                base: pasta.join(format!("fan{numero}")),
                numero,
            })
        })
        .collect();
    lista.sort_by(|a, b| a.numero.cmp(&b.numero));
    lista
}

/// A frequência média dos núcleos, em MHz.
pub fn frequencia_mhz(sysfs: &Path) -> Option<f64> {
    let base = sysfs.join("devices/system/cpu");
    let khz: Vec<u64> = fs::read_dir(&base)
        .ok()?
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_str()
                .and_then(|n| n.strip_prefix("cpu"))
                .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .filter_map(|e| ler_u64(&e.path().join("cpufreq/scaling_cur_freq")).ok())
        .collect();
    (!khz.is_empty()).then(|| khz.iter().sum::<u64>() as f64 / khz.len() as f64 / 1000.0)
}

#[cfg(test)]
pub mod sysfs_de_teste {
    //! Um sysfs de mentira com o que o MacBookPro11,2 tem.
    use std::fs;
    use std::path::Path;

    pub fn montar(raiz: &Path, celsius: u32) {
        let escrever = |caminho: &str, texto: &str| {
            let p = raiz.join(caminho);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, texto).unwrap();
        };
        // hwmon0 é outro chip, para provar que a busca é pelo nome.
        escrever("class/hwmon/hwmon0/name", "acpitz\n");
        escrever("class/hwmon/hwmon3/name", "coretemp\n");
        escrever(
            "class/hwmon/hwmon3/temp1_input",
            &format!("{}\n", celsius * 1000),
        );
        escrever("class/hwmon/hwmon5/name", "applesmc\n");
        for (n, max, atual) in [("1", 6156, 2160), ("2", 5700, 2000)] {
            escrever(
                &format!("class/hwmon/hwmon5/fan{n}_max"),
                &format!("{max}\n"),
            );
            escrever(&format!("class/hwmon/hwmon5/fan{n}_min"), "2000\n");
            escrever(
                &format!("class/hwmon/hwmon5/fan{n}_input"),
                &format!("{atual}\n"),
            );
        }
        let rapl = "class/powercap/intel-rapl:0";
        escrever(
            &format!("{rapl}/constraint_0_power_limit_uw"),
            "100000000\n",
        );
        escrever(
            &format!("{rapl}/constraint_1_power_limit_uw"),
            "125000000\n",
        );
        escrever(&format!("{rapl}/energy_uj"), "1000000\n");
        escrever(&format!("{rapl}/max_energy_range_uj"), "262143328850\n");
        for cpu in 0..4 {
            escrever(
                &format!("devices/system/cpu/cpu{cpu}/cpufreq/scaling_cur_freq"),
                &format!("{}\n", 2_000_000 + cpu * 400_000),
            );
        }
        escrever(
            "devices/system/cpu/cpufreq/policy0/scaling_cur_freq",
            "9999999\n",
        );
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn acha_os_chips_pelo_nome() {
        let d = tempfile::tempdir().unwrap();
        sysfs_de_teste::montar(d.path(), 87);
        assert!((Sensor::achar(d.path()).unwrap().celsius().unwrap() - 87.0).abs() < f64::EPSILON);
        let v = ventoinhas(d.path());
        assert_eq!(
            v.iter().map(|v| v.numero.as_str()).collect::<Vec<_>>(),
            ["1", "2"]
        );
        assert_eq!(v[0].maximo().unwrap(), 6156);
    }

    #[test]
    fn grava_o_pl1_e_o_piso() {
        let d = tempfile::tempdir().unwrap();
        sysfs_de_teste::montar(d.path(), 60);
        let r = Rapl::achar(d.path()).unwrap();
        r.definir_pl1_uw(38_000_000).unwrap();
        assert_eq!(r.pl1_uw().unwrap(), 38_000_000);
        assert_eq!(r.pl2_uw().unwrap(), 125_000_000);
        let v = ventoinhas(d.path());
        v[1].definir_piso(5700).unwrap();
        assert_eq!(v[1].piso().unwrap(), 5700);
    }

    #[test]
    fn frequencia_e_a_media_dos_nucleos() {
        let d = tempfile::tempdir().unwrap();
        sysfs_de_teste::montar(d.path(), 60);
        // 2000, 2400, 2800, 3200 MHz; o `cpufreq/policy0` não é um núcleo.
        assert!((frequencia_mhz(d.path()).unwrap() - 2600.0).abs() < 0.001);
    }

    #[test]
    fn sem_applesmc_nao_ha_ventoinha_e_nada_quebra() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("class/hwmon")).unwrap();
        assert!(ventoinhas(d.path()).is_empty());
        assert!(Sensor::achar(d.path()).is_err());
    }

    #[test]
    fn energia_conta_a_volta_do_contador() {
        assert_eq!(delta_de_energia(100, 250, 1000), 150);
        assert_eq!(delta_de_energia(900, 50, 1000), 150);
    }
}

/// Os contadores de estrangulamento térmico do kernel: (pacote, soma dos
/// núcleos) desde o boot. Com o termostato bem ajustado, eles param de subir.
pub fn estrangulamento(sysfs: &Path) -> Option<(u64, u64)> {
    let base = sysfs.join("devices/system/cpu");
    let pacote = ler_u64(&base.join("cpu0/thermal_throttle/package_throttle_count")).ok()?;
    let nucleos = fs::read_dir(&base)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|e| ler_u64(&e.path().join("thermal_throttle/core_throttle_count")).ok())
        .sum();
    Some((pacote, nucleos))
}

/// Um domínio de potência do RAPL além do pacote: `core` (os núcleos),
/// `uncore` (o gráfico e o resto do chip) e `dram` (a memória).
pub struct Dominio {
    pub nome: String,
    energia: PathBuf,
    teto: u64,
    anterior: Option<(u64, std::time::Instant)>,
}

impl Dominio {
    pub fn potencia_w(&mut self) -> Option<f64> {
        let agora = std::time::Instant::now();
        let e = ler_u64(&self.energia).ok()?;
        let antes = self.anterior.replace((e, agora))?;
        let s = agora.duration_since(antes.1).as_secs_f64();
        (s > 0.0).then(|| delta_de_energia(antes.0, e, self.teto) as f64 / 1e6 / s)
    }
}

pub fn dominios(sysfs: &Path) -> Vec<Dominio> {
    let base = sysfs.join("class/powercap");
    let mut v: Vec<Dominio> = fs::read_dir(&base)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with("intel-rapl:0:"))
        })
        .filter_map(|e| {
            let nome = fs::read_to_string(e.path().join("name"))
                .ok()?
                .trim()
                .to_string();
            Some(Dominio {
                nome,
                energia: e.path().join("energy_uj"),
                teto: ler_u64(&e.path().join("max_energy_range_uj")).unwrap_or(u64::MAX),
                anterior: None,
            })
        })
        .collect();
    v.sort_by(|a, b| a.nome.cmp(&b.nome));
    v
}

/// Os sensores de temperatura do `applesmc` (núcleos, GPU, memória, chipset,
/// aletas, bateria, apoio de mãos): (nome da chave do SMC, °C). Os que dão
/// valor impossível (sensor ausente devolve −127 ou −48) ficam de fora.
pub fn sensores_do_mac(sysfs: &Path) -> Vec<(String, f64)> {
    let Some(pasta) = hwmon(sysfs, "applesmc") else {
        return Vec::new();
    };
    let mut v: Vec<(String, f64)> = fs::read_dir(&pasta)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|e| {
            let nome = e.file_name().into_string().ok()?;
            let base = nome
                .strip_suffix("_input")
                .filter(|b| b.starts_with("temp"))?
                .to_string();
            let celsius = ler_u64(&e.path()).ok()? as f64 / 1000.0;
            let rotulo = fs::read_to_string(pasta.join(format!("{base}_label")))
                .map_or(base, |l| l.trim().to_string());
            (1.0..=125.0)
                .contains(&celsius)
                .then_some((rotulo, celsius))
        })
        .collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

#[cfg(test)]
mod testes_do_kernel {
    use super::*;

    #[test]
    fn estrangulamento_soma_os_nucleos() {
        let d = tempfile::tempdir().unwrap();
        for (cpu, nucleo) in [(0, 10), (1, 5), (2, 7)] {
            let p = d
                .path()
                .join(format!("devices/system/cpu/cpu{cpu}/thermal_throttle"));
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("core_throttle_count"), format!("{nucleo}\n")).unwrap();
            fs::write(p.join("package_throttle_count"), "342193\n").unwrap();
        }
        assert_eq!(estrangulamento(d.path()), Some((342_193, 22)));
    }

    #[test]
    fn dominios_e_sensores_do_mac() {
        let d = tempfile::tempdir().unwrap();
        for (i, nome) in ["core", "uncore", "dram"].iter().enumerate() {
            let p = d.path().join(format!("class/powercap/intel-rapl:0:{i}"));
            fs::create_dir_all(&p).unwrap();
            fs::write(p.join("name"), format!("{nome}\n")).unwrap();
            fs::write(p.join("energy_uj"), "1000\n").unwrap();
        }
        let nomes: Vec<String> = dominios(d.path()).into_iter().map(|x| x.nome).collect();
        assert_eq!(nomes, ["core", "dram", "uncore"]);

        let h = d.path().join("class/hwmon/hwmon2");
        fs::create_dir_all(&h).unwrap();
        fs::write(h.join("name"), "applesmc\n").unwrap();
        for (n, rotulo, mili) in [(1, "TC0P", 41000), (2, "Ts0P", 30000), (3, "TH0c", 0)] {
            fs::write(h.join(format!("temp{n}_input")), format!("{mili}\n")).unwrap();
            fs::write(h.join(format!("temp{n}_label")), format!("{rotulo}\n")).unwrap();
        }
        let s = sensores_do_mac(d.path());
        assert_eq!(
            s,
            [("TC0P".to_string(), 41.0), ("Ts0P".to_string(), 30.0)],
            "o sensor ausente fica de fora"
        );
    }
}

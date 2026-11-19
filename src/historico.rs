//! O histórico: quem esquenta a máquina, aprendido observando.
//!
//! A cada leitura, os nomes que dominam a CPU recebem uma **observação** do
//! calor daquele momento, ponderada pela fatia de cada um, numa média móvel
//! exponencial (`nota_c_s`, em °C/s):
//!
//! - subindo mais que 0,3 °C/s: o valor é a própria tendência;
//! - parada mas quente (acima de `freio_comeca_c`): o processo está mantendo a
//!   máquina quente, e o valor é 0,1 °C/s por grau acima;
//! - fria e sem subir: o valor é zero, e a nota **cai**. É assim que um falso
//!   suspeito se corrige sozinho.
//!
//! Com `nota_c_s ≥ suspeito_c_s` e observações de calor suficientes, o nome
//! entra no **histórico negativo**: quando ele volta a gastar CPU, a curva de
//! freio pode frear antes de a temperatura reagir.
//!
//! O histórico fica em `<estado>/historico.json` e sobrevive a reinícios.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::processos::Uso;

/// Peso de cada observação na média móvel.
const ALFA: f64 = 0.3;
/// Tendência a partir da qual uma leitura conta como "esquentando".
const ESQUENTANDO_C_S: f64 = 0.3;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Ficha {
    pub nota_c_s: f64,
    /// Observações com calor (as frias só baixam a nota).
    pub observacoes: u32,
    pub maior_tendencia_c_s: f64,
    /// Segundos desde 1970 da última vez que gastou CPU.
    pub visto_em: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Historico {
    pub fichas: BTreeMap<String, Ficha>,
}

/// Um nome do histórico negativo gastando CPU agora.
#[derive(Debug, Clone, PartialEq)]
pub struct Suspeito {
    pub nome: String,
    pub nota_c_s: f64,
    pub fatia: f64,
    pub copias: u32,
}

pub fn arquivo(estado: &Path) -> PathBuf {
    estado.join("historico.json")
}

impl Historico {
    pub fn ler(estado: &Path) -> Self {
        fs::read_to_string(arquivo(estado))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn gravar(&self, estado: &Path) -> Result<(), String> {
        fs::create_dir_all(estado).map_err(|e| format!("{}: {e}", estado.display()))?;
        let texto = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let caminho = arquivo(estado);
        fs::write(&caminho, texto).map_err(|e| format!("{}: {e}", caminho.display()))
    }

    pub fn eh_suspeito(&self, nome: &str, cfg: &Config) -> bool {
        self.fichas.get(nome).is_some_and(|f| {
            f.nota_c_s >= cfg.processos.suspeito_c_s
                && f.observacoes >= cfg.processos.minimo_de_observacoes
        })
    }

    /// Uma leitura. Devolve os nomes que acabaram de entrar no histórico negativo.
    pub fn aprender(
        &mut self,
        usos: &[Uso],
        temperatura: f64,
        tendencia_c_s: f64,
        agora_unix: u64,
        cfg: &Config,
    ) -> Vec<String> {
        let p = &cfg.processos;
        let (valor, com_calor) = if tendencia_c_s > ESQUENTANDO_C_S {
            (tendencia_c_s, true)
        } else if temperatura > cfg.freio_comeca_c {
            ((temperatura - cfg.freio_comeca_c) * 0.1, true)
        } else {
            (0.0, false)
        };
        let mut novos = Vec::new();
        for u in usos
            .iter()
            .filter(|u| u.fatia >= p.fatia_minima && !p.ignorar.contains(&u.chave))
        {
            let era = self.eh_suspeito(&u.chave, cfg);
            let f = self.fichas.entry(u.chave.clone()).or_default();
            // A fatia pondera: quem tem 80 % da CPU leva quase todo o mérito;
            // quem tem 5 %, quase nada.
            let peso = ALFA * u.fatia.min(1.0).sqrt();
            f.nota_c_s += peso * (valor - f.nota_c_s);
            if com_calor {
                f.observacoes = f.observacoes.saturating_add(1);
                f.maior_tendencia_c_s = f.maior_tendencia_c_s.max(tendencia_c_s);
            }
            f.visto_em = agora_unix;
            if !era && self.eh_suspeito(&u.chave, cfg) {
                novos.push(u.chave.clone());
            }
        }
        novos
    }

    /// Os suspeitos gastando CPU agora, do mais quente ao menos.
    pub fn suspeitos_ativos(&self, usos: &[Uso], cfg: &Config) -> Vec<Suspeito> {
        let mut v: Vec<Suspeito> = usos
            .iter()
            .filter(|u| u.fatia >= cfg.processos.fatia_minima && self.eh_suspeito(&u.chave, cfg))
            .map(|u| Suspeito {
                nome: u.chave.clone(),
                nota_c_s: self.fichas[&u.chave].nota_c_s,
                fatia: u.fatia,
                copias: u.copias,
            })
            .collect();
        v.sort_by(|a, b| (b.nota_c_s * b.fatia).total_cmp(&(a.nota_c_s * a.fatia)));
        v
    }

    /// Tira quem não é visto há mais de `dias`.
    pub fn esquecer(&mut self, agora_unix: u64, dias: u64) {
        let limite = agora_unix.saturating_sub(dias * 86_400);
        self.fichas.retain(|_, f| f.visto_em >= limite);
    }
}

/// A temperatura prevista pelos processos: quanto os suspeitos ativos costumam
/// subir, na proporção da CPU que ocupam juntos, `previsao_s` à frente.
pub fn prevista_por_processos(
    temperatura: f64,
    suspeitos: &[Suspeito],
    cfg: &Config,
) -> Option<f64> {
    if suspeitos.is_empty() {
        return None;
    }
    let fatia: f64 = suspeitos.iter().map(|s| s.fatia).sum::<f64>().min(1.0);
    let nota = suspeitos.iter().map(|s| s.nota_c_s).fold(0.0, f64::max);
    Some(temperatura + nota * fatia.sqrt() * cfg.previsao_s)
}

#[cfg(test)]
mod testes {
    use super::*;

    fn uso(nome: &str, fatia: f64) -> Uso {
        Uso {
            chave: nome.into(),
            fatia,
            copias: 8,
            rss_mb: 0.0,
        }
    }

    #[test]
    fn rustc_esquentando_vira_suspeito_em_tres_observacoes() {
        let c = Config::default();
        let mut h = Historico::default();
        let mut novos = Vec::new();
        for i in 0..6 {
            novos.extend(h.aprender(&[uso("rustc", 0.9)], 75.0, 2.0, 1000 + i, &c));
            if !novos.is_empty() {
                assert!(h.fichas["rustc"].observacoes >= 3);
                break;
            }
        }
        assert_eq!(novos, ["rustc"]);
        assert!(h.fichas["rustc"].nota_c_s >= 1.0);
    }

    #[test]
    fn quem_gasta_pouco_nao_leva_a_culpa() {
        let c = Config::default();
        let mut h = Historico::default();
        for i in 0..10 {
            h.aprender(
                &[uso("rustc", 0.9), uso("gnome-shell", 0.02)],
                75.0,
                2.0,
                i,
                &c,
            );
        }
        assert!(
            !h.fichas.contains_key("gnome-shell"),
            "abaixo da fatia mínima nem entra"
        );
    }

    #[test]
    fn falso_suspeito_se_corrige() {
        let c = Config::default();
        let mut h = Historico::default();
        for i in 0..6 {
            h.aprender(&[uso("java", 0.6)], 75.0, 2.0, i, &c);
        }
        assert!(h.eh_suspeito("java", &c));
        // Depois roda muito sem esquentar: a nota cai abaixo do limiar.
        for i in 0..20 {
            h.aprender(&[uso("java", 0.6)], 60.0, 0.0, 100 + i, &c);
        }
        assert!(
            !h.eh_suspeito("java", &c),
            "nota {}",
            h.fichas["java"].nota_c_s
        );
    }

    #[test]
    fn quente_e_parado_tambem_conta() {
        let c = Config::default();
        let mut h = Historico::default();
        for i in 0..10 {
            h.aprender(&[uso("qemu-system-x86", 0.5)], 92.0, 0.0, i, &c);
        }
        assert!(h.fichas["qemu-system-x86"].nota_c_s > 0.5);
    }

    #[test]
    fn previsao_por_processos_antes_de_esquentar() {
        let c = Config::default();
        let mut h = Historico::default();
        // +4,2 °C/s: o que o rustc fez na build do VintageLightbox em 19/11/2026.
        for i in 0..6 {
            h.aprender(&[uso("rustc", 0.9)], 75.0, 4.2, i, &c);
        }
        // A máquina está fria e parada; o rustc acaba de aparecer com 80 %.
        let s = h.suspeitos_ativos(&[uso("rustc", 0.8)], &c);
        assert_eq!(s.len(), 1);
        let prevista = prevista_por_processos(60.0, &s, &c).unwrap();
        assert!(
            prevista > c.freio_comeca_c,
            "prevê calor antes de ele chegar: {prevista}"
        );
        assert_eq!(prevista_por_processos(60.0, &[], &c), None);
    }

    #[test]
    fn quem_esquenta_pouco_nao_faz_frear_a_maquina_fria() {
        let c = Config::default();
        let mut h = Historico::default();
        for i in 0..6 {
            h.aprender(&[uso("java", 0.9)], 75.0, 2.0, i, &c);
        }
        let s = h.suspeitos_ativos(&[uso("java", 0.8)], &c);
        let prevista = prevista_por_processos(60.0, &s, &c).unwrap();
        assert!(
            prevista < c.freio_comeca_c,
            "a 60 °C, +2 °C/s não chega a 80 °C: {prevista}"
        );
    }

    #[test]
    fn grava_rele_e_esquece() {
        let d = tempfile::tempdir().unwrap();
        let c = Config::default();
        let mut h = Historico::default();
        h.aprender(&[uso("rustc", 0.9)], 75.0, 2.0, 100 * 86_400, &c);
        h.aprender(&[uso("cc", 0.9)], 75.0, 2.0, 10 * 86_400, &c);
        h.gravar(d.path()).unwrap();
        let mut lido = Historico::ler(d.path());
        assert_eq!(lido, h);
        lido.esquecer(100 * 86_400, 30);
        assert!(lido.fichas.contains_key("rustc"));
        assert!(!lido.fichas.contains_key("cc"), "visto há 90 dias");
    }
}

//! Ajustes com o serviço rodando: `sudo termostato chave=valor …`.
//!
//! Qualquer parâmetro do `/etc/termostato.toml` pode ser trocado na hora, sem
//! reiniciar e sem editar o arquivo do pacote. Os ajustes moram num arquivo do
//! estado (`/var/lib/termostato/ajustes.toml`, só o root grava), que o serviço
//! relê a cada volta e põe por cima do toml. Sobrevivem a um reinício e somem
//! com `termostato padrao` ou `--desinstalar`.
//!
//!   sudo termostato CPU50C                    # atalho de alvo_c=50
//!   sudo termostato pl1_normal_w=30 previsao_s=15 maos.modo=agir
//!   sudo termostato pl1_normal_w=padrao       # tira um ajuste
//!   sudo termostato CPULIVRE                  # sem alvo (alvo_c=0)
//!   sudo termostato padrao                    # tira todos
//!   termostato ajustes                        # mostra o que está em vigor
//!
//! Um ajuste é conferido antes de gravar, com as mesmas regras do toml: o que
//! deixaria a máquina sem proteção é recusado e nada é gravado.

use std::fs;
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use crate::config::Config;

/// A faixa do alvo. Abaixo de 40 °C a fedora não chega nem parada; acima de
/// 95 °C o chip já se freia sozinho (TCC em 92 °C).
pub const ALVO_MINIMO_C: f64 = 40.0;
pub const ALVO_MAXIMO_C: f64 = 95.0;

/// Parâmetros lidos uma vez só, na subida: trocar exige reiniciar o serviço.
pub const SO_NA_SUBIDA: &[&str] = &[
    "escuta",
    "sysfs",
    "estado",
    "proc",
    "docker_socket",
    "dev_cpu",
    "psi_parado_ms",
    "psi_janela_ms",
    "tcc_offset_c",
    "grafana",
];

/// O que a linha de comando pediu.
#[derive(Debug, Clone, PartialEq)]
pub enum Pedido {
    /// Cada chave com o valor novo, ou `None` para voltar ao do toml.
    Ajustar(Vec<(String, Option<Value>)>),
    Mostrar,
    Limpar,
}

/// `chave=valor`, `CPU50C`, `CPULIVRE`, `ajustes` ou `padrao`. Um argumento que
/// não é nada disso (o caminho da config) → `None`: é o serviço.
pub fn interpretar(args: &[String]) -> Option<Result<Pedido, String>> {
    let primeiro = args.first()?;
    match primeiro.as_str() {
        "ajustes" => return Some(Ok(Pedido::Mostrar)),
        "padrao" | "padrão" => return Some(Ok(Pedido::Limpar)),
        _ => {}
    }
    if !eh_ajuste(primeiro) {
        return None;
    }
    let mut pares = Vec::new();
    for a in args {
        match um_ajuste(a) {
            Ok(p) => pares.push(p),
            Err(e) => return Some(Err(e)),
        }
    }
    Some(Ok(Pedido::Ajustar(pares)))
}

fn eh_ajuste(arg: &str) -> bool {
    arg.contains('=')
        || (arg.to_uppercase().starts_with("CPU")
            && !arg.contains('/')
            && !arg.to_lowercase().ends_with(".toml"))
}

/// `CPU50C`, `cpu50`, `CPU62.5C`, `CPU62,5°C` → °C; `CPULIVRE` → 0.
fn cpu(arg: &str) -> Option<Result<f64, String>> {
    let maiusculo = arg.trim().to_uppercase();
    let resto = maiusculo.strip_prefix("CPU")?;
    if resto == "LIVRE" {
        return Some(Ok(0.0));
    }
    let numero = resto
        .trim_end_matches('C')
        .trim_end_matches('°')
        .replace(',', ".");
    Some(
        numero
            .parse::<f64>()
            .map_err(|_| format!("não entendi \"{arg}\": use CPU50C (alvo em °C) ou CPULIVRE")),
    )
}

fn um_ajuste(arg: &str) -> Result<(String, Option<Value>), String> {
    if let Some(c) = cpu(arg) {
        return Ok(("alvo_c".into(), Some(Value::Float(c?))));
    }
    let (chave, texto) = arg
        .split_once('=')
        .ok_or_else(|| format!("\"{arg}\" não é chave=valor"))?;
    let chave = chave.trim();
    if chave.is_empty() || chave.split('.').any(str::is_empty) {
        return Err(format!("\"{arg}\": chave vazia"));
    }
    let raiz = chave.split('.').next().unwrap_or(chave);
    if SO_NA_SUBIDA.contains(&raiz) {
        return Err(format!(
            "{chave} só vale na subida: mude no /etc/termostato.toml e reinicie o serviço"
        ));
    }
    let texto = texto.trim();
    if matches!(texto, "padrao" | "padrão") {
        return Ok((chave.into(), None));
    }
    Ok((chave.into(), Some(valor(texto))))
}

/// O texto como valor TOML (`38`, `2.5`, `true`, `["a", "b"]`); se não for, texto.
fn valor(texto: &str) -> Value {
    toml::from_str::<Table>(&format!("v = {texto}"))
        .ok()
        .and_then(|mut t| t.remove("v"))
        .unwrap_or_else(|| Value::String(texto.trim_matches('"').into()))
}

pub fn arquivo(estado: &Path) -> PathBuf {
    estado.join("ajustes.toml")
}

/// O texto do arquivo, como está (para saber se mudou); sem arquivo, vazio.
pub fn texto(estado: &Path) -> String {
    fs::read_to_string(arquivo(estado)).unwrap_or_default()
}

pub fn tabela(texto: &str) -> Result<Table, String> {
    toml::from_str(texto).map_err(|e| format!("ajustes.toml estragado: {e}"))
}

/// Põe (ou tira) um valor no caminho `a.b.c`; tira as tabelas que ficam vazias.
pub fn aplicar(t: &mut Table, chave: &str, v: Option<Value>) {
    match chave.split_once('.') {
        None => {
            match v {
                Some(v) => t.insert(chave.into(), v),
                None => t.remove(chave),
            };
        }
        Some((cabeca, resto)) => {
            let sub = t
                .entry(cabeca)
                .or_insert_with(|| Value::Table(Table::new()));
            if !sub.is_table() {
                *sub = Value::Table(Table::new());
            }
            if let Value::Table(s) = sub {
                aplicar(s, resto, v);
                if s.is_empty() {
                    t.remove(cabeca);
                }
            }
        }
    }
}

/// `por_cima` sobre `base`, descendo nas tabelas.
pub fn mesclar(base: &mut Table, por_cima: &Table) {
    for (k, v) in por_cima {
        match (base.get_mut(k), v) {
            (Some(Value::Table(b)), Value::Table(c)) => mesclar(b, c),
            _ => {
                base.insert(k.clone(), v.clone());
            }
        }
    }
}

/// O toml do pacote com os ajustes por cima, conferido.
pub fn config_com(caminho: &Path, ajustes: &Table) -> Result<Config, String> {
    let mut base: Table = match fs::read_to_string(caminho) {
        Ok(t) => toml::from_str(&t)
            .map_err(|e| format!("{} não é uma configuração válida: {e}", caminho.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Table::new(),
        Err(e) => return Err(format!("não li {}: {e}", caminho.display())),
    };
    mesclar(&mut base, ajustes);
    let cfg: Config = Value::Table(base)
        .try_into()
        .map_err(|e: toml::de::Error| format!("ajuste inválido: {}", e.message()))?;
    cfg.validar()?;
    Ok(cfg)
}

/// `chave = valor` de cada folha, em ordem: para mostrar e comparar.
pub fn achatar(t: &Table) -> Vec<(String, String)> {
    fn descer(prefixo: &str, t: &Table, saida: &mut Vec<(String, String)>) {
        for (k, v) in t {
            let chave = if prefixo.is_empty() {
                k.clone()
            } else {
                format!("{prefixo}.{k}")
            };
            match v {
                Value::Table(s) => descer(&chave, s, saida),
                outro => saida.push((chave, outro.to_string())),
            }
        }
    }
    let mut saida = Vec::new();
    descer("", t, &mut saida);
    saida
}

/// O que mudou entre dois conjuntos de ajustes, em linhas curtas.
pub fn diferencas(antes: &Table, depois: &Table) -> Vec<String> {
    let a = achatar(antes);
    let d = achatar(depois);
    let mut linhas = Vec::new();
    for (k, v) in &d {
        match a.iter().find(|(ka, _)| ka == k) {
            Some((_, va)) if va == v => {}
            Some((_, va)) => linhas.push(format!("{k} {va} → {v}")),
            None => linhas.push(format!("{k} = {v}")),
        }
    }
    for (k, _) in &a {
        if !d.iter().any(|(kd, _)| kd == k) {
            linhas.push(format!("{k} volta ao padrão"));
        }
    }
    linhas
}

/// Grava de uma vez (temporário e `rename`): o serviço nunca lê pela metade.
/// Sem ajuste nenhum, apaga o arquivo.
pub fn gravar(estado: &Path, t: &Table) -> Result<(), String> {
    let destino = arquivo(estado);
    if t.is_empty() {
        return apagar(estado);
    }
    let texto = toml::to_string(t).map_err(|e| e.to_string())?;
    let temporario = estado.join(".ajustes.novo");
    fs::write(&temporario, texto)
        .and_then(|()| fs::rename(&temporario, &destino))
        .map_err(|e| {
            let _ = fs::remove_file(&temporario);
            if e.kind() == std::io::ErrorKind::PermissionDenied {
                format!("{}: sem permissão, rode com sudo", destino.display())
            } else {
                format!("{}: {e}", destino.display())
            }
        })
}

pub fn apagar(estado: &Path) -> Result<(), String> {
    let caminho = arquivo(estado);
    match fs::remove_file(&caminho) {
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Err(format!(
            "{}: sem permissão, rode com sudo",
            caminho.display()
        )),
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(format!("{}: {e}", caminho.display()))
        }
        _ => Ok(()),
    }
}

/// Copia para `novo` os parâmetros que só valem na subida, e diz quais o
/// arquivo tentou mudar (alguém o editou à mão).
pub fn manter_os_da_subida(inicial: &Config, novo: &mut Config) -> Vec<&'static str> {
    let mut ignorados = Vec::new();
    macro_rules! manter {
        ($($campo:ident),*) => {$(
            if novo.$campo != inicial.$campo {
                ignorados.push(stringify!($campo));
                novo.$campo = inicial.$campo.clone();
            }
        )*};
    }
    manter!(
        escuta,
        sysfs,
        estado,
        proc,
        docker_socket,
        dev_cpu,
        psi_parado_ms,
        psi_janela_ms,
        tcc_offset_c,
        grafana
    );
    ignorados
}

#[cfg(test)]
mod testes {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_string()).collect()
    }

    fn pares(a: &[&str]) -> Vec<(String, Option<Value>)> {
        match interpretar(&args(a)) {
            Some(Ok(Pedido::Ajustar(p))) => p,
            outro => panic!("{outro:?}"),
        }
    }

    #[test]
    fn cpu50c_e_atalho_do_alvo() {
        for a in ["CPU50C", "cpu50c", "CPU50", "CPU50°C"] {
            assert_eq!(
                pares(&[a]),
                vec![("alvo_c".into(), Some(Value::Float(50.0)))]
            );
        }
        assert_eq!(
            pares(&["CPU62,5C"]),
            vec![("alvo_c".into(), Some(Value::Float(62.5)))]
        );
        assert_eq!(
            pares(&["CPULIVRE"]),
            vec![("alvo_c".into(), Some(Value::Float(0.0)))]
        );
    }

    #[test]
    fn varios_parametros_de_uma_vez() {
        let p = pares(&[
            "pl1_normal_w=30",
            "maos.modo=agir",
            "CPU55C",
            "previsao_s=padrao",
        ]);
        assert_eq!(
            p,
            vec![
                ("pl1_normal_w".into(), Some(Value::Integer(30))),
                ("maos.modo".into(), Some(Value::String("agir".into()))),
                ("alvo_c".into(), Some(Value::Float(55.0))),
                ("previsao_s".into(), None),
            ]
        );
    }

    #[test]
    fn caminho_de_config_e_o_servico() {
        assert_eq!(interpretar(&args(&["/etc/termostato.toml"])), None);
        assert_eq!(interpretar(&args(&["cpu.toml"])), None);
        assert_eq!(interpretar(&args(&[])), None);
        assert_eq!(interpretar(&args(&["ajustes"])), Some(Ok(Pedido::Mostrar)));
        assert_eq!(interpretar(&args(&["padrao"])), Some(Ok(Pedido::Limpar)));
    }

    #[test]
    fn recusa_o_que_so_vale_na_subida() {
        assert!(matches!(
            interpretar(&args(&["escuta=0.0.0.0:9101"])),
            Some(Err(_))
        ));
        assert!(matches!(
            interpretar(&args(&["grafana.url=x"])),
            Some(Err(_))
        ));
        assert!(matches!(interpretar(&args(&["CPUQUENTE"])), Some(Err(_))));
    }

    #[test]
    fn mescla_confere_e_recusa() {
        let d = tempfile::tempdir().unwrap();
        let base = d.path().join("termostato.toml");
        fs::write(&base, "pl1_normal_w = 38\n[maos]\nmodo = \"observar\"\n").unwrap();
        let mut t = Table::new();
        for (k, v) in pares(&["pl1_normal_w=30", "maos.modo=agir", "CPU50C"]) {
            aplicar(&mut t, &k, v);
        }
        let cfg = config_com(&base, &t).unwrap();
        assert!((cfg.pl1_normal_w - 30.0).abs() < 1e-9);
        assert_eq!(cfg.maos.modo, crate::config::Modo::Agir);
        assert!((cfg.alvo_c - 50.0).abs() < 1e-9);

        // Fora da faixa ou chave que não existe: recusado.
        let mut ruim = t.clone();
        aplicar(&mut ruim, "alvo_c", Some(Value::Float(30.0)));
        assert!(config_com(&base, &ruim).is_err());
        let mut ruim = t.clone();
        aplicar(&mut ruim, "pl1_normla_w", Some(Value::Integer(30)));
        assert!(config_com(&base, &ruim).is_err());
        let mut ruim = t.clone();
        aplicar(&mut ruim, "pl1_normal_w", Some(Value::Integer(500)));
        assert!(config_com(&base, &ruim).is_err());

        // Tirar um ajuste devolve o do toml e some com a tabela vazia.
        aplicar(&mut t, "maos.modo", None);
        assert!(!t.contains_key("maos"));
        let cfg = config_com(&base, &t).unwrap();
        assert_eq!(cfg.maos.modo, crate::config::Modo::Observar);
    }

    #[test]
    fn grava_le_mostra_diferencas_e_apaga() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(texto(d.path()), "");
        let mut t = Table::new();
        aplicar(&mut t, "alvo_c", Some(Value::Float(50.0)));
        gravar(d.path(), &t).unwrap();
        let lido = tabela(&texto(d.path())).unwrap();
        assert_eq!(lido, t);

        let mut depois = t.clone();
        aplicar(&mut depois, "alvo_c", Some(Value::Float(60.0)));
        aplicar(&mut depois, "maos.modo", Some(Value::String("agir".into())));
        let dif = diferencas(&t, &depois);
        assert_eq!(dif, vec!["alvo_c 50.0 → 60.0", "maos.modo = \"agir\""]);
        assert_eq!(diferencas(&depois, &Table::new()).len(), 2);

        gravar(d.path(), &Table::new()).unwrap();
        assert!(!arquivo(d.path()).exists());
        apagar(d.path()).unwrap();
    }

    #[test]
    fn os_da_subida_ficam() {
        let inicial = Config::default();
        let mut novo = Config {
            escuta: "0.0.0.0:1".into(),
            pl1_normal_w: 30.0,
            ..Config::default()
        };
        assert_eq!(manter_os_da_subida(&inicial, &mut novo), vec!["escuta"]);
        assert_eq!(novo.escuta, inicial.escuta);
        assert!((novo.pl1_normal_w - 30.0).abs() < 1e-9);
    }
}

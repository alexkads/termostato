//! O painel do termostato, em RavenRS: o desenho aprovado em
//! <https://claude.ai/artifact/WZ8myAZ9MhV63rVUQxrSiR> ("Painel ao vivo"), feito
//! só com os componentes do design system do RCB.
//!
//! O app não fala HTTP: a página (`web/index.html`) busca `GET /api/estado` a
//! cada segundo e entrega o JSON em [`receber`]; cada ajuste volta pela função
//! `window.termostatoAjustar`, que faz o `POST /api/ajustes`.

pub mod modelo;

use std::rc::Rc;

use ravenrs::prelude::*;
use serde_json::{Value, json};

use modelo::{Estado, ha, milhar, nome_do_sensor, num};

/// Quem manda um ajuste ao termostato (o corpo do `POST /api/ajustes`).
pub type Ajustar = Rc<dyn Fn(Value)>;

/// A largura máxima da página.
const LARGURA: f32 = 1440.0;
const VAO: f32 = 16.0;
const ALVOS: [f64; 5] = [0.0, 55.0, 60.0, 65.0, 70.0];
const MAOS: [&str; 3] = ["observar", "agir", "desligado"];

/// A página inteira. `estado` é `None` até o primeiro JSON chegar.
///
/// As colunas seguem a largura da janela: os quatro números em 4, 2 ou 1; a
/// temperatura e os ajustes lado a lado de 1000 px para cima; os três gráficos em
/// 3 ou 1; os pares em 2 ou 1. Cada cartão recebe a largura exata da coluna, para
/// as bordas alinharem em qualquer tamanho.
pub fn app(estado: State<Option<Estado>>, ajustar: Ajustar) -> impl View {
    Localized::new(
        Labels::pt_br(),
        responsive(move |tela: ravenrs::core::Size| {
            let livre = (tela.width - 48.0).max(300.0);
            let w = livre.min(LARGURA);
            let largo = w >= 1000.0;
            let par = w >= 900.0;
            let tres = if largo { (w - 2.0 * VAO) / 3.0 } else { w };
            let meio = if par { (w - VAO) / 2.0 } else { w };
            let e = || estado.clone();
            let pagina = VStack::new((
                cabecalho(e(), w),
                agora(e(), w),
                HStack::new((
                    temperatura(e(), if largo { (w - VAO) * 0.64 } else { w }),
                    ajustes(
                        e(),
                        ajustar.clone(),
                        if largo { (w - VAO) * 0.36 } else { w },
                    ),
                ))
                .spacing(VAO)
                .align(Align::Stretch)
                .wrap(),
                fluxo(e(), w),
                HStack::new((
                    potencia(e(), tres),
                    ventoinhas(e(), tres),
                    quem_gasta(e(), tres),
                ))
                .spacing(VAO)
                .align(Align::Stretch)
                .wrap(),
                HStack::new((freio_por_hora(e(), meio), sensores(e(), meio)))
                    .spacing(VAO)
                    .align(Align::Stretch)
                    .wrap(),
                HStack::new((
                    processos(e(), if par { (w - VAO) * 0.62 } else { w }),
                    feitos(e(), if par { (w - VAO) * 0.38 } else { w }),
                ))
                .spacing(VAO)
                .align(Align::Stretch)
                .wrap(),
                historico(e(), w),
                rodape(e(), w),
            ))
            .spacing(24.0)
            .width(w);
            ScrollView::new(vec![Box::new(
                HStack::new((Spacer::new(), pagina, Spacer::new())).width(livre),
            ) as AnyView])
            .padding(16.0)
        }),
    )
}

fn lido(estado: &State<Option<Estado>>) -> Estado {
    estado.get().unwrap_or_default()
}

/// Um cartão do painel: título, uma linha de descrição e o conteúdo.
fn cartao(titulo: &str, descricao: &str, largura: f32, conteudo: impl View + 'static) -> impl View {
    Card::new(
        VStack::new((
            VStack::new((
                Text::new(titulo).style(TextStyle::Headline),
                Text::new(descricao)
                    .style(TextStyle::Callout)
                    .foreground(Semantic::InkMuted),
            ))
            .spacing(4.0),
            conteudo,
        ))
        .spacing(16.0)
        .width(largura - 48.0),
    )
}

fn cabecalho(estado: State<Option<Estado>>, w: f32) -> impl View {
    view_fn(move || {
        let e = lido(&estado);
        let selo = match e.decisao.vence.as_str() {
            "alvo" => Badge::new(format!(
                "Freando pelo alvo de {} °C",
                num(e.ajustes.alvo_c, 0)
            ))
            .tone(Tone::Warning),
            "curva" => Badge::new("Freando pela curva").tone(Tone::Warning),
            "antecipado" => Badge::new("Freio antecipado").tone(Tone::Warning),
            _ if estado.get().is_none() => Badge::new("Lendo…"),
            _ => Badge::new("Limite normal").tone(Tone::Success),
        };
        HStack::new((
            VStack::new((
                HStack::new((
                    Text::new("Termostato").style(TextStyle::Title),
                    Badge::new(if e.versao.is_empty() {
                        "—".into()
                    } else {
                        e.versao.clone()
                    }),
                ))
                .spacing(12.0)
                .align(Align::Center),
                Text::new(format!(
                    "{} · Intel i7-4770HQ · Fedora",
                    if e.maquina.is_empty() {
                        "fedora"
                    } else {
                        &e.maquina
                    }
                ))
                .style(TextStyle::Callout)
                .foreground(Semantic::InkMuted),
            ))
            .spacing(6.0),
            Spacer::new(),
            HStack::new((selo, ThemeSwitcher::new()))
                .spacing(12.0)
                .align(Align::Center),
        ))
        .align(Align::Center)
        .width(w)
    })
}

/// Os quatro números de agora, cada um com a sua tendência.
fn agora(estado: State<Option<Estado>>, w: f32) -> impl View {
    view_fn(move || {
        let e = lido(&estado);
        let a = &e.agora;
        let ultimos = |f: fn(&modelo::Amostra) -> f64| -> Vec<f64> {
            let n = e.serie.len().saturating_sub(24);
            e.serie[n..].iter().map(f).collect()
        };
        let n = if w >= 1000.0 {
            4.0
        } else if w >= 560.0 {
            2.0
        } else {
            1.0
        };
        let largura = (w - (n - 1.0) * VAO) / n;
        let numero =
            |rotulo: &str, valor: String, delta: String, sobe: bool, bom: bool, serie: Vec<f64>| {
                Card::new(
                    VStack::new((
                        Stat::new(rotulo, valor).delta_toned(delta, sobe, bom),
                        Chart::new(ChartKind::SparkArea)
                            .series(rotulo, serie)
                            .height(40.0)
                            .hide_tooltip()
                            .no_appear(),
                    ))
                    .spacing(8.0)
                    .width(largura - 48.0),
                )
            };
        let tendencia = a.tendencia.unwrap_or(0.0);
        let curva = e.decisao.freio_comeca_c;
        HStack::new((
            numero(
                "CPU agora",
                a.temperatura
                    .map_or("—".into(), |t| format!("{} °C", num(t, 0))),
                format!(
                    "{}{} °C/s",
                    if tendencia >= 0.0 { "+" } else { "" },
                    num(tendencia, 1)
                ),
                tendencia >= 0.0,
                tendencia <= 0.0,
                ultimos(|s| s.temperatura),
            ),
            numero(
                "Prevista em 10 s",
                a.prevista
                    .map_or("—".into(), |t| format!("{} °C", num(t, 0))),
                format!("a curva começa a {} °C", num(curva, 0)),
                true,
                a.prevista.is_none_or(|p| p < curva),
                ultimos(|s| s.temperatura),
            ),
            numero(
                "Limite PL1",
                a.pl1.map_or("—".into(), |w| format!("{} W", num(w, 1))),
                format!("normal {} W", num(a.pl1_normal, 0)),
                false,
                !a.freio_acionado,
                ultimos(|s| s.pl1),
            ),
            numero(
                "Potência",
                a.potencia
                    .map_or("—".into(), |w| format!("{} W", num(w, 1))),
                a.frequencia_mhz
                    .map_or("—".into(), |f| format!("{} MHz", milhar(f))),
                true,
                true,
                ultimos(|s| s.potencia),
            ),
        ))
        .spacing(VAO)
        .align(Align::Stretch)
        .wrap()
    })
}

fn temperatura(estado: State<Option<Estado>>, largura: f32) -> impl View {
    cartao(
        "Temperatura",
        "Últimos 10 minutos, uma leitura a cada 10 s",
        largura,
        view_fn(move || {
            let e = lido(&estado);
            let mut grafico = Chart::new(ChartKind::Area)
                .categories(e.serie.iter().map(|s| ha(e.agora_unix, s.t)))
                .series("Temperatura", e.serie.iter().map(|s| s.temperatura))
                .value_domain(40.0, 100.0)
                .hide_category_axis()
                .show_value_axis()
                .gradient()
                .height(230.0)
                .no_appear()
                .format(|v| format!("{} °C", num(v, 0)));
            if let Some(corte) = e.agora.tcc_corte {
                grafico = grafico.reference(
                    ChartReference::new(corte)
                        .label(format!("o chip se freia a {} °C", num(corte, 0)))
                        .color(Semantic::Danger),
                );
            }
            if e.ajustes.alvo_c > 0.0 {
                grafico = grafico.reference(
                    ChartReference::new(e.ajustes.alvo_c)
                        .label(format!("alvo {} °C", num(e.ajustes.alvo_c, 0)))
                        .color(Semantic::Guide),
                );
            }
            grafico
        }),
    )
}

fn ajustes(estado: State<Option<Estado>>, ajustar: Ajustar, largura: f32) -> impl View {
    let (e1, e2, e3, e4) = (
        estado.clone(),
        estado.clone(),
        estado.clone(),
        estado.clone(),
    );
    let (a1, a2, a3, a4) = (ajustar.clone(), ajustar.clone(), ajustar.clone(), ajustar);
    let alvo = Binding::new(
        move || {
            let atual = lido(&e1).ajustes.alvo_c;
            ALVOS
                .iter()
                .position(|a| (a - atual).abs() < 0.5)
                .unwrap_or(0)
        },
        move |i: usize| {
            let valor = ALVOS.get(i).copied().unwrap_or(0.0);
            a1(json!({"ajustes": {"alvo_c": valor}}));
        },
    );
    let maos = Binding::new(
        move || {
            let atual = lido(&e2).ajustes.maos_modo;
            MAOS.iter().position(|m| *m == atual).unwrap_or(0)
        },
        move |i: usize| {
            a2(json!({"ajustes": {"maos.modo": MAOS.get(i).copied().unwrap_or("observar")}}))
        },
    );
    let normal = Binding::new(
        move || lido(&e3).ajustes.pl1_normal_w.max(25.0) as f32,
        move |w: f32| a3(json!({"ajustes": {"pl1_normal_w": w.round()}})),
    );
    cartao(
        "Ajustes",
        "Valem em até 2 s, sem reiniciar, e ficam no journal e no Grafana.",
        largura,
        VStack::new((
            VStack::new((
                SegmentedControl::new(alvo, ["Sem alvo", "55 °C", "60 °C", "65 °C", "70 °C"])
                    .label("Alvo de temperatura"),
                Text::new(
                    "Baixa o PL1 até 8 W se precisar. Com carga, menos de 55 °C não se alcança.",
                )
                .style(TextStyle::Callout)
                .foreground(Semantic::InkMuted),
            ))
            .spacing(8.0),
            VStack::new((
                SegmentedControl::new(maos, ["Observar", "Agir", "Desligado"]).label("Mãos"),
                view_fn(move || {
                    Text::new(match lido(&e4).ajustes.maos_modo.as_str() {
                        "agir" => "Segura de verdade quem abusa, e devolve com a calma.",
                        "desligado" => "Não olha os processos.",
                        _ => "Só mostra o que faria, sem mexer nos processos.",
                    })
                    .style(TextStyle::Callout)
                    .foreground(Semantic::InkMuted)
                }),
            ))
            .spacing(8.0),
            VStack::new((
                Text::new("Limite normal (W)").style(TextStyle::Callout),
                Slider::new(normal)
                    .range(25.0, 45.0)
                    .step(1.0)
                    .label("Limite normal (W)")
                    .show_value(),
            ))
            .spacing(8.0)
            .align(Align::Stretch),
            HStack::new((
                Button::new("Voltar ao padrão")
                    .style(ButtonStyle::Ghost)
                    .on_click(move || a4(json!({"padrao": true}))),
                Spacer::new(),
                view_fn(move || {
                    if lido(&estado).ajustes.pode_ajustar {
                        Badge::new("Você pode ajustar").tone(Tone::Success)
                    } else {
                        Badge::new("Só leitura: abra pela tailnet").tone(Tone::Neutral)
                    }
                }),
            ))
            .align(Align::Center),
        ))
        .spacing(24.0)
        .align(Align::Stretch),
    )
}

/// O fluxo da decisão da última volta: o caminho que venceu, em execução.
fn grafo(e: &Estado) -> FlowGraph {
    let d = &e.decisao;
    let a = &e.agora;
    let vence = d.vence.as_str();
    let temp = a
        .temperatura
        .map_or("—".into(), |t| format!("{} °C agora", num(t, 0)));
    let prev = a.prevista.map_or("—".into(), |p| {
        format!(
            "{}{} °C/s → {} °C",
            if a.tendencia.unwrap_or(0.0) >= 0.0 {
                "+"
            } else {
                ""
            },
            num(a.tendencia.unwrap_or(0.0), 1),
            num(p, 0)
        )
    });
    let curva = format!("pede {} W", num(d.curva_w, 1));
    let alvo = match d.alvo_w {
        Some(w) if e.ajustes.alvo_c > 0.0 => {
            format!("{} °C → {} W", num(e.ajustes.alvo_c, 0), num(w, 1))
        }
        _ => "sem alvo".into(),
    };
    let antecipa = match (&d.suspeitos_ativos[..], d.prevista_processos) {
        ([], _) => "nenhum suspeito ativo".into(),
        ([um, ..], Some(p)) => format!("{um} → {} °C", num(p, 0)),
        ([um, ..], None) => format!("{um} trabalhando"),
    };
    let processos = e.processos.first().map_or("—".into(), |p| {
        format!("{} {} %", p.nome, num(p.fatia * 100.0, 0))
    });
    let psi = a.psi_memoria.map_or("—".into(), |p| {
        format!("{} % · gatilho armado", num(p, 0))
    });
    let pl1 = a
        .pl1
        .map_or("—".into(), |w| format!("MSR 0x610 ← {} W", num(w, 1)));
    let menor = match vence {
        "alvo" => format!("{} W, pelo alvo", num(a.pl1_alvo, 1)),
        "antecipado" => format!("{} W, antecipado", num(a.pl1_alvo, 1)),
        "curva" => format!("{} W, pela curva", num(a.pl1_alvo, 1)),
        _ => format!("{} W, o normal", num(a.pl1_alvo, 1)),
    };
    let maos = match e.ajustes.maos_modo.as_str() {
        "agir" if a.segurados > 0 => format!("segurando {}", a.segurados),
        "desligado" => "desligadas".into(),
        _ => "pressão baixa: nada".into(),
    };
    let tcc = a.tcc_corte.map_or("—".into(), |c| {
        format!("corta sozinho a {} °C", num(c, 0))
    });

    // O caminho de quem venceu.
    let caminho: &[&str] = match vence {
        "alvo" => &["sensor>alvo", "alvo>menor", "menor>pl1"],
        "antecipado" => &[
            "processos>antecipa",
            "antecipa>curva",
            "curva>menor",
            "menor>pl1",
            "pl1>grafana",
        ],
        "curva" => &[
            "sensor>prev",
            "prev>curva",
            "curva>menor",
            "menor>pl1",
            "pl1>grafana",
        ],
        _ => &["sensor>prev", "prev>curva", "curva>menor", "menor>pl1"],
    };
    let no_caminho = |id: &str| caminho.iter().any(|c| c.split('>').any(|p| p == id));
    let apagado = |id: &str| match id {
        "alvo" => e.ajustes.alvo_c <= 0.0,
        "antecipa" => d.suspeitos_ativos.is_empty(),
        "maos" | "cgroup" => {
            e.ajustes.maos_modo == "desligado" || a.segurados == 0 && vence != "curva"
        }
        _ => false,
    };
    let no = |id: &str, tipo: &str, titulo: &str, sub: String, x: f32, y: f32| {
        let mut n = FlowNode::new(id, tipo, titulo, x, y).subtitle(sub);
        if x > 0.0 {
            n = n.port(FlowPort::input("in"));
        }
        if x < 1040.0 {
            n = n.port(FlowPort::output("out"));
        }
        if no_caminho(id) {
            n.status(FlowNodeStatus::Running)
        } else if apagado(id) {
            n.status(FlowNodeStatus::Disabled)
        } else {
            n
        }
    };
    let mut g = FlowGraph::new()
        .node(no("sensor", "leitura", "Sensor do pacote", temp, 0.0, 40.0))
        .node(no(
            "processos",
            "leitura",
            "Processos",
            processos,
            0.0,
            200.0,
        ))
        .node(no("psi", "leitura", "Pressão de memória", psi, 0.0, 360.0))
        .node(no("prev", "regra", "Previsão", prev, 260.0, 40.0))
        .node(no(
            "antecipa",
            "regra",
            "Histórico negativo",
            antecipa,
            260.0,
            200.0,
        ))
        .node(no("maos", "regra", "Mãos", maos, 260.0, 360.0))
        .node(no("curva", "regra", "Curva de freio", curva, 520.0, 40.0))
        .node(no("alvo", "regra", "Alvo", alvo, 520.0, 160.0))
        .node(no("menor", "regra", "Vale o menor", menor, 780.0, 100.0))
        .node(no("pl1", "acao", "RAPL PL1", pl1, 1040.0, 40.0))
        .node(no(
            "grafana",
            "acao",
            "Grafana",
            "anota o que muda".into(),
            1040.0,
            160.0,
        ))
        .node(no("tcc", "acao", "TCC no chip", tcc, 1040.0, 280.0))
        .node(no(
            "cgroup",
            "acao",
            "cgroup e Docker",
            "segura quem abusa".into(),
            1040.0,
            400.0,
        ));
    for ligacao in [
        "sensor>prev",
        "sensor>alvo",
        "prev>curva",
        "processos>antecipa",
        "antecipa>curva",
        "psi>maos",
        "maos>cgroup",
        "curva>menor",
        "alvo>menor",
        "menor>pl1",
        "pl1>grafana",
    ] {
        let (de, para) = ligacao.split_once('>').unwrap_or_default();
        let mut aresta = FlowEdge::new(
            ligacao,
            FlowEndpoint::new(de, "out"),
            FlowEndpoint::new(para, "in"),
        );
        aresta = if caminho.contains(&ligacao) {
            aresta.animated(true)
        } else {
            aresta.dashed()
        };
        g = g.edge(aresta);
    }
    g
}

/// O tamanho do diagrama em unidades do FlowCanvas: da primeira coluna à
/// última (1040 + 180) e da primeira linha à última (40 a 400 + 76).
const DIAGRAMA: (f32, f32, f32) = (1220.0, 40.0, 476.0);

/// O fluxo fica parado: enquadrado pela largura, sem arrastar nem zoom. A vista
/// é um `Binding::constant`, que ignora o que o arrastar e a roda pedem.
fn fluxo(estado: State<Option<Estado>>, w: f32) -> impl View {
    cartao(
        "Como ele decidiu o limite",
        "Cada volta de 2 s: o que ele leu, o que cada regra pediu e o que foi gravado no chip. O caminho em execução é o que venceu.",
        w,
        view_fn(move || {
            let e = lido(&estado);
            let (largura, topo, base) = DIAGRAMA;
            let caixa = w - 48.0;
            let zoom = ((caixa - 48.0) / largura).min(1.0);
            let altura = (base - topo) * zoom + 80.0;
            let vista = FlowViewport {
                offset: ravenrs::core::Point::new(
                    (caixa - largura * zoom) / 2.0,
                    (altura - (base - topo) * zoom) / 2.0 - topo * zoom,
                ),
                zoom,
                fit: false,
            };
            VStack::new((
                FlowCanvas::new(Binding::constant(grafo(&e)))
                    .view(Binding::constant(vista))
                    .range(zoom, zoom)
                    .controls(false)
                    .kind("leitura", FlowKind::new(IconName::Zap, "Leitura").tone(Semantic::Warning))
                    .kind("regra", FlowKind::new(IconName::GitBranch, "Regra").tone(Semantic::Success))
                    .kind("acao", FlowKind::new(IconName::Package, "Ação"))
                    .read_only(true)
                    .minimap(false)
                    .editable_titles(false)
                    .resizable(false)
                    .rotatable(false)
                    .quick_create(false)
                    .size(caixa, altura),
                Text::new(match e.decisao.vence.as_str() {
                    "alvo" => format!("Venceu o alvo: {} W segura a CPU perto de {} °C.", num(e.agora.pl1_alvo, 1), num(e.ajustes.alvo_c, 0)),
                    "curva" => format!("Venceu a curva: previu {} °C e freou a {} W antes do pico.", e.agora.prevista.map_or("—".into(), |p| num(p, 0)), num(e.agora.pl1_alvo, 1)),
                    "antecipado" => "Venceu o histórico: um suspeito começou a trabalhar, e ele freou antes de o sensor subir.".into(),
                    _ => format!("Ninguém precisou frear: vale o limite normal de {} W.", num(e.agora.pl1_normal, 0)),
                })
                .style(TextStyle::Callout),
            ))
            .spacing(12.0)
        }),
    )
}

fn potencia(estado: State<Option<Estado>>, largura: f32) -> impl View {
    cartao(
        "Potência e limite",
        "Quanto o pacote gasta e o teto que o termostato dá",
        largura,
        view_fn(move || {
            let e = lido(&estado);
            Chart::new(ChartKind::Line)
                .categories(e.serie.iter().map(|s| ha(e.agora_unix, s.t)))
                .series("Potência", e.serie.iter().map(|s| s.potencia))
                .series("Limite PL1", e.serie.iter().map(|s| s.pl1))
                .value_domain(0.0, 40.0)
                .hide_category_axis()
                .show_value_axis()
                .height(200.0)
                .no_appear()
                .format(|v| format!("{} W", num(v, 1)))
        }),
    )
}

fn ventoinhas(estado: State<Option<Estado>>, largura: f32) -> impl View {
    cartao(
        "Ventoinhas",
        "No máximo desde o boot",
        largura,
        view_fn(move || {
            let e = lido(&estado);
            let nomes = ["Esquerda", "Direita"];
            let maximo = e
                .ventoinhas
                .iter()
                .filter_map(|v| v.maximo)
                .fold(0.0, f64::max)
                .max(1.0);
            let maior = e
                .ventoinhas
                .iter()
                .filter_map(|v| v.rpm)
                .fold(0.0, f64::max);
            Chart::new(ChartKind::RadialBar)
                .categories(e.ventoinhas.iter().enumerate().map(|(i, v)| {
                    format!(
                        "{} {}",
                        nomes.get(i).copied().unwrap_or("Ventoinha"),
                        milhar(v.rpm.unwrap_or(0.0))
                    )
                }))
                .series("RPM", e.ventoinhas.iter().map(|v| v.rpm.unwrap_or(0.0)))
                .value_domain(0.0, maximo)
                .tracks()
                .vary_colors()
                .center_text(milhar(maior), "RPM")
                .height(200.0)
                .no_appear()
        }),
    )
}

fn quem_gasta(estado: State<Option<Estado>>, largura: f32) -> impl View {
    cartao(
        "Quem gasta a CPU",
        "Fatia da máquina agora",
        largura,
        view_fn(move || {
            let e = lido(&estado);
            let usado: f64 = e.processos.iter().map(|p| p.fatia).sum::<f64>().min(1.0);
            let mut nomes: Vec<String> = e.processos.iter().map(|p| p.nome.clone()).collect();
            let mut fatias: Vec<f64> = e.processos.iter().map(|p| p.fatia * 100.0).collect();
            nomes.push("Livre".into());
            fatias.push((1.0 - usado).max(0.0) * 100.0);
            Chart::new(ChartKind::Doughnut)
                .categories(nomes)
                .series("CPU", fatias)
                .vary_colors()
                .center_text(format!("{} %", num(usado * 100.0, 0)), "em uso")
                .height(200.0)
                .no_appear()
                .format(|v| format!("{} %", num(v, 0)))
        }),
    )
}

fn freio_por_hora(estado: State<Option<Estado>>, largura: f32) -> impl View {
    cartao(
        "Freando por hora",
        "Minutos com o PL1 abaixo do normal, nas últimas 12 h",
        largura,
        view_fn(move || {
            let e = lido(&estado);
            Chart::new(ChartKind::Bar)
                .categories(e.freio_por_hora.iter().map(|h| format!("{}h", h.hora)))
                .series(
                    "Minutos freando",
                    e.freio_por_hora.iter().map(|h| h.minutos),
                )
                .value_domain(0.0, 60.0)
                .height(268.0)
                .no_appear()
                .format(|v| format!("{} min", num(v, 0)))
        }),
    )
}

fn sensores(estado: State<Option<Estado>>, largura: f32) -> impl View {
    cartao(
        "Sensores do Mac",
        "Os seis mais quentes, pelo applesmc",
        largura,
        view_fn(move || {
            let e = lido(&estado);
            let barras: Vec<AnyView> = e
                .sensores
                .iter()
                .map(|s| {
                    Box::new(
                        ProgressBar::new((s.celsius / 100.0) as f32)
                            .label(nome_do_sensor(&s.nome))
                            .detail(format!("{} °C", num(s.celsius, 0)))
                            .tone(if s.celsius >= 55.0 {
                                Tone::Warning
                            } else {
                                Tone::Accent
                            }),
                    ) as AnyView
                })
                .collect();
            VStack::new(barras)
                .spacing(12.0)
                .align(Align::Stretch)
                .width(largura - 48.0)
        }),
    )
}

fn processos(estado: State<Option<Estado>>, largura: f32) -> impl View {
    view_fn(move || {
        let e = lido(&estado);
        let mut tabela = Table::new([
            Column::new("Processo").weight(2.0),
            Column::new("CPU").numeric(),
            Column::new("Nota de calor").numeric(),
            Column::new("Situação"),
        ]);
        for p in &e.processos {
            tabela = tabela.row([
                p.nome.clone(),
                format!("{} %", num(p.fatia * 100.0, 0)),
                p.nota.map_or("—".into(), |n| format!("{} °C/s", num(n, 2))),
                if p.suspeito {
                    "Suspeito".into()
                } else {
                    "Comum".into()
                },
            ]);
        }
        cartao(
            "Processos",
            "Quem gasta a CPU agora, com a nota de calor do histórico",
            largura,
            tabela,
        )
    })
}

fn feitos(estado: State<Option<Estado>>, largura: f32) -> impl View {
    view_fn(move || {
        let e = lido(&estado);
        let linhas: Vec<AnyView> = e
            .feitos
            .iter()
            .take(4)
            .map(|f| {
                let icone = if f.tags.iter().any(|t| t == "ajuste") {
                    IconName::Settings
                } else if f.tags.iter().any(|t| t == "freio") {
                    IconName::Zap
                } else {
                    IconName::Check
                };
                let (titulo, detalhe) = partes_do_aviso(&f.texto);
                let caber = ((largura - 200.0) / 7.0).max(16.0) as usize;
                let (titulo, detalhe) = (encurtar(&titulo, caber), encurtar(&detalhe, caber + 8));
                let mut item = ListItem::new(titulo)
                    .icon(icone)
                    .trailing(ha(e.agora_unix, f.em_ms / 1000.0));
                if !detalhe.is_empty() {
                    item = item.subtitle(detalhe);
                }
                Box::new(item) as AnyView
            })
            .collect();
        let lista: AnyView = if linhas.is_empty() {
            Box::new(Text::new("Nada desde que o termostato subiu.").foreground(Semantic::InkMuted))
        } else {
            Box::new(
                VStack::new((List::new(linhas),))
                    .width(largura)
                    .align(Align::Stretch),
            )
        };
        cartao(
            "O que ele fez",
            "Os últimos avisos, os mesmos do journal e do Grafana",
            largura,
            lista,
        )
    })
}

/// Corta em `n` letras, com reticências: o horário à direita nunca sai do cartão.
pub fn encurtar(texto: &str, n: usize) -> String {
    if texto.chars().count() <= n {
        texto.to_string()
    } else {
        texto
            .chars()
            .take(n.saturating_sub(1))
            .collect::<String>()
            .trim_end()
            .to_string()
            + "…"
    }
}

/// `"🔥 88 °C agora, …: freio de calor acionado, PL1 em 25.0 W"` →
/// (`"Freio de calor acionado, PL1 em 25.0 W"`, `"88 °C agora, …"`): o journal
/// diz o porquê antes dos dois-pontos e o quê depois; o emoji fica de fora,
/// porque a fonte do painel não o tem.
pub fn partes_do_aviso(texto: &str) -> (String, String) {
    let limpo = texto
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .trim();
    let (antes, depois) = limpo.rsplit_once(": ").unwrap_or(("", limpo));
    let mut titulo = depois.trim().to_string();
    if let Some(primeira) = titulo.get(..1) {
        titulo = primeira.to_uppercase() + &titulo[1..];
    }
    (titulo, antes.trim().to_string())
}

fn historico(estado: State<Option<Estado>>, w: f32) -> impl View {
    view_fn(move || {
        let e = lido(&estado);
        let suspeitos = e.historico.iter().filter(|f| f.suspeito).count();
        let oito: Vec<&modelo::Ficha> = e.historico.iter().take(8).collect();
        let grafico = Chart::new(ChartKind::Bar)
            .horizontal()
            .categories(oito.iter().map(|f| f.nome.clone()))
            .series("Nota de calor", oito.iter().map(|f| f.nota))
            .reference(
                ChartReference::new(e.suspeito_c_s)
                    .label(format!("vira suspeito ({} °C/s)", num(e.suspeito_c_s, 0)))
                    .color(Semantic::Guide),
            )
            .value_domain(0.0, 1.5)
            .height(420.0)
            .no_appear()
            .format(|v| format!("{} °C/s", num(v, 2)));
        let mut tabela = Table::new([
            Column::new("Processo").weight(2.0),
            Column::new("Nota").numeric(),
            Column::new("Maior subida").numeric(),
            Column::new("Vezes visto").numeric(),
            Column::new("Visto por último"),
            Column::new("Situação"),
        ]);
        for f in e.historico.iter().take(10) {
            let situacao = if f.suspeito {
                "Suspeito"
            } else if f.nota >= e.suspeito_c_s * 0.9 {
                "Quase"
            } else if f.observacoes < 3 {
                "Pouco visto"
            } else {
                "Comum"
            };
            tabela = tabela.row([
                f.nome.clone(),
                format!("{} °C/s", num(f.nota, 2)),
                format!("+{} °C/s", num(f.pico, 1)),
                milhar(f64::from(f.observacoes)),
                ha(e.agora_unix, f.visto_em),
                situacao.to_string(),
            ]);
        }
        VStack::new((
            HStack::new((
                VStack::new((
                    Text::new("Histórico dos mal-educados").style(TextStyle::Headline),
                    Text::new("Quanto cada processo esquenta a máquina quando trabalha. Com nota 1 °C/s e 3 observações ele vira suspeito, e o freio vem antes. Esquece quem some por 30 dias.")
                        .style(TextStyle::Callout)
                        .foreground(Semantic::InkMuted),
                ))
                .spacing(4.0)
                .width(if w >= 900.0 { w * 0.7 } else { w }),
                Spacer::new(),
                Badge::new(format!("{} no histórico · {suspeitos} suspeitos", e.historico.len())),
            ))
            .align(Align::Center)
            .width(w)
            .wrap(),
            HStack::new((
                cartao(
                    "Nota de calor",
                    "°C por segundo que a CPU sobe com ele, em média",
                    if w >= 900.0 { (w - VAO) * 0.36 } else { w },
                    grafico,
                ),
                VStack::new((tabela,)).width(if w >= 900.0 { (w - VAO) * 0.64 } else { w }),
            ))
            .spacing(VAO)
            .align(Align::Stretch)
            .wrap(),
        ))
        .spacing(12.0)
    })
}

fn rodape(estado: State<Option<Estado>>, w: f32) -> impl View {
    view_fn(move || {
        let e = lido(&estado);
        let ventoinhas = e
            .ventoinhas
            .iter()
            .map(|v| milhar(v.rpm.unwrap_or(0.0)))
            .collect::<Vec<_>>()
            .join(" · ");
        let viva = estado.get().is_some();
        StatusBar::new()
            .left((StatusBar::action(format!(
                "Termostato {}",
                if e.versao.is_empty() {
                    "—"
                } else {
                    &e.versao
                }
            )),))
            .center((StatusBar::text(if viva {
                format!(
                    "Lido agora · TCC {} °C · PSI {} %",
                    e.agora.tcc_corte.map_or("—".into(), |c| num(c, 0)),
                    e.agora.psi_memoria.map_or("—".into(), |p| num(p, 0))
                )
            } else {
                "Sem resposta do termostato".into()
            })
            .grow(1.0),))
            .center_tone(if viva { Tone::Success } else { Tone::Danger })
            .right((StatusBar::text(format!("Ventoinhas {ventoinhas} RPM")),))
            .width(w)
    })
}

#[cfg(target_arch = "wasm32")]
mod web {
    use std::cell::RefCell;
    use std::rc::Rc;

    use ravenrs::prelude::State;
    use wasm_bindgen::prelude::*;

    use crate::modelo::Estado;

    thread_local! {
        static ESTADO: State<Option<Estado>> = State::new(None);
        static APP: RefCell<Option<ravenrs::web::WebApp>> = const { RefCell::new(None) };
    }

    #[wasm_bindgen]
    extern "C" {
        /// `web/index.html`: faz o `POST /api/ajustes` (com espera curta para a régua).
        #[wasm_bindgen(js_namespace = window, js_name = termostatoAjustar)]
        fn ajustar_js(corpo: &str);
    }

    /// A página entrega cada `GET /api/estado` aqui.
    #[wasm_bindgen]
    pub fn receber(json: &str) {
        match serde_json::from_str::<Estado>(json) {
            Ok(e) => ESTADO.with(|s| s.set_if_changed(Some(e))),
            Err(e) => web_sys::console::error_1(&format!("estado ilegível: {e}").into()),
        }
    }

    /// A página avisa que o termostato não respondeu.
    #[wasm_bindgen]
    pub fn sem_resposta() {
        ESTADO.with(|s| s.set_if_changed(None));
    }

    #[wasm_bindgen(start)]
    pub fn start() {
        console_error_panic_hook::set_once();
        let estado = ESTADO.with(Clone::clone);
        let ajustar: crate::Ajustar = Rc::new(|corpo| ajustar_js(&corpo.to_string()));
        wasm_bindgen_futures::spawn_local(async move {
            let opcoes = ravenrs::web::WebOptions::new("painel")
                .backend(ravenrs::web::WebBackend::WebGl2)
                .follow_system_theme();
            match ravenrs::web::start(crate::app(estado, ajustar), opcoes).await {
                Ok(app) => APP.with(|a| *a.borrow_mut() = Some(app)),
                Err(e) => web_sys::console::error_1(&e.to_string().into()),
            }
        });
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn exemplo() -> Estado {
        serde_json::from_str(include_str!("../exemplo.json")).unwrap()
    }

    #[test]
    fn o_grafo_marca_quem_venceu() {
        let mut e = exemplo();
        e.decisao.vence = "alvo".into();
        let g = grafo(&e);
        let json = serde_json::to_string(&g.to_json()).unwrap();
        assert!(json.contains("Vale o menor"));
        assert!(json.contains("pelo alvo"));
    }

    #[test]
    fn a_pagina_desenha_sem_janela() {
        let estado = State::new(Some(exemplo()));
        let ajustes = Rc::new(std::cell::RefCell::new(Vec::<Value>::new()));
        let guardados = ajustes.clone();
        let ajustar: Ajustar = Rc::new(move |v| guardados.borrow_mut().push(v));
        let mut ui = Ui::new(app(estado, ajustar), Theme::raven_dark());
        ui.set_viewport(ravenrs::core::Size::new(1280.0, 900.0), 1.0);
        ui.update();
        let textos = ui
            .scene()
            .primitives()
            .iter()
            .filter(|p| matches!(p, ravenrs::core::Primitive::Glyphs(_)))
            .count();
        assert!(textos > 40, "a página tem texto: {textos}");
    }

    #[test]
    fn o_aviso_vira_titulo_e_detalhe() {
        assert_eq!(
            partes_do_aviso(
                "🔥 88 °C agora, 98 °C previstos: freio de calor acionado, PL1 em 25.0 W"
            ),
            (
                "Freio de calor acionado, PL1 em 25.0 W".into(),
                "88 °C agora, 98 °C previstos".into()
            )
        );
        assert_eq!(
            partes_do_aviso("🎛️ ajuste: alvo_c = 65.0"),
            ("Alvo_c = 65.0".into(), "ajuste".into())
        );
        assert_eq!(
            partes_do_aviso("🌡️ termostato 0.8.0 subiu"),
            ("Termostato 0.8.0 subiu".into(), String::new())
        );
    }

    #[test]
    fn encurta_com_reticencias() {
        assert_eq!(encurtar("Freio de calor acionado", 10), "Freio de…");
        assert_eq!(encurtar("curto", 10), "curto");
    }

    #[test]
    fn sem_estado_ainda_desenha() {
        let ajustar: Ajustar = Rc::new(|_| {});
        let mut ui = Ui::new(app(State::new(None), ajustar), Theme::raven_light());
        ui.set_viewport(ravenrs::core::Size::new(390.0, 800.0), 2.0);
        ui.update();
    }
}

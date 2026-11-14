// AstraScreenShare - programa de PC (Tauri). Substitui o main.js do Electron.
// A interface continua sendo a pasta web/. O web/bridge.js monta o objeto
// window.astra (que o web/app.js usa) em cima dos comandos definidos aqui.
//
// RASCUNHO: escrito sem compilar. Pontos em aberto estão marcados com TODO.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, UserAttentionType, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::{ManagerExt as _, MacosLauncher};
use tauri_plugin_dialog::{DialogExt as _, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_global_shortcut::{GlobalShortcutExt as _, ShortcutState};
use tauri_plugin_notification::NotificationExt as _;

#[cfg(windows)]
use std::os::windows::process::CommandExt as _;

const VERSAO: &str = env!("CARGO_PKG_VERSION");
const TOQUE_VALE: Duration = Duration::from_secs(20);
const CHAVE_PUBLICA_PEM: &str = "-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEATmEmLbLFobUtr8QqqxcZiNiJ7pBiHZ7p5fNdRuRQ0ws=\n-----END PUBLIC KEY-----\n";
const BRIDGE: &str = include_str!("../../web/bridge.js");
const TECLAS_DE_JOGO: &[&str] = &["W", "A", "S", "D", "⌫", "Tab", "Enter", "Caps", "Esc", "Espaço", "PgUp", "PgDn",
    "End", "Home", "←", "↑", "→", "↓", "Ins", "Del", "Shift", "Ctrl", "Alt"];

static AVISOU_SEGUNDO_PLANO: AtomicBool = AtomicBool::new(false);

// ---------- endereços ----------
// Só valem em testes (build de debug). No programa instalado ficam fixos.
fn site() -> String {
    if cfg!(debug_assertions) {
        if let Ok(v) = std::env::var("ASTRA_SITE") { return v; }
    }
    "https://astrascreenshare.com/app/".into()
}
fn servidor() -> String {
    if cfg!(debug_assertions) {
        if let Ok(v) = std::env::var("ASTRA_SERVIDOR") { return v; }
    }
    "wss://astrascreenshare.com/ws".into()
}

// ---------- estado compartilhado ----------
#[derive(Clone, serde::Serialize)]
struct Fonte {
    id: String,
    nome: String,
    tela: bool,
    indice: usize,
    // data URL da miniatura (desktop.js usa como src). Vazio até implementar as miniaturas.
    miniatura: String,
}

#[derive(Default)]
struct Estado {
    transmitindo: bool,
    fontes: Vec<Fonte>,
    fonte_escolhida: Option<(Fonte, String)>, // (tela, modo de som)
    escolha_em: Option<Instant>,
    monitor_da_live: Option<usize>,
    ultimo_toque: Option<Instant>,
    captura_em: Option<Instant>,
    capturando_tela: String,
    som_modo: String,
    som_ativo: bool,
    capturador: Option<Child>,
    silencio: Option<Child>,
    teclas: Option<Child>,
    relaxado: bool,
    protegido: bool,
    visor_aberto: bool,
    emergencia: String,
    desempenho_ativo: bool,
    atualizando: bool,
    aparelho: String,
    riscos_seg: Option<(Instant, u32)>,
    saindo: bool,
}
type Shared = Mutex<Estado>;

fn tocou(est: &Estado) -> bool {
    est.ultimo_toque.map_or(false, |t| t.elapsed() < TOQUE_VALE)
}
fn captura_recente(est: &Estado) -> bool {
    !est.capturando_tela.is_empty()
        && (est.transmitindo || est.captura_em.map_or(false, |t| t.elapsed() < Duration::from_secs(60)))
}

// ---------- aparelho (código para banimento) ----------
fn sha256_hex(texto: &str) -> String {
    let mut h = Sha256::new();
    h.update(texto.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(windows)]
fn ler_registro(chave: &str, valor: &str) -> String {
    let reg = std::path::Path::new(&std::env::var("SystemRoot").unwrap_or("C:\\Windows".into()))
        .join("System32").join("reg.exe");
    let Ok(saida) = Command::new(reg)
        .args(["query", chave, "/v", valor])
        .creation_flags(0x0800_0000)
        .output() else { return String::new() };
    for linha in String::from_utf8_lossy(&saida.stdout).lines() {
        let t = linha.trim();
        if t.starts_with(valor) {
            return t.split_whitespace().skip(2).collect::<Vec<_>>().join(" ")
                .replace(['{', '}'], "").to_lowercase();
        }
    }
    String::new()
}
#[cfg(not(windows))]
fn ler_registro(_chave: &str, _valor: &str) -> String { String::new() }

fn ler_aparelho() -> String {
    let guid = ler_registro("HKLM\\SOFTWARE\\Microsoft\\Cryptography", "MachineGuid");
    let uuid = ler_registro("HKLM\\SYSTEM\\HardwareConfig", "LastConfig");
    if guid.is_empty() && uuid.is_empty() { return String::new(); }
    sha256_hex(&format!("astra-aparelho-v1|{guid}|{uuid}"))
}

// ---------- atualização assinada ----------
fn chave_publica() -> Result<VerifyingKey, String> {
    let corpo: String = CHAVE_PUBLICA_PEM.lines().filter(|l| !l.starts_with("-----")).collect();
    let der = base64::engine::general_purpose::STANDARD.decode(corpo).map_err(|e| e.to_string())?;
    // O DER do Ed25519 termina com os 32 bytes da chave.
    let bytes: [u8; 32] = der[der.len() - 32..].try_into().map_err(|_| "chave pública inválida".to_string())?;
    VerifyingKey::from_bytes(&bytes).map_err(|e| e.to_string())
}

fn versao_partes(v: &str) -> [u32; 3] {
    let mut p = [0u32; 3];
    for (i, n) in v.split('.').take(3).enumerate() { p[i] = n.parse().unwrap_or(0); }
    p
}

fn nome_do_instalador(cabecalho: Option<&str>) -> Result<String, String> {
    let cd = cabecalho.unwrap_or("");
    let nome = cd.split("filename=").nth(1).unwrap_or("").trim().trim_matches(|c| c == '"' || c == ';').to_string();
    let ok_formato = nome.starts_with("AstraScreenShare-Instalador-") && nome.ends_with(".exe")
        && nome.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    if !ok_formato { return Err("o servidor não disse qual versão está mandando".into()); }
    let versao = nome.trim_start_matches("AstraScreenShare-Instalador-").trim_end_matches(".exe");
    // Só versão MAIS NOVA: nunca volta para um instalador antigo (mesmo que assinado).
    if versao_partes(versao) <= versao_partes(VERSAO) {
        return Err("o servidor ofereceu uma versão que não é mais nova que a sua".into());
    }
    Ok(nome)
}

fn verificar_assinatura(nome: &str, sha: &str, assinatura_b64: &str) -> Result<(), String> {
    let bytes = base64::engine::general_purpose::STANDARD.decode(assinatura_b64.trim()).map_err(|_| "assinatura inválida".to_string())?;
    let sig = Signature::from_slice(&bytes).map_err(|_| "assinatura inválida".to_string())?;
    chave_publica()?
        .verify(format!("{nome}\n{sha}").as_bytes(), &sig)
        .map_err(|_| "a assinatura do instalador não confere, por segurança ele não foi aberto".to_string())
}

#[tauri::command]
fn conferir_modulo(nome: String, texto: String, assinatura: String) -> bool {
    if nome.is_empty() || nome.len() > 32 || !nome.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
        return false;
    }
    if texto.len() > 2_000_000 { return false; }
    let sha = sha256_hex(&texto);
    verificar_assinatura(&format!("modulo:{nome}"), &sha, &assinatura).is_ok()
}

#[tauri::command]
async fn atualizar(app: AppHandle) -> Result<(), String> {
    {
        let state = app.state::<Shared>();
        let mut est = state.lock().unwrap();
        if est.atualizando { return Err("já está atualizando".into()); }
        est.atualizando = true;
    }
    let resultado = baixar_e_instalar(&app).await;
    if resultado.is_err() {
        app.state::<Shared>().lock().unwrap().atualizando = false;
    }
    resultado
}

async fn baixar_e_instalar(app: &AppHandle) -> Result<(), String> {
    let cliente = reqwest::Client::new();
    let resp = cliente.get(format!("{}baixar/pc", site())).header("Cache-Control", "no-cache")
        .send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() { return Err(format!("o servidor respondeu {}", resp.status())); }
    let nome = nome_do_instalador(resp.headers().get("content-disposition").and_then(|v| v.to_str().ok()))?;
    let total = resp.content_length().unwrap_or(0);

    // Pasta nova e aleatória a cada atualização.
    let agora = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let pasta = std::env::temp_dir().join(format!("astra-atualizacao-{agora}"));
    fs::create_dir_all(&pasta).map_err(|e| e.to_string())?;
    let destino = pasta.join("AstraScreenShare-atualizacao.exe");
    let mut arquivo = fs::File::create(&destino).map_err(|e| e.to_string())?;

    let mut hash = Sha256::new();
    let mut recebido: u64 = 0;
    let mut ultimo: i64 = -1;
    let mut corrente = resp.bytes_stream();
    while let Some(pedaco) = corrente.next().await {
        let pedaco = pedaco.map_err(|e| e.to_string())?;
        hash.update(&pedaco);
        arquivo.write_all(&pedaco).map_err(|e| e.to_string())?;
        recebido += pedaco.len() as u64;
        let pct = if total > 0 { (recebido * 100 / total) as i64 } else { 0 };
        if pct != ultimo {
            ultimo = pct;
            let _ = app.emit_to("main", "atualizar-progresso", pct);
        }
    }
    arquivo.flush().map_err(|e| e.to_string())?;
    drop(arquivo);

    // Tem que ser um programa do Windows inteiro ("MZ" e o tamanho certo).
    let mut inicio = [0u8; 2];
    fs::File::open(&destino).and_then(|mut f| f.read_exact(&mut inicio)).map_err(|e| e.to_string())?;
    let tamanho = fs::metadata(&destino).map_err(|e| e.to_string())?.len();
    if &inicio != b"MZ" || (total > 0 && tamanho != total) {
        return Err("o arquivo baixado veio incompleto".into());
    }

    let sha = format!("{:x}", hash.finalize());
    let assinatura_url = reqwest::Url::parse(&site()).map_err(|e| e.to_string())?
        .join(&format!("../assinaturas/{nome}.sig")).map_err(|e| e.to_string())?;
    let sig_resp = cliente.get(assinatura_url).header("Cache-Control", "no-cache").send().await
        .map_err(|e| e.to_string())?;
    if !sig_resp.status().is_success() {
        return Err("a versão nova ainda não tem assinatura; tente de novo mais tarde".into());
    }
    let assinatura = sig_resp.text().await.map_err(|e| e.to_string())?;
    verificar_assinatura(&nome, &sha, &assinatura)?;

    // Confere de novo o arquivo que está no disco, logo antes de abrir.
    let sha_disco = {
        let bytes = fs::read(&destino).map_err(|e| e.to_string())?;
        format!("{:x}", Sha256::digest(&bytes))
    };
    if sha_disco != sha { return Err("o arquivo mudou depois de baixado, por segurança ele não foi aberto".into()); }

    let _ = app.emit_to("main", "atualizar-progresso", "instalando");
    // TODO: salvar a cópia de migração (localStorage) antes de trocar a versão.
    let mut cmd = Command::new(&destino);
    cmd.args(["/S", "--force-run"]);
    #[cfg(windows)]
    cmd.creation_flags(0x0000_0008 | 0x0000_0200); // DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    cmd.spawn().map_err(|e| e.to_string())?;
    let app2 = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(800));
        sair(&app2);
    });
    Ok(())
}

// ---------- janelas ----------
fn url_da_pagina(url: &tauri::Url) -> bool {
    // Só a própria página (protocolo do app). Link nenhum troca o que está na janela.
    url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost") || url.scheme() == "about"
}

fn criar_janela_principal(app: &AppHandle, boot: &Value, oculto: bool) -> tauri::Result<WebviewWindow> {
    let script = format!("window.__ASTRA_BOOT__ = {boot};\n{BRIDGE}");
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("AstraScreenShare")
        .inner_size(1280.0, 780.0)
        .min_inner_size(400.0, 520.0)
        .decorations(false)                 // a janela é do Astra (botões no web/app.js)
        .background_color(tauri::window::Color(0x1e, 0x1f, 0x22, 255))
        .visible(!oculto)
        .initialization_script(&script)
        .on_navigation(|url| url_da_pagina(url))
        .build()
}

fn criar_janela_extra(app: &AppHandle, rotulo: &str, arquivo: &str, transparente: bool) -> tauri::Result<WebviewWindow> {
    let janela = WebviewWindowBuilder::new(app, rotulo, WebviewUrl::App(arquivo.into()))
        .decorations(false)
        .transparent(transparente)
        .resizable(false)
        .skip_taskbar(true)
        .focusable(false)
        .always_on_top(true)
        .visible(false)
        .shadow(false)
        .visible_on_all_workspaces(true)
        .on_navigation(|url| url_da_pagina(url))
        .build()?;
    janela.set_ignore_cursor_events(true)?;
    Ok(janela)
}

// Janela transparente com os desenhos dos amigos (fica de fora da captura).
fn desenho(app: &AppHandle) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window("desenho") { return Some(w); }
    let w = criar_janela_extra(app, "desenho", "desenho.html", true).ok()?;
    let _ = w.set_content_protected(true);
    Some(w)
}

// Pop-up do chat por cima do jogo (canto de cima, à direita).
fn popup(app: &AppHandle) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window("popup") { return Some(w); }
    let w = criar_janela_extra(app, "popup", "popup.html", true).ok()?;
    let _ = w.set_content_protected(true);
    Some(w)
}

fn posicionar_popup(app: &AppHandle, w: &WebviewWindow) {
    let Ok(Some(monitor)) = w.primary_monitor() else { return };
    let tam = monitor.size();
    let escala = monitor.scale_factor();
    let zoom = (tam.width as f64 / escala / 670.0).max(1.0);
    let largura = (200.0 * zoom).round();
    let altura = (tam.height as f64 / escala * 0.6).round();
    let x = (tam.width as f64 / escala) - largura - 12.0;
    let _ = w.set_size(tauri::LogicalSize::new(largura, altura));
    let _ = w.set_position(tauri::LogicalPosition::new(x, 12.0));
    let _ = w.set_zoom(zoom);
    let _ = app; // mantém a assinatura simples
}

fn mostrar(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn tocar_agora(app: &AppHandle) {
    let state = app.state::<Shared>();
    state.lock().unwrap().ultimo_toque = Some(Instant::now());
}

// ---------- ícone perto do relógio ----------
fn status_texto(est: &Estado) -> Option<String> {
    let mut partes = vec![];
    if !est.capturando_tela.is_empty() { partes.push("tela"); }
    if est.som_ativo { partes.push("som"); }
    if est.teclas.is_some() { partes.push("teclas"); }
    if partes.is_empty() { None } else { Some(format!("● Capturando: {}", partes.join(", "))) }
}

fn atualizar_indicador(app: &AppHandle, est: &Estado) {
    let txt = status_texto(est);
    if let Some(w) = app.get_webview_window("main") {
        let titulo = match &txt { Some(t) => format!("AstraScreenShare ({t})"), None => "AstraScreenShare".into() };
        let _ = w.set_title(&titulo);
    }
    if let Some(tray) = app.tray_by_id("main") {
        let dica = match &txt { Some(t) => format!("AstraScreenShare: {t}"), None => "AstraScreenShare".into() };
        let _ = tray.set_tooltip(Some(dica));
        if let Ok(menu) = montar_menu(app, est) { let _ = tray.set_menu(Some(menu)); }
    }
}

fn montar_menu(app: &AppHandle, est: &Estado) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    if let Some(t) = status_texto(est) {
        menu.append(&MenuItem::with_id(app, "status", t, false, None::<&str>)?)?;
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
    menu.append(&MenuItem::with_id(app, "abrir", "Abrir AstraScreenShare", true, None::<&str>)?)?;
    let rotulo_tela = if est.transmitindo { "Parar transmissão" } else { "Transmitir tela" };
    menu.append(&MenuItem::with_id(app, "tela", rotulo_tela, true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    let iniciar = app.autolaunch().is_enabled().unwrap_or(false);
    menu.append(&CheckMenuItem::with_id(app, "iniciar", "Iniciar com o Windows", true, iniciar, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "sair", "Sair", true, None::<&str>)?)?;
    Ok(menu)
}

fn acao_menu(app: &AppHandle, id: &str) {
    match id {
        "abrir" => mostrar(app),
        "tela" => {
            tocar_agora(app);   // clique de verdade no menu do ícone
            let _ = app.emit_to("main", "menu-tela", ());
        }
        "iniciar" => {
            let ligado = app.autolaunch().is_enabled().unwrap_or(false);
            let _ = if ligado { app.autolaunch().disable() } else { app.autolaunch().enable() };
            marcar_configurado(app);
            let state = app.state::<Shared>();
            let est = state.lock().unwrap();
            atualizar_indicador(app, &est);
        }
        "sair" => sair(app),
        _ => {}
    }
}

// ---------- avisos ----------
fn avisar(app: &AppHandle, titulo: &str, texto: &str, sempre: bool) {
    if !sempre {
        // Ao vivo com o "Não perturbe" ligado, o aviso não aparece.
        if app.state::<Shared>().lock().unwrap().silencio.is_some() { return; }
    }
    let _ = app.notification().builder().title(titulo).body(texto).show();
}

fn primeira_vez_iniciar(app: &AppHandle) {
    let marca = app.path().app_data_dir().map(|d| d.join("perguntou-iniciar")).ok();
    if marca.as_ref().map_or(true, |m| m.exists()) { return; }
    let app2 = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(600));
        let sim = app2.dialog()
            .message("Abrir a Astra junto com o Windows?\n\nAssim ela fica perto do relógio e você recebe os convites dos amigos na hora.\n\nDá para mudar quando quiser: clique com o botão direito no ícone perto do relógio.")
            .title("AstraScreenShare")
            .kind(MessageDialogKind::Info)
            .buttons(MessageDialogButtons::OkCancelCustom("Sim, abrir com o Windows".into(), "Não".into()))
            .blocking_show();
        if sim { let _ = app2.autolaunch().enable(); } else { let _ = app2.autolaunch().disable(); }
        marcar_configurado(&app2);
    });
}

fn marcar_configurado(app: &AppHandle) {
    if let Ok(dir) = app.path().app_data_dir() {
        let _ = fs::create_dir_all(&dir);
        let _ = fs::write(dir.join("perguntou-iniciar"), "1");
    }
}

// ---------- escolher a tela ----------
#[tauri::command]
fn listar_fontes(app: AppHandle, state: State<Shared>) -> Vec<Fonte> {
    let mut est = state.lock().unwrap();
    if !tocou(&est) { return vec![]; }
    let monitores = app.get_webview_window("main")
        .and_then(|w| w.available_monitors().ok())
        .unwrap_or_default();
    // TODO: janelas (window:...) e miniaturas ainda não entram aqui.
    let fontes: Vec<Fonte> = monitores.iter().enumerate().map(|(i, m)| Fonte {
        id: format!("screen:{i}"),
        nome: m.name().cloned().unwrap_or_else(|| format!("Tela {}", i + 1)),
        tela: true,
        indice: i,
        miniatura: String::new(),
    }).collect();
    est.fontes = fontes.clone();
    fontes
}

#[derive(Deserialize)]
struct Escolha {
    id: String,
    som: String,
}

#[tauri::command]
fn definir_fonte(escolha: Escolha, state: State<Shared>) -> bool {
    let mut est = state.lock().unwrap();
    if !tocou(&est) {
        est.fonte_escolhida = None;
        return false;
    }
    let achada = est.fontes.iter().find(|f| f.id == escolha.id).cloned();
    est.fontes.clear();
    match achada {
        Some(f) => {
            est.monitor_da_live = if f.tela { Some(f.indice) } else { None };
            est.fonte_escolhida = Some((f, escolha.som));
            est.escolha_em = Some(Instant::now());
            true
        }
        None => {
            est.fonte_escolhida = None;
            false
        }
    }
}

#[tauri::command]
fn transmitindo(sim: bool, app: AppHandle, state: State<Shared>) {
    let mut est = state.lock().unwrap();
    est.transmitindo = sim;
    if sim {
        // TODO: a captura de tela ainda precisa de um caminho nativo ou de confirmação do WebView2
        // (no Electron era o setDisplayMediaRequestHandler). Aqui só registramos a transmissão.
        if let Some((fonte, som)) = est.fonte_escolhida.clone() {
            let recente = est.escolha_em.map_or(false, |t| t.elapsed() < Duration::from_secs(30));
            if recente {
                est.captura_em = Some(Instant::now());
                est.capturando_tela = fonte.nome.clone();
                est.som_modo = som;
                atualizar_indicador(&app, &est);
                avisar(&app, "A Astra começou a transmitir",
                    &format!("Capturando: {}. Para parar, use \"Parar\" no Astra ou o menu do ícone perto do relógio.", fonte.nome), true);
            }
        }
    } else {
        ligar_silencio_interno(&mut est, false);
        desligar_desenho(&app);
        parar_teclas(&mut est);
        parar_capturador(&mut est);
        est.capturando_tela.clear();
        est.captura_em = None;
        atualizar_indicador(&app, &est);
    }
    let _ = app.emit_to("main", "transmitindo-estado", sim);
}

// ---------- som (astra-audio.exe) ----------
fn caminho_capturador(app: &AppHandle) -> Option<std::path::PathBuf> {
    let dev = std::path::PathBuf::from("resources").join("astra-audio.exe");
    let instalado = app.path().resource_dir().ok().map(|d| d.join("astra-audio.exe"));
    if let Some(p) = instalado { if p.exists() { return Some(p); } }
    if dev.exists() { return Some(dev); }
    None
}

fn parar_capturador(est: &mut Estado) {
    if let Some(mut c) = est.capturador.take() { let _ = c.kill(); let _ = c.wait(); }
    est.som_ativo = false;
}

#[tauri::command]
fn som_iniciar(app: AppHandle, state: State<Shared>) -> Result<(), String> {
    let mut est = state.lock().unwrap();
    parar_capturador(&mut est);
    if !captura_recente(&est) { return Err("O som só é capturado durante uma transmissão.".into()); }
    if !cfg!(windows) { return Err("Esse modo de som só existe no Windows.".into()); }
    let exe = caminho_capturador(&app).ok_or("Capturador de som não encontrado.")?;
    let pid = std::process::id().to_string();
    let args: Vec<String> = if est.som_modo == "tudo" {
        // Todo o som do PC menos o do próprio Astra (senão faz eco).
        vec!["excluir-pid".into(), pid]
    } else {
        // Sem o Discord e sem o próprio Astra.
        vec!["mixar".into(), pid, "Discord.exe".into(), "DiscordPTB.exe".into(), "DiscordCanary.exe".into()]
    };
    let mut cmd = Command::new(&exe);
    cmd.args(&args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let mut filho = cmd.spawn().map_err(|e| e.to_string())?;
    let saida = filho.stdout.take().ok_or("sem saída")?;
    let erros = filho.stderr.take().ok_or("sem erro")?;
    let (tx, rx) = mpsc::channel::<Result<(), String>>();
    std::thread::spawn(move || {
        for linha in BufReader::new(erros).lines().map_while(Result::ok) {
            if linha.starts_with("OK") { let _ = tx.send(Ok(())); }
            else if linha.starts_with("ERRO") {
                let msg = if linha.contains("Windows antigo") {
                    "Seu Windows não permite separar o som (precisa do Windows 10 versão 2004 ou mais novo). Use \"Todo o som do PC\".".to_string()
                } else {
                    format!("Não consegui capturar o som: {}", linha.trim_start_matches("ERRO").trim())
                };
                let _ = tx.send(Err(msg));
            }
        }
    });
    let app2 = app.clone();
    std::thread::spawn(move || {
        let mut leitor = BufReader::new(saida);
        let mut buf = [0u8; 8192];
        let mut sobra: Vec<u8> = Vec::new();
        loop {
            match leitor.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    sobra.extend_from_slice(&buf[..n]);
                    let inteiro = sobra.len() - sobra.len() % 4; // quadros inteiros (2 canais x 16 bits)
                    if inteiro > 0 {
                        let pedaco: Vec<u8> = sobra.drain(..inteiro).collect();
                        let _ = app2.emit_to("main", "som-dados", base64::engine::general_purpose::STANDARD.encode(pedaco));
                    }
                }
            }
        }
    });
    est.capturador = Some(filho);
    est.som_ativo = true;
    atualizar_indicador(&app, &est);
    drop(est);

    match rx.recv_timeout(Duration::from_secs(8)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => {
            let state = app.state::<Shared>();
            let mut est = state.lock().unwrap();
            parar_capturador(&mut est);
            atualizar_indicador(&app, &est);
            Err(e)
        }
        Err(_) => {
            let state = app.state::<Shared>();
            let mut est = state.lock().unwrap();
            parar_capturador(&mut est);
            atualizar_indicador(&app, &est);
            Err("O capturador de som não respondeu.".into())
        }
    }
}

#[tauri::command]
fn som_parar(app: AppHandle, state: State<Shared>) {
    let mut est = state.lock().unwrap();
    parar_capturador(&mut est);
    atualizar_indicador(&app, &est);
}

// ---------- "Não perturbe" durante a live ----------
// Desliga o "Não perturbe" (o processo "silencio" volta ao normal ao fechar a entrada).
fn ligar_silencio_interno(est: &mut Estado, sim: bool) {
    if !sim {
        if let Some(mut c) = est.silencio.take() { drop(c.stdin.take()); let _ = c.kill(); let _ = c.wait(); }
    }
}

#[tauri::command]
fn silencio(sim: bool, app: AppHandle, state: State<Shared>) {
    let mut est = state.lock().unwrap();
    if sim && est.silencio.is_none() && cfg!(windows) {
        if let Some(exe) = caminho_capturador(&app) {
            let mut cmd = Command::new(exe);
            cmd.arg("silencio").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
            #[cfg(windows)]
            cmd.creation_flags(0x0800_0000);
            if let Ok(filho) = cmd.spawn() { est.silencio = Some(filho); }
        }
    } else {
        ligar_silencio_interno(&mut est, false);
    }
    if let Some(p) = popup(&app) { let _ = p.set_content_protected(sim); }
}

// ---------- teclas apertadas ----------
fn nome_tecla(vk: u32) -> String {
    match vk {
        8 => "⌫".into(), 9 => "Tab".into(), 13 => "Enter".into(), 20 => "Caps".into(), 27 => "Esc".into(),
        32 => "Espaço".into(), 33 => "PgUp".into(), 34 => "PgDn".into(), 35 => "End".into(), 36 => "Home".into(),
        37 => "←".into(), 38 => "↑".into(), 39 => "→".into(), 40 => "↓".into(), 45 => "Ins".into(), 46 => "Del".into(),
        16 | 160 | 161 => "Shift".into(), 17 | 162 | 163 => "Ctrl".into(), 18 | 164 | 165 => "Alt".into(),
        48..=57 | 65..=90 => char::from_u32(vk).unwrap_or('?').to_string(),
        112..=135 => format!("F{}", vk - 111),
        _ => String::new(),
    }
}

// Só teclas de jogo saem do PC: letras (menos WASD), números e símbolos nunca aparecem.
fn tecla_pode_aparecer(nome: &str) -> bool {
    TECLAS_DE_JOGO.contains(&nome)
        || (nome.starts_with('F') && nome[1..].parse::<u32>().map_or(false, |n| (1..=24).contains(&n)))
}

fn parar_teclas(est: &mut Estado) {
    if let Some(mut c) = est.teclas.take() { drop(c.stdin.take()); let _ = c.kill(); let _ = c.wait(); }
}

#[tauri::command]
fn teclas(sim: bool, app: AppHandle, state: State<Shared>) {
    let mut est = state.lock().unwrap();
    if !sim {
        parar_teclas(&mut est);
        atualizar_indicador(&app, &est);
        return;
    }
    // Só durante uma transmissão que a pessoa começou.
    if !captura_recente(&est) || est.teclas.is_some() || !cfg!(windows) { return; }
    let Some(exe) = caminho_capturador(&app) else { return };
    let mut cmd = Command::new(exe);
    cmd.arg("teclas").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let Ok(mut filho) = cmd.spawn() else { return };
    let saida = filho.stdout.take();
    est.teclas = Some(filho);
    atualizar_indicador(&app, &est);
    drop(est);
    let Some(saida) = saida else { return };
    let app2 = app.clone();
    std::thread::spawn(move || {
        let mut apertadas: Vec<String> = Vec::new();
        let mut ultimo = String::new();
        for linha in BufReader::new(saida).lines().map_while(Result::ok) {
            let mut partes = linha.split_whitespace();
            let (Some(tipo), Some(codigo)) = (partes.next(), partes.next()) else { continue };
            let nome = nome_tecla(codigo.parse().unwrap_or(0));
            if nome.is_empty() || !tecla_pode_aparecer(&nome) { continue; }
            let pos = apertadas.iter().position(|t| *t == nome);
            let janela_em_foco = app2.get_webview_window("main").and_then(|w| w.is_focused().ok()).unwrap_or(false);
            if tipo == "D" && pos.is_none() && !janela_em_foco { apertadas.push(nome); }
            else if tipo == "U" { if let Some(i) = pos { apertadas.remove(i); } }
            while apertadas.len() > 6 { apertadas.remove(0); }
            const MODIFICADORES: [&str; 3] = ["Ctrl", "Shift", "Alt"];
            let mut lista: Vec<String> = MODIFICADORES.iter()
                .filter(|m| apertadas.iter().any(|t| t.as_str() == **m))
                .map(|m| m.to_string())
                .collect();
            lista.extend(apertadas.iter().filter(|t| !MODIFICADORES.contains(&t.as_str())).cloned());
            let chave = lista.join("+");
            if chave != ultimo {
                ultimo = chave;
                let _ = app2.emit_to("main", "teclas-dados", lista);
            }
        }
    });
}

// ---------- desenhos dos amigos ----------
#[tauri::command]
fn risco(r: Value, app: AppHandle, state: State<Shared>) {
    let mut est = state.lock().unwrap();
    if !est.transmitindo { return; }
    // No máximo 60 riscos por segundo.
    let agora = Instant::now();
    let (inicio, n) = est.riscos_seg.unwrap_or((agora, 0));
    let (inicio, n) = if agora.duration_since(inicio) > Duration::from_secs(1) { (agora, 0) } else { (inicio, n) };
    if n >= 60 { est.riscos_seg = Some((inicio, n)); return; }
    est.riscos_seg = Some((inicio, n + 1));
    let Some(indice) = est.monitor_da_live else { return };
    drop(est);
    let Some(janela) = desenho(&app) else { return };
    if let Some(m) = app.get_webview_window("main").and_then(|w| w.available_monitors().ok()).and_then(|v| v.into_iter().nth(indice)) {
        let _ = janela.set_position(*m.position());
        let _ = janela.set_size(*m.size());
    }
    let _ = janela.show();
    let _ = janela.set_always_on_top(true);
    let _ = janela.eval(&format!("desenhar({})", r));
}

fn desligar_desenho(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("desenho") { let _ = w.destroy(); }
}

// Tecla de emergência: apaga os rabiscos na hora e avisa a janela.
#[tauri::command]
fn tecla_emergencia(acel: String, app: AppHandle, state: State<Shared>) -> bool {
    let gs = app.global_shortcut();
    {
        let mut est = state.lock().unwrap();
        if !est.emergencia.is_empty() {
            let _ = gs.unregister(est.emergencia.as_str());
            est.emergencia.clear();
        }
    }
    if acel.is_empty() { return true; }
    let app2 = app.clone();
    let ok = gs.on_shortcut(acel.as_str(), move |app, _atalho, evento| {
        if evento.state() == ShortcutState::Pressed {
            desligar_desenho(&app2);
            let _ = app.emit_to("main", "emergencia", ());
        }
    }).is_ok();
    if ok { state.lock().unwrap().emergencia = acel; }
    ok
}

// ---------- janela: botões, tela cheia, relaxado, acorde ----------
#[tauri::command]
fn janela(acao: String, app: AppHandle) {
    let Some(w) = app.get_webview_window("main") else { return };
    match acao.as_str() {
        "min" => { let _ = w.minimize(); }
        "max" => { if w.is_maximized().unwrap_or(false) { let _ = w.unmaximize(); } else { let _ = w.maximize(); } }
        "fechar" => { let _ = w.close(); } // só esconde (ver on_window_event)
        _ => {}
    }
}

#[tauri::command]
fn proteger_tela(sim: bool, app: AppHandle, state: State<Shared>) {
    let mut est = state.lock().unwrap();
    est.visor_aberto = sim;
    if let Some(w) = app.get_webview_window("main") { let _ = w.set_content_protected(sim || est.protegido); }
}

#[tauri::command]
fn gravador_nvidia() -> String {
    // Lista os processos do gravador da NVIDIA (ele copia a tela direto da placa de vídeo).
    let Ok(saida) = Command::new("tasklist").args(["/FO", "CSV", "/NH"]).output() else { return String::new() };
    let nomes = String::from_utf8_lossy(&saida.stdout).to_lowercase();
    ["nvidia overlay.exe", "nvidia share.exe", "nvsphelper64.exe", "nvsphelper.exe"].iter()
        .find(|n| nomes.contains(&format!("\"{n}\"")))
        .map(|n| n.to_string())
        .unwrap_or_default()
}

static RELAXADO: AtomicBool = AtomicBool::new(false);

#[tauri::command]
fn relaxado(sim: bool, app: AppHandle) {
    let antes = RELAXADO.swap(sim, Ordering::SeqCst);
    if sim && !antes { manter_tela_acesa(); }
    if let Some(w) = app.get_webview_window("main") { let _ = w.set_fullscreen(sim); }
}

// Enquanto o modo relaxado estiver ligado, a tela não apaga sozinha.
#[cfg(windows)]
fn manter_tela_acesa() {
    std::thread::spawn(|| {
        use windows::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED};
        while RELAXADO.load(Ordering::SeqCst) {
            unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_DISPLAY_REQUIRED); }
            std::thread::sleep(Duration::from_secs(30));
        }
        unsafe { SetThreadExecutionState(ES_CONTINUOUS); }
    });
}
#[cfg(not(windows))]
fn manter_tela_acesa() {}

#[tauri::command]
fn acorde(nome: String, app: AppHandle, state: State<Shared>) {
    let Some(w) = app.get_webview_window("main") else { return };
    let estava_na_frente = w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false);
    let _ = w.unminimize();
    let _ = w.show();
    let _ = w.set_always_on_top(true);
    let _ = w.request_user_attention(Some(UserAttentionType::Critical));
    let w2 = w.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(6));
        let _ = w2.set_always_on_top(false);
        let _ = w2.request_user_attention(None);
    });
    let _ = state; // mantém a assinatura
    if !estava_na_frente {
        let texto: String = format!("{} quer que você veja uma jogada agora!", nome.chars().take(40).collect::<String>());
        avisar(&app, "ACORDA! 🔔", &texto, false);
    }
}

#[tauri::command]
fn mostrar_janela(app: AppHandle) { mostrar(&app); }

#[tauri::command]
fn tocou(state: State<Shared>) {
    state.lock().unwrap().ultimo_toque = Some(Instant::now());
}

#[tauri::command]
fn aviso(titulo: String, texto: String, app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) { return; }
    }
    avisar(&app, &titulo, &texto, false);
}

#[tauri::command]
fn aparelho(state: State<Shared>) -> String {
    state.lock().unwrap().aparelho.clone()
}

// ---------- pop-up do chat ----------
#[tauri::command]
fn popup_chat(m: Value, app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        if w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) { return; }
    }
    if !m.is_object() { return; }
    let Some(p) = popup(&app) else { return };
    if !p.is_visible().unwrap_or(false) {
        posicionar_popup(&app, &p);
        let _ = p.show();
    }
    let _ = p.emit("mensagem", m);
}

#[tauri::command]
fn popup_vazio(app: AppHandle) {
    if let Some(p) = app.get_webview_window("popup") {
        let _ = p.hide();
    }
}

// ---------- painel de desempenho ----------
#[tauri::command]
fn desempenho(ligado: bool, app: AppHandle, state: State<Shared>) {
    {
        let mut est = state.lock().unwrap();
        est.desempenho_ativo = ligado;
    }
    if !ligado { return; }
    let app2 = app.clone();
    std::thread::spawn(move || {
        use sysinfo::{Pid, System};
        let mut sys = System::new_all();
        let eu = Pid::from_u32(std::process::id());
        loop {
            if !app2.state::<Shared>().lock().unwrap().desempenho_ativo { break; }
            sys.refresh_all();
            let (cpu_eu, ram_eu) = sys.process(eu).map(|p| (p.cpu_usage(), p.memory())).unwrap_or((0.0, 0));
            let dados = json!({
                // TODO: GPU ainda não medida (no Electron vinha do astra-audio "desempenho").
                "app": { "cpu": cpu_eu.round(), "ram": ram_eu / 1_048_576, "gpu": null, "ramGpu": 0 },
                "pc": {
                    "cpu": sys.global_cpu_usage().round(),
                    "ramUsada": sys.used_memory() / 1_048_576,
                    "ramTotal": sys.total_memory() / 1_048_576,
                    "gpu": null
                }
            });
            let _ = app2.emit_to("main", "desempenho-dados", dados);
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

// ---------- sair / reiniciar ----------
fn parar_tudo(est: &mut Estado) {
    parar_capturador(est);
    parar_teclas(est);
    ligar_silencio_interno(est, false);
}

fn sair(app: &AppHandle) {
    {
        let state = app.state::<Shared>();
        let mut est = state.lock().unwrap();
        est.saindo = true;
        parar_tudo(&mut est);
    }
    app.exit(0);
}

#[tauri::command]
fn reiniciar(app: AppHandle) {
    let _ = app.restart();
}

// ---------- montagem ----------
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| mostrar(app)))
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--oculto"])))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Mutex::new(Estado::default()))
        .invoke_handler(tauri::generate_handler![
            conferir_modulo, atualizar, listar_fontes, definir_fonte, transmitindo, som_iniciar, som_parar,
            silencio, teclas, risco, tecla_emergencia, janela, proteger_tela, gravador_nvidia, relaxado,
            acorde, mostrar_janela, tocou, aviso, aparelho, popup_chat, popup_vazio, desempenho, reiniciar,
        ])
        .on_window_event(|janela, evento| match evento {
            WindowEvent::CloseRequested { api, .. } if janela.label() == "main" => {
                // Fechar só esconde: o programa continua perto do relógio.
                api.prevent_close();
                let _ = janela.hide();
                if !AVISOU_SEGUNDO_PLANO.swap(true, Ordering::Relaxed) {
                    let _ = janela.app_handle().notification().builder()
                        .title("AstraScreenShare continua ligado")
                        .body("Ele fica aqui perto do relógio. Para fechar de vez, clique com o botão direito no ícone e em Sair.")
                        .show();
                }
            }
            WindowEvent::Resized(_) if janela.label() == "main" => {
                let maximizada = janela.is_maximized().unwrap_or(false);
                let _ = janela.emit("janela-max", maximizada);
            }
            _ => {}
        })
        .setup(|app| {
            let handle = app.handle().clone();
            let oculto = std::env::args().any(|a| a == "--oculto");
            let aparelho = ler_aparelho();
            {
                let state = app.state::<Shared>();
                state.lock().unwrap().aparelho = aparelho.clone();
            }
            let boot = json!({
                "servidor": servidor(),
                "site": site(),
                "versao": VERSAO,
                "aparelho": aparelho,
                "semMouse": false,   // o filtro de mouse do Electron não se aplica ao WebView2
            });
            criar_janela_principal(&handle, &boot, oculto)?;
            let menu = montar_menu(&handle, &Estado::default())?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().ok_or("sem ícone")?.clone())
                .tooltip("AstraScreenShare")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, evento| acao_menu(app, evento.id().as_ref()))
                .on_tray_icon_event(|tray, evento| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = evento {
                        mostrar(tray.app_handle());
                    }
                })
                .build(app)?;
            if !oculto { primeira_vez_iniciar(&handle); }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("erro ao montar o AstraScreenShare")
        .run(|app, evento| {
            if let RunEvent::Exit = evento {
                let state = app.state::<Shared>();
                let mut est = state.lock().unwrap();
                parar_tudo(&mut est);
            }
        });
}

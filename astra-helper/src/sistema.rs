// Funções do sistema usadas pelos modos do ajudante (sem áudio):
//  - nomes dos processos e pid de uma janela;
//  - "Não perturbe" (desliga as notificações pop-up durante a live);
//  - teclas de jogo (só uma lista fixa, sem gancho de teclado global);
//  - uso da GPU por processo (contadores do Windows, PDH).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{BOOL, CloseHandle, HWND};
use windows::Win32::System::Console::SetConsoleCtrlHandler;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

// Quando o programa fecha o stdin, o ajudante encerra (silencio e teclas).
static PARAR: AtomicBool = AtomicBool::new(false);

fn vigiar_stdin() {
    thread::spawn(|| {
        let mut buf = [0u8; 64];
        let mut entrada = std::io::stdin();
        loop {
            match entrada.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
        PARAR.store(true, Ordering::SeqCst);
    });
}

// ---------- processos ----------

/// Mapa pid -> nome do executável (em minúsculas), a partir de um instantâneo do sistema.
pub fn nomes_de_processos() -> HashMap<u32, String> {
    let mut mapa = HashMap::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return mapa;
        };
        let mut entrada: PROCESSENTRY32W = std::mem::zeroed();
        entrada.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut entrada).is_ok() {
            loop {
                let nome = String::from_utf16_lossy(&entrada.szExeFile[..])
                    .trim_end_matches('\0')
                    .to_lowercase();
                mapa.insert(entrada.th32ProcessID, nome);
                if Process32NextW(snap, &mut entrada).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    mapa
}

/// Pid do processo dono de uma janela (0 se a janela não existir).
pub fn pid_da_janela(hwnd: isize) -> u32 {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid));
    }
    pid
}

// ---------- Não perturbe ----------
// Desliga os avisos pop-up do Windows enquanto a live roda e devolve o valor anterior ao sair.
// Usa a chave de notificações do Windows (NOC_GLOBAL_SETTING_TOASTS_ENABLED).
// ATENÇÃO: é a chave de "notificações pop-up", não exatamente o "Foco de atenção" do Windows 11.

const CHAVE_NOTIF: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Notifications\Settings";
const VALOR_NOTIF: &str = "NOC_GLOBAL_SETTING_TOASTS_ENABLED";

fn ler_notificacoes() -> Option<u32> {
    let saida = Command::new("reg")
        .args(["query", CHAVE_NOTIF, "/v", VALOR_NOTIF])
        .output()
        .ok()?;
    if !saida.status.success() {
        return None;
    }
    let texto = String::from_utf8_lossy(&saida.stdout);
    let linha = texto.lines().find(|l| l.contains(VALOR_NOTIF))?;
    let valor = linha.split_whitespace().last()?;
    u32::from_str_radix(valor.trim_start_matches("0x"), 16).ok()
}

fn gravar_notificacoes(valor: u32) -> bool {
    Command::new("reg")
        .args(["add", CHAVE_NOTIF, "/v", VALOR_NOTIF, "/t", "REG_DWORD", "/d", &valor.to_string(), "/f"])
        .output()
        .map(|s| s.status.success())
        .unwrap_or(false)
}

fn apagar_notificacoes() {
    let _ = Command::new("reg")
        .args(["delete", CHAVE_NOTIF, "/v", VALOR_NOTIF, "/f"])
        .output();
}

// Arquivo com o valor de antes da live (em %LOCALAPPDATA%\\astra-helper). Se o helper morrer
// antes de restaurar, a próxima execução devolve o valor guardado.
fn arquivo_backup() -> std::path::PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    let pasta = std::path::Path::new(&base).join("astra-helper");
    let _ = std::fs::create_dir_all(&pasta);
    pasta.join("notif-anterior.txt")
}

fn restaurar_do_backup() {
    let arquivo = arquivo_backup();
    if let Ok(texto) = std::fs::read_to_string(&arquivo) {
        match texto.trim() {
            "none" => apagar_notificacoes(),
            v => {
                if let Ok(n) = v.parse::<u32>() {
                    gravar_notificacoes(n);
                }
            }
        }
        let _ = std::fs::remove_file(&arquivo);
    }
}

unsafe extern "system" fn ao_ctrl_c(_tipo: u32) -> BOOL {
    PARAR.store(true, Ordering::SeqCst);
    BOOL(1)
}

pub fn silencio() -> Result<(), String> {
    // Se a execução anterior foi interrompida, desfaz o que ela deixou ligado.
    restaurar_do_backup();

    let antes = ler_notificacoes();
    let arquivo = arquivo_backup();
    let guardado = match antes {
        Some(v) => v.to_string(),
        None => "none".to_string(),
    };
    std::fs::write(&arquivo, &guardado).map_err(|e| e.to_string())?;
    if !gravar_notificacoes(0) {
        let _ = std::fs::remove_file(&arquivo);
        return Err("este Windows nao tem o Nao perturbe".into());
    }
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(ao_ctrl_c), true);
    }
    eprintln!("OK silencio {}", antes.unwrap_or(1));
    vigiar_stdin();
    while !PARAR.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(100));
    }
    // Volta ao valor de antes da live.
    restaurar_do_backup();
    Ok(())
}

// ---------- teclas de jogo ----------
// Não usa gancho de teclado: consulta só uma lista fixa de teclas, a cada 15 ms.
// Letras fora de WASD, números, símbolos e teclado numérico nunca são lidos.

fn e_tecla_de_jogo(vk: i32) -> bool {
    matches!(
        vk,
        0x57 | 0x41 | 0x53 | 0x44      // W A S D
        | 0x25..=0x28                   // setas
        | 0x10 | 0xA0 | 0xA1            // Shift
        | 0x11 | 0xA2 | 0xA3            // Ctrl
        | 0x12 | 0xA4 | 0xA5            // Alt
        | 0x09 | 0x0D | 0x08 | 0x1B     // Tab, Enter, Backspace, Esc
        | 0x20 | 0x14                   // Espaço, Caps
        | 0x21..=0x24                   // PgUp, PgDn, End, Home
        | 0x2D | 0x2E                   // Ins, Del
        | 0x70..=0x87                   // F1 a F24
    )
}

pub fn teclas() -> Result<(), String> {
    vigiar_stdin();
    eprintln!("OK teclas");
    let mut anterior = [false; 256];
    let stdout = std::io::stdout();
    while !PARAR.load(Ordering::SeqCst) {
        let mut saida = stdout.lock();
        for vk in 0..256i32 {
            if !e_tecla_de_jogo(vk) {
                continue;
            }
            let baixo = unsafe { (GetAsyncKeyState(vk) as u16) & 0x8000 != 0 };
            if baixo != anterior[vk as usize] {
                anterior[vk as usize] = baixo;
                let tipo = if baixo { "D" } else { "U" };
                let _ = writeln!(saida, "{tipo} {vk}");
            }
        }
        let _ = saida.flush();
        drop(saida);
        thread::sleep(Duration::from_millis(15));
    }
    Ok(())
}

// ---------- desempenho (GPU) ----------
// Lê "\GPU Engine(*)\Utilization Percentage" (PDH) e soma por processo.
// O nome de cada instância tem o formato "pid_<pid>_luid_..._engtype_...".

const PDH_MAIS_DADOS: u32 = 0x800007D2;

fn pid_do_nome_da_instancia(nome: &str) -> Option<u32> {
    nome.strip_prefix("pid_")?.split('_').next()?.parse().ok()
}

pub fn desempenho() -> Result<(), String> {
    unsafe {
        // Os tipos de handle do PDH são inferidos pela própria chamada da API.
        let mut consulta = std::mem::zeroed();
        if PdhOpenQueryW(PCWSTR::null(), 0, &mut consulta) != 0 {
            return Err("não deu para abrir o contador de GPU".into());
        }
        let mut contador = std::mem::zeroed();
        if PdhAddEnglishCounterW(consulta, w!("\\GPU Engine(*)\\Utilization Percentage"), 0, &mut contador) != 0 {
            return Err("contador de GPU indisponível neste Windows".into());
        }
        PdhCollectQueryData(consulta);
        eprintln!("OK desempenho");

        let stdout = std::io::stdout();
        loop {
            thread::sleep(Duration::from_secs(1));
            PdhCollectQueryData(consulta);

            // 1ª chamada: descobre o tamanho do buffer. 2ª: lê os dados.
            let mut tamanho = 0u32;
            let mut quantidade = 0u32;
            let st = PdhGetFormattedCounterArrayW(contador, PDH_FMT_DOUBLE, &mut tamanho, &mut quantidade, None);
            if st != PDH_MAIS_DADOS && st != 0 {
                continue;
            }
            // Buffer de u64 para ficar alinhado (a estrutura do PDH precisa de alinhamento de ponteiro).
            let mut buf = vec![0u64; (tamanho as usize + 7) / 8 + 1];
            let st = PdhGetFormattedCounterArrayW(
                contador,
                PDH_FMT_DOUBLE,
                &mut tamanho,
                &mut quantidade,
                Some(buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W),
            );
            if st != 0 {
                continue;
            }
            let itens = std::slice::from_raw_parts(
                buf.as_ptr() as *const PDH_FMT_COUNTERVALUE_ITEM_W,
                quantidade as usize,
            );

            let mut total = 0f64;
            let mut por_pid: HashMap<u32, f64> = HashMap::new();
            for item in itens {
                let nome = item.szName.to_string().unwrap_or_default();
                let valor = item.FmtValue.Anonymous.doubleValue;
                total += valor;
                if let Some(pid) = pid_do_nome_da_instancia(&nome) {
                    *por_pid.entry(pid).or_default() += valor;
                }
            }
            let partes: Vec<String> = por_pid
                .iter()
                .filter(|(_, v)| **v >= 0.5)
                .map(|(pid, v)| format!("{}:{}", pid, v.round().min(100.0) as u32))
                .collect();

            let mut saida = stdout.lock();
            let _ = writeln!(saida, "GPU {} {}", total.min(100.0).round() as u32, partes.join(" "));
            let _ = saida.flush();
        }
    }
}

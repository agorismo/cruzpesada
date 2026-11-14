// Captura de som por processo (WASAPI "process loopback", Windows 10 versão 2004 ou mais novo).
//
//  - processo(pid, incluir): só o som dessa árvore de processos (incluir) ou todo o som
//    menos essa árvore (excluir). Usado em "excluir-pid" (menos o próprio Astra) e "incluir-janela".
//  - mixar(reserva, excluir_nomes): som de todas as sessões de áudio do PC que não sejam do próprio
//    Astra (reserva) nem de programas da lista (ex.: Discord). Cada sessão vira um cliente de
//    loopback por processo, e os sons são somados aqui.
//
// Saída: PCM 16 bits, 48 kHz, estéreo, intercalado (L, R, L, R...) em stdout.
// Antes de começar, escreve "OK ..." no stderr (o main.js espera essa linha).

use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use windows::core::{implement, Interface, IUnknown, Result as WinResult, GUID, HRESULT, PCWSTR};
use windows::Win32::Media::Audio::{
    eConsole, eRender, IActivateAudioInterfaceAsyncOperation, IActivateAudioInterfaceCompletionHandler,
    IActivateAudioInterfaceCompletionHandler_Impl, IAudioCaptureClient, IAudioClient, IAudioSessionControl2,
    IAudioSessionManager2, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
};

use crate::sistema;

// Função do mmdevapi.dll que ativa um cliente de áudio para um processo (Windows 10 2004+).
// Declarada aqui com ponteiros brutos para não depender de tipos PROPVARIANT da crate windows.
#[link(name = "mmdevapi")]
extern "system" {
    fn ActivateAudioInterfaceAsync(
        deviceinterfacepath: PCWSTR,
        riid: *const GUID,
        activationparams: *const c_void,
        completionhandler: *mut c_void,
        activationoperation: *mut *mut c_void,
    ) -> HRESULT;
}

const VAD_PROCESS_LOOPBACK: &str = "VAD\\Process_Loopback";
const VT_BLOB: u16 = 65;
const TIPO_PROCESS_LOOPBACK: u32 = 1;
const MODO_INCLUIR_ARVORE: u32 = 0;
const MODO_EXCLUIR_ARVORE: u32 = 1;
const ERRO_NAO_SUPORTADO: i32 = 0x8007_0032u32 as i32;
const SINAL_SILENCIO: u32 = 0x2; // AUDCLNT_BUFFERFLAGS_SILENT
const QUADRO_MS: u64 = 10;
const AMOSTRAS_POR_QUADRO: usize = 480 * 2; // 10 ms estéreo a 48 kHz
const MAX_BUFFER_CLIENTE: usize = 48000 * 2; // no máximo 1 s guardado por cliente

// AUDIOCLIENT_ACTIVATION_PARAMS com o bloco de loopback de processo.
#[repr(C)]
struct ParametrosLoopback {
    tipo: u32,
    pid: u32,
    modo: u32,
}

// PROPVARIANT do tipo VT_BLOB, no layout de 24 bytes (x64) que a API espera.
#[repr(C)]
struct PropVariantBlob {
    vt: u16,
    r1: u16,
    r2: u16,
    r3: u16,
    cb: u32,
    _pad: u32,
    ptr: *mut u8,
}

// Recebe o resultado assíncrono da ativação do cliente de áudio.
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Concluido {
    tx: Mutex<Option<mpsc::Sender<Result<IAudioClient, String>>>>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for Concluido_Impl {
    fn ActivateCompleted(&self, operacao: Option<&IActivateAudioInterfaceAsyncOperation>) -> WinResult<()> {
        let resultado = resultado_da_ativacao(operacao);
        if let Some(tx) = self.tx.lock().unwrap().take() {
            let _ = tx.send(resultado);
        }
        Ok(())
    }
}

fn resultado_da_ativacao(operacao: Option<&IActivateAudioInterfaceAsyncOperation>) -> Result<IAudioClient, String> {
    let operacao = operacao.ok_or("sem resposta do Windows")?;
    let mut hr = HRESULT(0);
    let mut interface: Option<IUnknown> = None;
    unsafe { operacao.GetActivateResult(&mut hr, &mut interface) }.map_err(traduzir)?;
    hr.ok().map_err(traduzir)?;
    let interface = interface.ok_or("sem interface de áudio")?;
    interface.cast::<IAudioClient>().map_err(traduzir)
}

fn traduzir(e: windows::core::Error) -> String {
    if e.code().0 == ERRO_NAO_SUPORTADO {
        return "Windows antigo: precisa do Windows 10 versao 2004 ou mais novo".into();
    }
    format!("{e}")
}

/// Cria um cliente de áudio ligado a um processo (ou à árvore dele).
fn ativar(pid: u32, modo: u32) -> Result<IAudioClient, String> {
    let params = ParametrosLoopback { tipo: TIPO_PROCESS_LOOPBACK, pid, modo };
    let pv = PropVariantBlob {
        vt: VT_BLOB,
        r1: 0,
        r2: 0,
        r3: 0,
        cb: std::mem::size_of::<ParametrosLoopback>() as u32,
        _pad: 0,
        ptr: &params as *const ParametrosLoopback as *mut u8,
    };
    let dispositivo: Vec<u16> = VAD_PROCESS_LOOPBACK.encode_utf16().chain(Some(0)).collect();
    let (tx, rx) = mpsc::channel();
    let concluido: IActivateAudioInterfaceCompletionHandler = Concluido { tx: Mutex::new(Some(tx)) }.into();

    let mut operacao: *mut c_void = std::ptr::null_mut();
    let hr = unsafe {
        ActivateAudioInterfaceAsync(
            PCWSTR(dispositivo.as_ptr()),
            &IAudioClient::IID,
            &pv as *const PropVariantBlob as *const c_void,
            concluido.as_raw(),
            &mut operacao,
        )
    };
    if !operacao.is_null() {
        // Libera a referência devolvida pela API (não precisamos dela depois).
        drop(unsafe { IUnknown::from_raw(operacao) });
    }
    hr.ok().map_err(traduzir)?;

    // params e pv precisam continuar vivos até a resposta chegar.
    let resultado = rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| "o Windows demorou para abrir o som do programa".to_string())?;
    drop(params);
    resultado
}

/// Configura o cliente em PCM 16 bits, 48 kHz, estéreo, e começa a capturar.
fn iniciar(cliente: &IAudioClient) -> Result<IAudioCaptureClient, String> {
    let formato = WAVEFORMATEX {
        wFormatTag: 1,
        nChannels: 2,
        nSamplesPerSec: 48000,
        nAvgBytesPerSec: 48000 * 4,
        nBlockAlign: 4,
        wBitsPerSample: 16,
        cbSize: 0,
    };
    unsafe {
        cliente.Initialize(
            AUDCLNT_SHAREMODE_SHARED,
            AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
            0,
            0,
            &formato,
            None,
        )
    }
    .map_err(traduzir)?;
    let captura: IAudioCaptureClient = unsafe { cliente.GetService() }.map_err(traduzir)?;
    unsafe { cliente.Start() }.map_err(traduzir)?;
    Ok(captura)
}

/// Lê tudo o que chegou e acrescenta as amostras (i16, intercaladas) em `saida`.
fn ler(captura: &IAudioCaptureClient, saida: &mut Vec<i16>) -> Result<(), String> {
    loop {
        let pacote = unsafe { captura.GetNextPacketSize() }.map_err(traduzir)?;
        if pacote == 0 {
            return Ok(());
        }
        let mut dados: *mut u8 = std::ptr::null_mut();
        let mut quadros = 0u32;
        let mut sinalizadores = 0u32;
        unsafe { captura.GetBuffer(&mut dados, &mut quadros, &mut sinalizadores, None, None) }
            .map_err(traduzir)?;
        let n = quadros as usize * 2;
        if sinalizadores & SINAL_SILENCIO != 0 || dados.is_null() {
            saida.extend(std::iter::repeat(0i16).take(n));
        } else {
            let amostras = unsafe { std::slice::from_raw_parts(dados as *const i16, n) };
            saida.extend_from_slice(amostras);
        }
        unsafe { captura.ReleaseBuffer(quadros) }.map_err(traduzir)?;
    }
}

fn escrever_amostras(saida: &mut impl Write, amostras: &[i16]) -> std::io::Result<()> {
    let mut bytes = Vec::with_capacity(amostras.len() * 2);
    for a in amostras {
        bytes.extend_from_slice(&a.to_le_bytes());
    }
    saida.write_all(&bytes)?;
    saida.flush()
}

/// Um processo (ou árvore) só, sem mistura: o som sai direto em stdout.
pub fn processo(pid: u32, incluir: bool) -> Result<(), String> {
    let modo = if incluir { MODO_INCLUIR_ARVORE } else { MODO_EXCLUIR_ARVORE };
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok().map_err(traduzir)?;
    let cliente = ativar(pid, modo)?;
    let captura = iniciar(&cliente)?;
    eprintln!("OK processo {pid}");
    let mut saida = std::io::stdout().lock();
    let mut quadro = Vec::new();
    loop {
        quadro.clear();
        ler(&captura, &mut quadro)?;
        if !quadro.is_empty() {
            escrever_amostras(&mut saida, &quadro).map_err(|e| e.to_string())?;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

// ---------- mistura de várias sessões ----------

struct Canal {
    buffer: Arc<Mutex<VecDeque<i16>>>,
    parar: Arc<AtomicBool>,
}

fn canal(pid: u32, buffer: Arc<Mutex<VecDeque<i16>>>, parar: Arc<AtomicBool>) {
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok();
    let Ok(cliente) = ativar(pid, MODO_INCLUIR_ARVORE) else {
        unsafe { CoUninitialize() };
        return;
    };
    let Ok(captura) = iniciar(&cliente) else {
        unsafe { CoUninitialize() };
        return;
    };
    let mut quadro = Vec::new();
    while !parar.load(Ordering::SeqCst) {
        quadro.clear();
        if ler(&captura, &mut quadro).is_ok() && !quadro.is_empty() {
            let mut b = buffer.lock().unwrap();
            b.extend(quadro.iter().copied());
            while b.len() > MAX_BUFFER_CLIENTE {
                b.pop_front();
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    unsafe {
        let _ = cliente.Stop();
        CoUninitialize();
    }
}

/// Pids das sessões de áudio que estão tocando agora no dispositivo padrão.
fn pids_com_audio() -> Result<Vec<u32>, String> {
    let mut pids = Vec::new();
    unsafe {
        let enumerador: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(traduzir)?;
        let dispositivo = enumerador.GetDefaultAudioEndpoint(eRender, eConsole).map_err(traduzir)?;
        let gerente: IAudioSessionManager2 = dispositivo.Activate(CLSCTX_ALL, None).map_err(traduzir)?;
        let lista = gerente.GetSessionEnumerator().map_err(traduzir)?;
        let total = lista.GetCount().map_err(traduzir)?;
        for i in 0..total {
            let controle = lista.GetSession(i).map_err(traduzir)?;
            let controle2: IAudioSessionControl2 = controle.cast().map_err(traduzir)?;
            if let Ok(pid) = controle2.GetProcessId() {
                if pid != 0 {
                    pids.push(pid);
                }
            }
        }
    }
    pids.sort_unstable();
    pids.dedup();
    Ok(pids)
}

/// Mistura o som de todos os programas com áudio, menos o próprio Astra e os da lista de exclusão.
pub fn mixar(reserva: u32, excluir_nomes: &[String]) -> Result<(), String> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok().map_err(traduzir)?;
    let excluir: Vec<String> = excluir_nomes.iter().map(|n| n.to_lowercase()).collect();
    let mut canais: HashMap<u32, Canal> = HashMap::new();

    // Atualiza os clientes: novos programas entram, programas que pararam de tocar saem.
    let atualizar = |canais: &mut HashMap<u32, Canal>| -> Result<usize, String> {
        let nomes = sistema::nomes_de_processos();
        let desejados: Vec<u32> = pids_com_audio()?
            .into_iter()
            .filter(|&pid| pid != reserva && pid != std::process::id())
            .filter(|pid| nomes.get(pid).map_or(true, |nome| !excluir.iter().any(|e| e == nome)))
            .collect();
        canais.retain(|pid, c| {
            let manter = desejados.contains(pid);
            if !manter {
                c.parar.store(true, Ordering::SeqCst);
            }
            manter
        });
        for pid in desejados {
            canais.entry(pid).or_insert_with(|| {
                let buffer = Arc::new(Mutex::new(VecDeque::new()));
                let parar = Arc::new(AtomicBool::new(false));
                let (b, p) = (buffer.clone(), parar.clone());
                thread::spawn(move || canal(pid, b, p));
                Canal { buffer, parar }
            });
        }
        Ok(canais.len())
    };

    let quantos = atualizar(&mut canais)?;
    eprintln!("OK mixar {quantos} programas");

    let mut saida = std::io::stdout().lock();
    let mut ultima_lista = Instant::now();
    let mut mistura = vec![0i32; AMOSTRAS_POR_QUADRO];
    loop {
        if ultima_lista.elapsed() >= Duration::from_secs(2) {
            let _ = atualizar(&mut canais);
            ultima_lista = Instant::now();
        }
        mistura.iter_mut().for_each(|m| *m = 0);
        for c in canais.values() {
            let mut b = c.buffer.lock().unwrap();
            for m in mistura.iter_mut() {
                match b.pop_front() {
                    Some(v) => *m += v as i32,
                    None => break,
                }
            }
        }
        let quadro: Vec<i16> = mistura.iter().map(|&v| v.clamp(-32768, 32767) as i16).collect();
        escrever_amostras(&mut saida, &quadro).map_err(|e| e.to_string())?;
        thread::sleep(Duration::from_millis(QUADRO_MS));
    }
}

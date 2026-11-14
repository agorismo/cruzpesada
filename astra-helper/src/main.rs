// astra-audio (crate astra-helper): ajudante do AstraScreenShare, em Rust.
//
// Substitui o astra-audio.exe antigo (C++/MinGW) mantendo o mesmo protocolo que o main.js espera:
//   stderr: "OK <info>" quando está pronto; "ERRO <texto>" em caso de falha
//   stdout: PCM 16 bits, 48 kHz, estéreo (modos de som); "D <vk>" / "U <vk>" (teclas);
//           "GPU <total> pid:valor ..." (desempenho)
//
// Modos (e quem chama no main.js):
//   mixar <pid-do-astra> <nome>...   som da live sem o Discord (som-iniciar, modo padrão)
//   excluir-pid <pid>                todo o som do PC menos a árvore desse processo (modo "tudo")
//   incluir-pid <pid>                só o som dessa árvore de processos
//   incluir-janela <hwnd>            só o som da janela escolhida (modo "janela")
//   excluir-nome <nome> [pid]        mixar sem um programa pelo nome
//   silencio                         "Não perturbe" durante a live (silenciar até o stdin fechar)
//   teclas                           teclas de jogo apertadas (até o stdin fechar)
//   desempenho                       uso da GPU por processo, a cada 1 s

mod audio;
mod sistema;

use std::process::ExitCode;

const USO: &str = "uso: astra-audio mixar <pid> <nome>... | excluir-pid <pid> | incluir-pid <pid> | \
incluir-janela <hwnd> | excluir-nome <nome> [pid] | silencio | teclas | desempenho";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match executar(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ERRO {e}");
            ExitCode::FAILURE
        }
    }
}

fn executar(a: &[String]) -> Result<(), String> {
    let pid_em = |i: usize| -> Result<u32, String> {
        a.get(i)
            .ok_or_else(|| USO.to_string())?
            .parse()
            .map_err(|_| format!("valor invalido: {}", a[i]))
    };

    match a.first().map(String::as_str) {
        Some("mixar") => audio::mixar(pid_em(1)?, a.get(2..).unwrap_or(&[])),
        Some("excluir-pid") => audio::processo(pid_em(1)?, false),
        Some("incluir-pid") => audio::processo(pid_em(1)?, true),
        Some("incluir-janela") => {
            let hwnd: isize = a
                .get(1)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| USO.to_string())?;
            let pid = sistema::pid_da_janela(hwnd);
            if pid == 0 {
                return Err("janela não encontrada".into());
            }
            audio::processo(pid, true)
        }
        Some("excluir-nome") => {
            let nome = a.get(1).ok_or_else(|| USO.to_string())?.clone();
            let reserva = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            audio::mixar(reserva, &[nome])
        }
        Some("silencio") => sistema::silencio(),
        Some("teclas") => sistema::teclas(),
        Some("desempenho") => sistema::desempenho(),
        _ => Err(USO.to_string()),
    }
}

# AstraScreenShare (Tauri)

Versão do programa de PC sem Electron. Backend em Rust (`src-tauri/`), interface na pasta `web/`.

## Pré-requisitos (Windows)
- Rust: https://rustup.rs
- Tauri CLI: `cargo install tauri-cli --version "^2"`
- WebView2 Runtime (já vem no Windows 10/11 atualizado)

## Preparar
1. Coloque `astra-audio.exe` em `src-tauri/resources/`.
2. Gere os ícones: `cargo tauri icon web/icone-512.png` (cria `src-tauri/icons/`).

## Rodar e gerar o instalador
- Testar: `cargo tauri dev`
- Instalador: `cargo tauri build`

## Status
- Rascunho ainda não compilado. Erros de compilação são esperados e serão corrigidos.
- Captura de tela usa o seletor do sistema (getDisplayMedia do WebView2); a escolha dupla de tela precisa ser testada.
- Pendentes: miniaturas das telas, janelas específicas, mini tela, GPU no painel, cópia de migração.

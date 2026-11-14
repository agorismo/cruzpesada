# AstraScreenShare (Tauri)

Versão do programa de PC sem Electron. Backend em Rust (`src-tauri/`), interface na pasta `web/`.

## Pré-requisitos (Windows)
- Rust: https://rustup.rs
- Tauri CLI: `cargo install tauri-cli --version "^2"`
- WebView2 Runtime (já vem no Windows 10/11 atualizado)

## Build completo (recomendado)

Na pasta `astra-tauri`, no PowerShell:

```powershell
.\build.ps1
```

O script compila o ajudante, copia o `astra-audio.exe`, gera os ícones e gera o instalador.

## Preparar (manual)
1. Compile o ajudante (veja `astra-helper/README.md`): ele gera `astra-audio.exe` em `src-tauri/resources/`.
2. Gere os ícones: `cargo tauri icon web/icone-512.png` (cria `src-tauri/icons/`).

## Rodar e gerar o instalador
- Testar: `cargo tauri dev`
- Instalador: `cargo tauri build`

## Status
- Desempenho: CPU/RAM pelo sysinfo; GPU pelo ajudante `astra-audio desempenho`.
- Teclas e Não perturbe: pelo ajudante (ver `astra-helper/README.md`).
- Rascunho ainda não compilado. Erros de compilação são esperados e serão corrigidos.
- Captura de tela usa o seletor do sistema (getDisplayMedia do WebView2); a escolha dupla de tela precisa ser testada.
- Pendentes: miniaturas das telas, janelas específicas, mini tela, GPU no painel, cópia de migração.

## Compilar pelo GitHub (sem instalar nada no PC)

1. Suba o conteúdo desta pasta (`astra-tauri`) como a raiz de um repositório no GitHub.
2. Abra a aba **Actions** e espere o workflow **Build Windows** terminar (uns 15 a 25 minutos na primeira vez).
3. Abra a execução concluída e baixe o artefato **AstraScreenShare-instalador**.

Para rodar de novo sem fazer push, use **Run workflow** na mesma aba.

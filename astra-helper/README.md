# astra-helper (astra-audio.exe em Rust)

Ajudante do AstraScreenShare que faz o que o programa não faz sozinho:

| Modo | Para quê | Quem chama (main.rs / main.js) |
|------|----------|-------------------------------|
| `mixar <pid> <nomes...>` | Som da live sem o Discord | som-iniciar, modo padrão |
| `excluir-pid <pid>` | Todo o som do PC menos o do Astra | som-iniciar, modo "tudo" |
| `incluir-pid <pid>` | Som de uma árvore de processos | (uso futuro) |
| `incluir-janela <hwnd>` | Só o som de uma janela | som-iniciar, modo "janela" |
| `excluir-nome <nome> [pid]` | Mixar sem um programa pelo nome | (uso futuro) |
| `silencio` | "Não perturbe" durante a live | ligarSilencio |
| `teclas` | Teclas de jogo apertadas | ligarTeclas |
| `desempenho` | Uso da GPU por processo | ligarMedidorGpu |

## Protocolo (igual ao do executável antigo)

- **stderr:** `OK <info>` quando pronto; `ERRO <texto>` em falha.
- **stdout (som):** PCM 16 bits, 48 kHz, estéreo, intercalado.
- **stdout (teclas):** `D <código>` ao apertar e `U <código>` ao soltar (código de tecla virtual do Windows).
- **stdout (desempenho):** `GPU <total> <pid>:<valor> ...` a cada segundo.
- **silencio / teclas:** encerram quando o stdin fecha; o `silencio` restaura o valor anterior das notificações.

## Diferenças em relação ao original

- **Teclas:** sem gancho de teclado global. Só consulta uma lista fixa de teclas de jogo (WASD, setas,
  modificadores, Tab, Enter, Esc, Espaço, F1 a F24). Letras fora de WASD, números e símbolos nunca são lidos.
- **Não perturbe:** muda a chave de notificações pop-up do Windows (`NOC_GLOBAL_SETTING_TOASTS_ENABLED`).
  Não é exatamente o "Foco de atenção" do Windows 11; se for preciso, esse ajuste precisa ser investigado.
- **Mixar:** cada programa com áudio vira um cliente WASAPI de loopback por processo, e a mistura é feita aqui.

## Compilar (Windows, Rust + MSVC)

```powershell
cd astra-helper
cargo build --release
copy target\release\astra-audio.exe ..\src-tauri\resources\astra-audio.exe
```

Requer Windows 10 versão 2004 ou mais novo para o som por processo.

## Estado

Escrito sem compilação. As partes mais prováveis de precisar de ajuste no primeiro build:
- as assinaturas COM (`ActivateAudioInterfaceAsync`, `IAudioSessionManager2`, `GetBuffer`);
- o acesso ao valor do PDH (`FmtValue.Anonymous.doubleValue`);
- o tipo de `PROPVARIANT` (layout em `PropVariantBlob`).
Testar no Windows com a opção "Só o som da janela" e "Sem o Discord" antes de liberar.

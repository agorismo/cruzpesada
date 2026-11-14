# Gera o instalador do AstraScreenShare do zero.
# Uso (PowerShell, na pasta astra-tauri): .\build.ps1
# Pré-requisito único: cargo install tauri-cli --version "^2" --locked
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

function Executar($comando, $argumentos, $pasta) {
    Push-Location $pasta
    try {
        & $comando @argumentos
        if ($LASTEXITCODE -ne 0) { throw "falhou: $comando $($argumentos -join ' ') (código $LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
}

Write-Host "1/4 Compilando o ajudante (astra-audio.exe)..." -ForegroundColor Cyan
Executar cargo @("build", "--release") "astra-helper"
New-Item -ItemType Directory -Force src-tauri\resources | Out-Null
Copy-Item astra-helper\target\release\astra-audio.exe src-tauri\resources\ -Force

Write-Host "2/4 Ícones do programa..." -ForegroundColor Cyan
if (-not (Test-Path src-tauri\icons\icon.ico)) {
    Executar cargo @("tauri", "icon", "..\web\icone-512.png") "src-tauri"
}

Write-Host "3/4 Compilando o programa e o instalador..." -ForegroundColor Cyan
Executar cargo @("tauri", "build") "src-tauri"

Write-Host "4/4 Pronto. Instalador em: src-tauri\target\release\bundle\nsis\" -ForegroundColor Green

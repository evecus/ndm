# NDM-RS Windows 构建脚本
# 用法：在项目根目录执行  powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1

$ErrorActionPreference = 'Stop'

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host '==> 检查 Rust 工具链' -ForegroundColor Cyan
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Error '未找到 cargo，请先安装 rustup：https://rustup.rs'
}

$rustTarget = rustc -vV | Select-String 'host:' | ForEach-Object { ($_ -split ':\s*')[1] }
Write-Host "    host: $rustTarget"
if ($rustTarget -notmatch 'windows') {
    Write-Warning "当前默认目标不是 Windows（$rustTarget）。如需交叉编译，请安装对应 target 与链接器。"
}

Write-Host '==> cargo build --release' -ForegroundColor Cyan
cargo build --release
if ($LASTEXITCODE -ne 0) { Write-Error '编译失败' }

$exe = Join-Path $root 'target\release\ndm-rs.exe'
if (-not (Test-Path $exe)) { Write-Error "未找到产物：$exe" }

$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force -Path $dist | Out-Null
Copy-Item $exe (Join-Path $dist 'ndm-rs.exe') -Force
Copy-Item (Join-Path $root 'README.md') (Join-Path $dist 'README.md') -Force

$zip = Join-Path $root 'ndm-rs-windows-x64.zip'
if (Test-Path $zip) { Remove-Item $zip -Force }
Compress-Archive -Path (Join-Path $dist '*') -DestinationPath $zip -CompressionLevel Optimal

Write-Host ''
Write-Host "==> 完成" -ForegroundColor Green
Write-Host "    可执行文件: $dist\ndm-rs.exe"
Write-Host "    分发压缩包: $zip"
Write-Host ''
Write-Host '提示：官方扩展固定连接 ws://127.0.0.1:10007/download，请使用默认端口启动。'

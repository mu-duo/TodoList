<#
.SYNOPSIS
    TodoList 分发构建脚本：同步版本号、构建并收集发布产物到 release/。

.DESCRIPTION
    - 参数 -Version 必填，形如 1.0.0。
    - 参数 -SkipBuild 只做版本同步与校验，不构建、不收集产物。
    - 任一步失败都会打印原因并以非零码退出。
    - 幂等、无需管理员权限。

.EXAMPLE
    pwsh scripts/build-release.ps1 -Version 1.0.0
    pwsh scripts/build-release.ps1 -Version 1.0.0 -SkipBuild
#>
param(
    [Parameter(Mandatory = $true)]
    [string]$Version,

    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'

# 仓库根目录（本脚本位于 scripts/ 下）
$repoRoot    = Split-Path -Parent $PSScriptRoot
$packageJson  = Join-Path $repoRoot 'package.json'
$tauriConf    = Join-Path $repoRoot 'src-tauri\tauri.conf.json'
$cargoToml    = Join-Path $repoRoot 'src-tauri\Cargo.toml'
$targetDir    = Join-Path $repoRoot 'src-tauri\target\release'
$nsisDir      = Join-Path $targetDir 'bundle\nsis'
$releaseDir   = Join-Path $repoRoot 'release'

# 统一用 UTF-8（无 BOM）读写，避免中文内容与换行风格被改动
$utf8NoBom = New-Object System.Text.UTF8Encoding($false)

function Fail([string]$Message) {
    Write-Host "错误: $Message" -ForegroundColor Red
    exit 1
}

try {
    # ---------- 1. 校验版本格式 ----------
    if ($Version -notmatch '^\d+\.\d+\.\d+$') {
        Fail "-Version 必须形如 1.0.0（当前: '$Version'）"
    }

    Write-Host "==> 同步版本号到 $Version" -ForegroundColor Cyan

    # package.json：替换 "version": "..." 的值
    $pkg = [System.IO.File]::ReadAllText($packageJson, $utf8NoBom)
    $pkg = $pkg -replace '("version"\s*:\s*")[^"]*(")', "`${1}$Version`${2}"
    [System.IO.File]::WriteAllText($packageJson, $pkg, $utf8NoBom)

    # tauri.conf.json：替换 "version": "..." 的值
    $conf = [System.IO.File]::ReadAllText($tauriConf, $utf8NoBom)
    $conf = $conf -replace '("version"\s*:\s*")[^"]*(")', "`${1}$Version`${2}"
    [System.IO.File]::WriteAllText($tauriConf, $conf, $utf8NoBom)

    # Cargo.toml：仅替换 [package] 段内的 version
    $cargo = [System.IO.File]::ReadAllText($cargoToml, $utf8NoBom)
    $cargo = $cargo -replace '(?ms)(^\s*\[package\].*?^\s*version\s*=\s*")[^"]*(")', "`${1}$Version`${2}"
    [System.IO.File]::WriteAllText($cargoToml, $cargo, $utf8NoBom)

    # ---------- 2. 回读三处版本并打印确认一致 ----------
    $pkgVer   = [regex]::Match([System.IO.File]::ReadAllText($packageJson, $utf8NoBom), '"version"\s*:\s*"([^"]*)"').Groups[1].Value
    $confVer  = [regex]::Match([System.IO.File]::ReadAllText($tauriConf, $utf8NoBom),   '"version"\s*:\s*"([^"]*)"').Groups[1].Value
    $cargoVer = [regex]::Match([System.IO.File]::ReadAllText($cargoToml, $utf8NoBom),   '(?ms)^\s*\[package\].*?^\s*version\s*=\s*"([^"]*)"').Groups[1].Value

    Write-Host "    package.json          : $pkgVer"
    Write-Host "    src-tauri/tauri.conf.json : $confVer"
    Write-Host "    src-tauri/Cargo.toml  : $cargoVer"

    if ($pkgVer -ne $Version -or $confVer -ne $Version -or $cargoVer -ne $Version) {
        Fail "三处版本号未全部同步为 $Version（见上），请检查文件格式"
    }
    Write-Host "    三处版本一致 ✓" -ForegroundColor Green

    # ---------- 仅同步版本时到此结束 ----------
    if ($SkipBuild) {
        Write-Host "==> -SkipBuild：已完成版本同步与校验，跳过构建与产物收集" -ForegroundColor Yellow
        exit 0
    }

    # ---------- 3. 构建 ----------
    Write-Host "==> pnpm tauri build" -ForegroundColor Cyan
    Push-Location $repoRoot
    try {
        # pnpm 会把横幅（如 `$ tauri "build"`）写到 stderr；在 $ErrorActionPreference='Stop'
        # 下 PowerShell 会把它当成 NativeCommandError 抛出，导致构建明明成功也被判失败。
        # 因此这里临时放宽，只用退出码判定成败。
        $prevEap = $ErrorActionPreference
        $ErrorActionPreference = 'Continue'
        & pnpm tauri build 2>&1 | Out-Host
        $buildExit = $LASTEXITCODE
        $ErrorActionPreference = $prevEap
        if ($buildExit -ne 0) {
            Fail "pnpm tauri build 失败（退出码 $buildExit）"
        }
    }
    finally {
        Pop-Location
    }

    # ---------- 4. 收集产物 ----------
    Write-Host "==> 收集产物到 release/" -ForegroundColor Cyan
    if (-not (Test-Path -LiteralPath $releaseDir)) {
        New-Item -ItemType Directory -Path $releaseDir | Out-Null
    }

    $setupName = "TodoList_${Version}_x64-setup.exe"
    $bareName  = "TodoList_${Version}_x64.exe"
    $setupOut  = Join-Path $releaseDir $setupName
    $bareOut   = Join-Path $releaseDir $bareName

    # 先清空旧的同名产物（含校验文件），保证幂等
    foreach ($old in @($setupOut, "$setupOut.sha256", $bareOut, "$bareOut.sha256")) {
        if (Test-Path -LiteralPath $old) { Remove-Item -LiteralPath $old -Force }
    }

    # NSIS 安装器
    $installer = $null
    if (Test-Path -LiteralPath $nsisDir) {
        $installer = Get-ChildItem -LiteralPath $nsisDir -Filter '*-setup.exe' -File | Select-Object -First 1
    }
    if (-not $installer) {
        $candidates = @()
        if (Test-Path -LiteralPath $nsisDir) {
            $candidates = Get-ChildItem -LiteralPath $nsisDir -Filter '*.exe' -File | ForEach-Object { $_.FullName }
        }
        $list = if ($candidates.Count -gt 0) { $candidates -join "`n    " } else { "(目录不存在或其中无 .exe)" }
        Fail "未找到 NSIS 安装器（期望 $nsisDir\*-setup.exe）。实际候选:`n    $list"
    }
    Move-Item -LiteralPath $installer.FullName -Destination $setupOut -Force
    Write-Host "    $setupName" -ForegroundColor Green

    # 裸 exe（target/release 根层，排除 bundle 子目录）
    $bare = Get-ChildItem -LiteralPath $targetDir -Filter '*.exe' -File |
        Where-Object { $_.FullName -notlike '*\bundle\*' } |
        Select-Object -First 1
    if (-not $bare) {
        $candidates = Get-ChildItem -LiteralPath $targetDir -Filter '*.exe' -File -ErrorAction SilentlyContinue |
            ForEach-Object { $_.FullName }
        $list = if ($candidates.Count -gt 0) { $candidates -join "`n    " } else { "(目录不存在或其中无 .exe)" }
        Fail "未找到裸 exe（期望 $targetDir\*.exe）。实际候选:`n    $list"
    }
    Move-Item -LiteralPath $bare.FullName -Destination $bareOut -Force
    Write-Host "    $bareName" -ForegroundColor Green

    # ---------- 5. 生成 SHA256 校验文件 ----------
    foreach ($file in @($setupOut, $bareOut)) {
        $hash = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash.ToLower()
        "$hash  $([System.IO.Path]::GetFileName($file))" |
            Set-Content -LiteralPath "$file.sha256" -Encoding ascii
        Write-Host "    $([System.IO.Path]::GetFileName($file)).sha256" -ForegroundColor Green
    }

    # ---------- 6. 打印 release/ 清单 ----------
    Write-Host "==> release/ 内容:" -ForegroundColor Cyan
    Get-ChildItem -LiteralPath $releaseDir | Sort-Object Name | ForEach-Object {
        "{0,12}  {1}" -f $_.Length, $_.Name
    }

    Write-Host "==> 完成 ✓" -ForegroundColor Green
    exit 0
}
catch {
    Fail $_.Exception.Message
}

<#
    update-version.ps1 —— HypoMuxPlus 版本号统一更新脚本

    将指定的新版本号写入项目中所有涉及 HypoMuxPlus 版本号的位置：
      - package.json / package-lock.json（项目自身版本，不触碰第三方依赖版本）
      - src-tauri/Cargo.toml（[package] 版本）
      - src-tauri/tauri.conf.json（运行时版本，前端 useAppVersion 的权威来源）
      - src/lib/version.ts（离线回退版本，与 tauri.conf.json 保持一致）
      - README.md / README_EN.md（下载徽章与 Release 下载链接中的 vX.Y.Z）
      - website/index.html（官网下载链接与版本标签，若本地存在）

    设计要点：
      - 精确锁定「本项目版本号」，绝不误改第三方依赖版本（如 package-lock 里的依赖、Cargo.toml 里的依赖版本）。
      - 使用无 BOM 的 UTF-8 读写，避免破坏 JSON / 源码文件。
      - 幂等：旧版本 == 新版本时全部为空操作。
      - 支持 -DryRun 预演，不落盘。

    通常由「一键更新版本号.bat」调用，也可单独运行：
      powershell -NoProfile -ExecutionPolicy Bypass -File scripts/update-version.ps1 -NewVersion 1.2.0 [-DryRun]
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$NewVersion,

    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'

# 严格校验版本号格式 X.Y.Z
if ($NewVersion -notmatch '^[0-9]+\.[0-9]+\.[0-9]+$') {
    Write-Error "版本号格式无效: '$NewVersion'，必须为 X.Y.Z（例如 1.2.0）"
    exit 1
}

# 仓库根目录 = 本脚本所在 scripts 目录的上一级
$root = Split-Path -Parent $PSScriptRoot

# 从 tauri.conf.json 读取当前（旧）版本号，作为精确替换的锚点
$tauriConfPath = Join-Path $root 'src-tauri/tauri.conf.json'
if (-not (Test-Path -LiteralPath $tauriConfPath)) {
    Write-Error "未找到 tauri.conf.json：$tauriConfPath"
    exit 1
}
$tauriConfText = [IO.File]::ReadAllText($tauriConfPath)
$oldMatch = [regex]::Match($tauriConfText, '"version":\s*"([0-9]+\.[0-9]+\.[0-9]+)"')
if (-not $oldMatch.Success) {
    Write-Error "无法从 tauri.conf.json 解析出当前版本号"
    exit 1
}
$OldVersion = $oldMatch.Groups[1].Value

Write-Host ("当前版本: {0}  ->  目标版本: {1}" -f $OldVersion, $NewVersion) -ForegroundColor Cyan
if ($DryRun) { Write-Host "[DryRun] 仅预演，不写入文件" -ForegroundColor Yellow }
if ($OldVersion -eq $NewVersion) {
    Write-Host "新旧版本号一致，无需修改。" -ForegroundColor Yellow
}

$utf8NoBom = New-Object System.Text.UTF8Encoding($false)
$escOld = [regex]::Escape($OldVersion)

# 对单个文件应用一条正则替换；maxCount<=0 表示替换全部匹配
function Invoke-VersionReplace {
    param(
        [string]$RelPath,
        [string]$Pattern,
        [string]$Replacement,
        [int]$MaxCount = 0
    )
    $full = Join-Path $root $RelPath
    if (-not (Test-Path -LiteralPath $full)) {
        Write-Host ("  跳过 (不存在): {0}" -f $RelPath) -ForegroundColor DarkGray
        return
    }
    $text = [IO.File]::ReadAllText($full)
    $re = [regex]$Pattern
    $hitCount = $re.Matches($text).Count
    if ($hitCount -eq 0) {
        Write-Host ("  未命中: {0}  (模式: {1})" -f $RelPath, $Pattern) -ForegroundColor DarkYellow
        return
    }
    if ($MaxCount -gt 0) {
        $newText = $re.Replace($text, $Replacement, $MaxCount)
        $applied = [Math]::Min($hitCount, $MaxCount)
    } else {
        $newText = $re.Replace($text, $Replacement)
        $applied = $hitCount
    }
    if ($newText -ne $text -and -not $DryRun) {
        [IO.File]::WriteAllText($full, $newText, $utf8NoBom)
    }
    Write-Host ("  OK  {0}  (替换 {1} 处)" -f $RelPath, $applied) -ForegroundColor Green
}

Write-Host "`n开始更新版本号引用..." -ForegroundColor Cyan

# package.json —— 仅项目自身 "version"（顶层唯一）
Invoke-VersionReplace 'package.json' ('"version":\s*"' + $escOld + '"') ('"version": "' + $NewVersion + '"')

# package-lock.json —— 仅前两处（顶层 version 与 packages[""] .version），
# 绝不触碰其后的第三方依赖版本
Invoke-VersionReplace 'package-lock.json' ('"version":\s*"' + $escOld + '"') ('"version": "' + $NewVersion + '"') 2

# Cargo.toml —— 仅 [package] 段的行首 version（依赖版本不在行首，天然规避）
Invoke-VersionReplace 'src-tauri/Cargo.toml' ('(?m)^version\s*=\s*"' + $escOld + '"') ('version = "' + $NewVersion + '"') 1

# tauri.conf.json —— 运行时版本（productName 行独立，不受影响）
Invoke-VersionReplace 'src-tauri/tauri.conf.json' ('"version":\s*"' + $escOld + '"') ('"version": "' + $NewVersion + '"')

# version.ts —— 离线回退版本
Invoke-VersionReplace 'src/lib/version.ts' ('cached \|\| "' + $escOld + '"') ('cached || "' + $NewVersion + '"')

# README（中/英）—— 徽章与 Release 下载链接中的 vX.Y.Z
Invoke-VersionReplace 'README.md' ('v' + $escOld) ('v' + $NewVersion)
Invoke-VersionReplace 'README_EN.md' ('v' + $escOld) ('v' + $NewVersion)

# 官网源码（若本地存在）—— 下载链接与版本标签中的 vX.Y.Z
Invoke-VersionReplace 'website/index.html' ('v' + $escOld) ('v' + $NewVersion)

Write-Host "`n版本号引用更新完成。" -ForegroundColor Cyan
exit 0

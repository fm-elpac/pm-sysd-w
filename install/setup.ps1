# pm-sysd-w 安装/卸载脚本 (实验性).
#
# 用法 (在项目根目录运行):
#   powershell -ExecutionPolicy Bypass -File install\setup.ps1 install-user
#   powershell -ExecutionPolicy Bypass -File install\setup.ps1 uninstall-user
#
# install-user 会复制 3 个 exe 到 `%LOCALAPPDATA%\pm-sysd-w\bin\`,
# 加入当前用户 PATH, 写入空配置文件,
# 并把 `pm-sysd-init.exe --user --conf=...` 安装为 per-user 服务.
#
# uninstall-user 只删除 per-user 服务, 保留 `%LOCALAPPDATA%\pm-sysd-w\` 目录.
#
# 安装 / 删除服务需要管理员权限.

[CmdletBinding()]
param(
  [Parameter(Position = 0)]
  [string]$Command
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$script:ServiceName = 'pm_sysd_w_user'
$script:BinDirName = 'bin'

function Test-Admin {
  $id = [System.Security.Principal.WindowsIdentity]::GetCurrent()
  $p = New-Object System.Security.Principal.WindowsPrincipal($id)
  return $p.IsInRole([System.Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Get-BaseDir {
  $local = $env:LOCALAPPDATA
  if ([string]::IsNullOrWhiteSpace($local)) {
    throw '环境变量 LOCALAPPDATA 为空, 无法确定安装目录.'
  }
  return (Join-Path $local 'pm-sysd-w')
}

function Find-Exe {
  param([string]$Name)

  $candidates = @(
    (Join-Path $PWD.Path $Name),
    (Join-Path (Join-Path $PWD.Path 'target\release') $Name),
    (Join-Path $PSScriptRoot $Name),
    (Join-Path (Join-Path $PSScriptRoot '..\target\release') $Name)
  )

  foreach ($c in $candidates) {
    if (Test-Path -LiteralPath $c -PathType Leaf) {
      return $c
    }
  }
  return $null
}

function Copy-Exes {
  param([string]$BinDir)

  $names = @('pm-sysd-init.exe', 'pm-sysd-ctl.exe', 'pm-sysd-log.exe')
  New-Item -ItemType Directory -Force -Path $BinDir | Out-Null

  foreach ($n in $names) {
    $src = Find-Exe $n
    if (-not $src) {
      throw "未找到 $n, 请在项目根目录运行, 或先执行 cargo build --release --features cli_*."
    }
    $dst = Join-Path $BinDir $n
    Copy-Item -LiteralPath $src -Destination $dst -Force
    Write-Host "已复制 $src -> $dst"
  }
}

function Add-ToUserPath {
  param([string]$BinDir)

  $current = [Environment]::GetEnvironmentVariable('Path', 'User')
  if ($null -eq $current) {
    $current = ''
  }

  $found = $false
  foreach ($p in ($current -split ';')) {
    if ([string]::Equals($p.TrimEnd('\'), $BinDir.TrimEnd('\'), [System.StringComparison]::OrdinalIgnoreCase)) {
      $found = $true
      break
    }
  }

  if (-not $found) {
    $trimmed = $current.TrimEnd(';')
    if ($trimmed.Length -eq 0) {
      $new = $BinDir
    } else {
      $new = $trimmed + ';' + $BinDir
    }
    [Environment]::SetEnvironmentVariable('Path', $new, 'User')
    Write-Host "已把 $BinDir 加入当前用户 PATH."
  } else {
    Write-Host "$BinDir 已在当前用户 PATH 中."
  }
}

function Write-EmptyConfig {
  param([string]$ConfPath)

  if (Test-Path -LiteralPath $ConfPath) {
    Write-Host "配置文件已存在, 不覆盖: $ConfPath"
    return
  }

  $content = "# pm_sysd_w.conf.toml (空配置)`n" +
    "# 本文件由 install/setup.ps1 install-user 生成.`n" +
    "# 可按需添加 [[t.sysd]] unit 配置.`n" +
    "pm_tc = 1`n"
  [System.IO.File]::WriteAllText($ConfPath, $content)
  Write-Host "已写入空配置文件: $ConfPath"
}

function Service-Exists {
  param([string]$Name)
  return (Test-Path -LiteralPath ("HKLM:\SYSTEM\CurrentControlSet\Services\" + $Name))
}

function Install-Service {
  param([string]$BinDir, [string]$Name, [string]$ConfPath)

  if (Service-Exists $Name) {
    Write-Host "服务已存在: $Name (跳过创建)"
    return
  }

  $binPath = Join-Path $BinDir 'pm-sysd-init.exe'
  # sc.exe 的 binPath 值需要把 exe 路径与配置路径用引号包住, 并附加 --user / --conf 参数.
  $binArg = '"\"' + $binPath + '\" --user --conf=\"' + $ConfPath + '\""'
  $displayName = 'pm-sysd-w user service'
  $cmd = 'sc.exe create ' + $Name + ' type= userown start= auto binPath= ' + $binArg + ' DisplayName= "' + $displayName + '"'
  cmd.exe /c $cmd
  if ($LASTEXITCODE -ne 0) {
    throw "sc.exe create 失败, 退出码 $LASTEXITCODE"
  }
  Write-Host "已安装 per-user 服务: $Name (用户登录时自动启动 pm-sysd-init.exe --user --conf=...)"
}

function Uninstall-Service {
  param([string]$Name)

  if (-not (Service-Exists $Name)) {
    Write-Host "服务不存在: $Name (无需删除)"
    return
  }

  sc.exe delete $Name
  if ($LASTEXITCODE -ne 0) {
    throw "sc.exe delete 失败, 退出码 $LASTEXITCODE"
  }
  Write-Host "已删除服务: $Name"
}

if (-not (Test-Admin)) {
  Write-Host '错误: 此命令需要管理员权限 (安装 / 删除 per-user 服务). 请以管理员身份运行 PowerShell.'
  exit 1
}

switch ($Command) {
  'install-user' {
    $baseDir = Get-BaseDir
    $binDir = Join-Path $baseDir $script:BinDirName
    $confPath = Join-Path $baseDir 'pm_sysd_w.conf.toml'

    Copy-Exes $binDir
    Add-ToUserPath $binDir
    Write-EmptyConfig $confPath
    Install-Service $binDir $script:ServiceName $confPath

    Write-Host ''
    Write-Host '安装完成. 当前 PowerShell 的 PATH 尚未刷新, 请重新打开终端后再使用 pm-sysd-ctl / pm-sysd-log.'
  }
  'uninstall-user' {
    Uninstall-Service $script:ServiceName
    Write-Host ''
    Write-Host '卸载完成. %LOCALAPPDATA%\pm-sysd-w\ 目录已保留.'
  }
  default {
    Write-Host '用法:'
    Write-Host '  powershell -ExecutionPolicy Bypass -File install\setup.ps1 install-user'
    Write-Host '  powershell -ExecutionPolicy Bypass -File install\setup.ps1 uninstall-user'
    exit 1
  }
}

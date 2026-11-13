# pm-sysd-w 命名管道权限测试脚本 (手动运行).
#
# 需管理员运行; 需先编译二进制:
#   cargo build --features cli_init
#   cargo build --features cli_ctl
#
# 运行:
#   powershell -ExecutionPolicy Bypass -File test\test-permission.ps1

$ErrorActionPreference = 'Continue'

# 检查管理员权限.
$id = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($id)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Write-Error '请以管理员运行本脚本.'
    exit 1
}

$RepoRoot = Split-Path -Parent $PSScriptRoot
$binDir = Join-Path $RepoRoot 'target\debug'
if (-not (Test-Path (Join-Path $binDir 'pm-sysd-init.exe'))) {
    $binDir = Join-Path $RepoRoot 'target\release'
}
$initExe = Join-Path $binDir 'pm-sysd-init.exe'
$ctlExe = Join-Path $binDir 'pm-sysd-ctl.exe'
if (-not (Test-Path $initExe) -or -not (Test-Path $ctlExe)) {
    Write-Error '未找到 pm-sysd-init.exe / pm-sysd-ctl.exe, 请先编译.'
    exit 1
}

$MainPipe = 'pm_sysd_w_system_default'
$RoPipe = 'pm_sysd_w_system_default_ro'
$TestUser = 'pm_sysd_w_perm_test'
$TestPass = 'PmSysd#2026!'
$WorkDir = Join-Path $env:ProgramData 'pm_sysd_w_perm_test'

# 清理旧用户 / 旧工作目录.
Remove-LocalUser -Name $TestUser -ErrorAction SilentlyContinue
if (Test-Path $WorkDir) { Remove-Item $WorkDir -Recurse -Force }
New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null

$confPath = Join-Path $WorkDir 'pm_sysd_w.conf.toml'
@'
pm_tc = 1

[[t.sysd]]
"env:Unit" = "test.service"
"env:Service.ExecStart" = "cmd /c exit 0"
'@ | Set-Content -Path $confPath -Encoding ASCII

# 用于 非管理员 连接测试的 模拟登录 (LogonUser + 线程模拟).
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;

public static class Impersonation {
    [DllImport("advapi32.dll", SetLastError = true, CharSet = CharSet.Unicode)]
    public static extern bool LogonUser(
        string lpszUsername,
        string lpszDomain,
        string lpszPassword,
        int dwLogonType,
        int dwLogonProvider,
        out IntPtr phToken);

    [DllImport("kernel32.dll", SetLastError = true)]
    public static extern bool CloseHandle(IntPtr hObject);
}
'@

function Test-PipeConnect {
    param([string]$Pipe, [string]$Dir = 'InOut')

    $dirEnum = switch ($Dir) {
        'In'    { [System.IO.Pipes.PipeDirection]::In }
        'Out'   { [System.IO.Pipes.PipeDirection]::Out }
        default { [System.IO.Pipes.PipeDirection]::InOut }
    }

    try {
        $c = New-Object System.IO.Pipes.NamedPipeClientStream('.', $Pipe, $dirEnum)
        $c.Connect(3000)
        $c.Dispose()
        return 0
    } catch {
        return 1
    }
}

function Test-PipeConnectAs {
    param([PSCredential]$Cred, [string]$Pipe, [string]$Dir = 'InOut')

    $user = ($Cred.UserName -split '\\')[-1]
    $domain = if ($Cred.UserName -match '^(.+)\\(.+)$') { $Matches[1] } else { $env:COMPUTERNAME }
    $pass = $Cred.GetNetworkCredential().Password

    $token = [IntPtr]::Zero
    if (-not [Impersonation]::LogonUser($user, $domain, $pass, 2, 0, [ref]$token)) {
        throw "LogonUser 失败 (错误码 $([Runtime.InteropServices.Marshal]::GetLastWin32Error()))."
    }

    try {
        $ctx = [Security.Principal.WindowsIdentity]::Impersonate($token)
        try {
            return (Test-PipeConnect $Pipe $Dir)
        } finally {
            $ctx.Undo()
        }
    } finally {
        [Impersonation]::CloseHandle($token) | Out-Null
    }
}

# 启动 --system 实例 (当前管理员).
$init = Start-Process -FilePath $initExe -ArgumentList "--system", "--conf=$confPath" -PassThru -WindowStyle Hidden

$exitCode = 1
try {
    # 等待主管道可连接.
    $ready = $false
    for ($i = 0; $i -lt 40; $i++) {
        if ((Test-PipeConnect $MainPipe 'InOut') -eq 0) { $ready = $true; break }
        Start-Sleep -Milliseconds 250
    }
    if (-not $ready) { throw '等待主管道超时.' }

    # 创建临时标准用户 (非管理员).
    $sec = ConvertTo-SecureString $TestPass -AsPlainText -Force
    New-LocalUser -Name $TestUser -Password $sec -Description 'pm-sysd-w permission test' | Out-Null
    $cred = New-Object System.Management.Automation.PSCredential("$env:COMPUTERNAME\$TestUser", $sec)

    $results = @(
        [pscustomobject]@{ Test = 'admin main';    Pipe = $MainPipe; Expect = 0; Got = (Test-PipeConnect $MainPipe 'InOut') },
        [pscustomobject]@{ Test = 'admin ro';      Pipe = $RoPipe;   Expect = 0; Got = (Test-PipeConnect $RoPipe 'InOut') },
        [pscustomobject]@{ Test = 'nonadmin main'; Pipe = $MainPipe; Expect = 1; Got = (Test-PipeConnectAs $cred $MainPipe 'InOut') },
        [pscustomobject]@{ Test = 'nonadmin ro';   Pipe = $RoPipe;   Expect = 0; Got = (Test-PipeConnectAs $cred $RoPipe 'InOut') }
    )

    $fail = 0
    foreach ($r in $results) {
        $ok = ($r.Got -eq $r.Expect)
        if (-not $ok) { $fail++ }
        $status = if ($ok) { 'PASS' } else { 'FAIL' }
        Write-Output ("{0,-14} expect={1} got={2} -> {3}" -f $r.Test, $r.Expect, $r.Got, $status)
    }

    if ($fail -eq 0) {
        Write-Output 'ALL PASS'
        $exitCode = 0
    } else {
        Write-Output 'SOME FAIL'
    }
}
finally {
    if ($init -and -not $init.HasExited) {
        & $ctlExe --system exit >$null 2>&1
        Start-Sleep -Milliseconds 500
        if (-not $init.HasExited) { Stop-Process -Id $init.Id -Force -ErrorAction SilentlyContinue }
    }
    Remove-LocalUser -Name $TestUser -ErrorAction SilentlyContinue
    if (Test-Path $WorkDir) { Remove-Item $WorkDir -Recurse -Force -ErrorAction SilentlyContinue }
}

exit $exitCode

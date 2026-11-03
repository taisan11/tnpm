$ErrorActionPreference = 'Stop'
$dir = $env:TNPM_ALIAS_DIRECTORY
$action = $env:TNPM_ALIAS_ACTION
$cmdPath = Join-Path $dir 'npm.cmd'
$psPath = Join-Path $dir 'npm.ps1'
$statePath = Join-Path $dir '.tnpm-npm-alias.json'
$cmd = "@`"%~dp0tnpm.exe`" %*`r`n@exit /b %errorlevel%`r`n"
$ps = "& `"`$PSScriptRoot\tnpm.exe`" @args`r`nexit `$LASTEXITCODE`r`n"
$utf8 = New-Object System.Text.UTF8Encoding($false)

function Read-Text($path) {
    if ([IO.File]::Exists($path)) { return [IO.File]::ReadAllText($path) }
    return $null
}
function New-File($path, $content) {
    $stream = [IO.File]::Open($path, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $bytes = $utf8.GetBytes($content)
        $stream.Write($bytes, 0, $bytes.Length)
    } finally { $stream.Dispose() }
}
function Normalize-Entry($entry) {
    $entry = [Environment]::ExpandEnvironmentVariables($entry.Trim().Trim('"'))
    try { $entry = [IO.Path]::GetFullPath($entry) } catch { }
    return $entry.TrimEnd([char[]]@('\', '/'))
}
function Notify-Environment {
    try {
        Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class TnpmAliasEnvironment {
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr SendMessageTimeout(IntPtr window, uint msg, UIntPtr wparam,
        string lparam, uint flags, uint timeout, out UIntPtr result);
}
'@
        $result = [UIntPtr]::Zero
        [void][TnpmAliasEnvironment]::SendMessageTimeout([IntPtr]65535, 0x001A, [UIntPtr]::Zero, 'Environment', 0x0002, 1000, [ref]$result)
    } catch { Write-Warning 'Could not broadcast the PATH change; restart your terminal.' }
}

$key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
if ($null -eq $key) { throw 'Cannot open the user environment registry key.' }
try {
    $old = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    if ($action -eq 'create') {
        if ([IO.File]::Exists($statePath)) {
            if ((Read-Text $cmdPath) -cne $cmd -or (Read-Text $psPath) -cne $ps) { throw 'Existing tnpm npm alias files were modified; inspect them before continuing.' }
            Write-Host 'The tnpm npm alias already exists. Open a new terminal to use it.'
            return
        }
        if ((Test-Path -LiteralPath $cmdPath) -or (Test-Path -LiteralPath $psPath) -or (Test-Path -LiteralPath $statePath)) {
            throw 'npm.cmd, npm.ps1, or the alias state already exists in the tnpm directory; no files were overwritten.'
        }
        $entries = @($old -split ';' | Where-Object { $_ -ne '' })
        $normalized = Normalize-Entry $dir
        $remaining = @($entries | Where-Object { (Normalize-Entry $_) -ine $normalized })
        $newPath = (@($dir) + $remaining) -join ';'
        $kind = if ($null -eq $key.GetValue('Path', $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)) { 'Missing' } else { $key.GetValueKind('Path').ToString() }
        $state = @{ version = 1; originalPath = $old; installedPath = $newPath; originalKind = $kind } | ConvertTo-Json -Compress
        $created = @()
        try {
            New-File $cmdPath $cmd; $created += $cmdPath
            New-File $psPath $ps; $created += $psPath
            New-File $statePath $state; $created += $statePath
            $key.SetValue('Path', $newPath, [Microsoft.Win32.RegistryValueKind]::ExpandString)
        } catch {
            foreach ($file in $created) { [IO.File]::Delete($file) }
            throw
        }
        Notify-Environment
        Write-Host 'Created npm.cmd and npm.ps1 and moved the tnpm directory to the front of the user PATH.'
        Write-Host 'Open a new terminal. System PATH entries and shell aliases may still take priority.'
    } elseif ($action -eq 'delete') {
        if (-not [IO.File]::Exists($statePath)) {
            Write-Host 'No tnpm npm alias state found; nothing was changed.'
            return
        }
        $state = (Read-Text $statePath) | ConvertFrom-Json
        if ($state.version -ne 1 -or $null -eq $state.originalPath -or $null -eq $state.installedPath -or $state.originalKind -notin @('Missing', 'String', 'ExpandString')) { throw 'Invalid tnpm npm alias state; inspect it before continuing.' }
        foreach ($pair in @(@($cmdPath, $cmd), @($psPath, $ps))) {
            $content = Read-Text $pair[0]
            if ($null -ne $content -and $content -cne $pair[1]) { throw "Alias file was modified: $($pair[0]); nothing was removed." }
        }
        if ($old -ceq $state.installedPath) {
            if ($state.originalKind -eq 'Missing') { $key.DeleteValue('Path', $false) }
            else {
                $kind = [Enum]::Parse([Microsoft.Win32.RegistryValueKind], [string]$state.originalKind)
                $key.SetValue('Path', [string]$state.originalPath, $kind)
            }
        } else { Write-Warning 'User PATH changed since alias creation; leaving it unchanged.' }
        foreach ($file in @($cmdPath, $psPath, $statePath)) { if ([IO.File]::Exists($file)) { [IO.File]::Delete($file) } }
        Notify-Environment
        Write-Host 'Removed the tnpm npm alias. Open a new terminal.'
    } else { throw 'Invalid alias action.' }
} finally { $key.Close() }

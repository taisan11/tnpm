# Installs the latest stable tnpm release for Windows x64 without admin rights.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not [Environment]::Is64BitOperatingSystem) {
    throw 'tnpm currently provides a Windows x64 executable only.'
}
if ([string]::IsNullOrWhiteSpace($env:LOCALAPPDATA)) {
    throw 'LOCALAPPDATA is not set.'
}

$asset = 'tnpm-x86_64-pc-windows-msvc.exe'
$base = 'https://github.com/taisan11/tnpm/releases/latest/download'
$installDir = if ([string]::IsNullOrWhiteSpace($env:TNPM_INSTALL_DIR)) {
    Join-Path $env:LOCALAPPDATA 'Programs\tnpm'
} else {
    [System.IO.Path]::GetFullPath($env:TNPM_INSTALL_DIR)
}
$installDir = [System.IO.Path]::GetFullPath($installDir)
$tempDir = Join-Path ([System.IO.Path]::GetTempPath()) ('tnpm-install-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tempDir -Force | Out-Null

try {
    $download = Join-Path $tempDir 'tnpm.exe'
    $checksums = Join-Path $tempDir 'SHA256SUMS'
    Write-Host "Downloading $asset..."
    Invoke-WebRequest -Uri "$base/$asset" -OutFile $download -UseBasicParsing
    Invoke-WebRequest -Uri "$base/SHA256SUMS" -OutFile $checksums -UseBasicParsing

    $pattern = '^([0-9a-fA-F]{64})\s+\*?' + [regex]::Escape($asset) + '$'
    $expected = @(
        foreach ($line in [System.IO.File]::ReadAllLines($checksums)) {
            if ($line.TrimEnd() -match $pattern) { $Matches[1].ToLowerInvariant() }
        }
    )
    if ($expected.Count -ne 1) {
        throw "Missing or duplicated SHA-256 checksum for $asset."
    }
    $actual = (Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $expected[0]) {
        throw 'SHA-256 mismatch; nothing was installed.'
    }

    & $download --help *> $null
    if ($LASTEXITCODE -ne 0) {
        throw 'The downloaded tnpm executable cannot run on this system.'
    }

    New-Item -ItemType Directory -Path $installDir -Force | Out-Null
    $staged = Join-Path $installDir ('.tnpm-install-' + [guid]::NewGuid().ToString('N') + '.exe')
    try {
        Copy-Item -LiteralPath $download -Destination $staged -ErrorAction Stop
        Move-Item -LiteralPath $staged -Destination (Join-Path $installDir 'tnpm.exe') -Force -ErrorAction Stop
    } finally {
        if (Test-Path -LiteralPath $staged) {
            Remove-Item -LiteralPath $staged -Force
        }
    }

    # Edit the user registry value, not the machine PATH. Preserve %VARIABLE% entries.
    $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
    if ($null -eq $key) { throw 'Cannot open the user environment registry key.' }
    $pathUpdated = $false
    try {
        $userPath = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        $normalizedInstallDir = $installDir.TrimEnd([char[]]@('\', '/'))
        $exists = $false
        foreach ($entry in ($userPath -split ';')) {
            $entry = $entry.Trim().Trim('"')
            if (-not $entry) { continue }
            $expanded = [Environment]::ExpandEnvironmentVariables($entry)
            try {
                $normalizedEntry = [System.IO.Path]::GetFullPath($expanded).TrimEnd([char[]]@('\', '/'))
            } catch {
                $normalizedEntry = $expanded.TrimEnd([char[]]@('\', '/'))
            }
            if ([string]::Equals($normalizedEntry, $normalizedInstallDir, [StringComparison]::OrdinalIgnoreCase)) {
                $exists = $true
                break
            }
        }
        if (-not $exists) {
            $newPath = if ([string]::IsNullOrWhiteSpace($userPath)) {
                $installDir
            } else {
                $userPath.TrimEnd(';') + ';' + $installDir
            }
            $key.SetValue('Path', $newPath, [Microsoft.Win32.RegistryValueKind]::ExpandString)
            $pathUpdated = $true
        }
    } finally {
        $key.Close()
    }

    # Also update this PowerShell session immediately.
    $processEntries = @($env:Path -split ';' | ForEach-Object { $_.TrimEnd([char[]]@('\', '/')) })
    if (-not ($processEntries | Where-Object { $_ -ieq $installDir.TrimEnd([char[]]@('\', '/')) })) {
        $env:Path = $env:Path.TrimEnd(';') + ';' + $installDir
    }

    if ($pathUpdated) {
        # Notify Explorer so newly opened terminals inherit the updated PATH.
        try {
            if (-not ('TnpmInstallerEnvironment' -as [type])) {
                Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class TnpmInstallerEnvironment {
    [DllImport("user32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr SendMessageTimeout(IntPtr window, uint msg, UIntPtr wparam,
        string lparam, uint flags, uint timeout, out UIntPtr result);
}
'@
            }
            $result = [UIntPtr]::Zero
            [void][TnpmInstallerEnvironment]::SendMessageTimeout([IntPtr]65535, 0x001A, [UIntPtr]::Zero, 'Environment', 0x0002, 1000, [ref]$result)
        } catch {
            Write-Warning 'PATH is saved, but the environment-change notification failed. Restart your terminal.'
        }
    }

    Write-Host "Installed tnpm to $(Join-Path $installDir 'tnpm.exe')"
    Write-Host 'The install directory is registered in your user PATH (no administrator rights needed).'
} finally {
    Remove-Item -LiteralPath $tempDir -Recurse -Force -ErrorAction SilentlyContinue
}

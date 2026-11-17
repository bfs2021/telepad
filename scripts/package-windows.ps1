# Build the Windows release files.
#   pwsh scripts/package-windows.ps1              # dist/telepad-<version>-windows-x86_64.zip (+ .sha256), portable
#   pwsh scripts/package-windows.ps1 -Installer   # also ...-windows-x86_64-setup.exe (+ .sha256), needs Inno Setup 6
# Releases are published by .github/workflows/release.yml on a pushed v* tag,
# for every platform at once (macOS / Linux use scripts/package-unix.sh).
param([switch]$Installer)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
$name = "telepad-$version-windows-x86_64"
$dist = Join-Path $root 'dist'
$stage = Join-Path $dist $name

cargo build --release --locked
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force (Join-Path $stage 'licenses') | Out-Null

Copy-Item target/release/telepad.exe, target/release/telepad-check.exe $stage
Copy-Item LICENSE-MIT, LICENSE-APACHE (Join-Path $stage 'licenses')
Copy-Item crates/telepad-gui/assets/fonts/LICENSE-Phosphor.txt (Join-Path $stage 'licenses/Phosphor-MIT.txt')
Copy-Item crates/telepad-gui/assets/os/ATTRIBUTION.md (Join-Path $stage 'licenses/OS-logos.md')

@'
# Example OpenSSH client config. Rename to "config" next to telepad.exe
# before the first start to import these hosts, or use Hosts > Import from a file in the app.

# Description shown under the host name
Host example
  HostName 192.0.2.10
  User admin
  Port 22
  IdentityFile ~/.ssh/id_ed25519
'@ | Set-Content -Encoding utf8 (Join-Path $stage 'config.example')

@"
Telepad $version (Windows x86_64)
=================================

Portable zip: run telepad.exe. Nothing is installed; delete the folder to remove it.
The file "portable" next to the exe keeps all data in this folder.
Installer: Telepad is in the Start menu; uninstall it in Settings > Apps.

First start
  * If a file named "config" (OpenSSH client config) is next to the exe (or in the
    data folder), its hosts are imported into telepad.db automatically.
  * Otherwise the host list starts empty: use Hosts > Add one host, or Hosts > Import
    from a file (OpenSSH config or JSON), or Hosts > Import ~/.ssh/config.

Data (portable: next to the exe; installed: %APPDATA%\Telepad)
  telepad.db           host database (SQLite) - back it up / Export to move it
  telepad.ssh_config   generated for ssh, rewritten on every change (do not edit)
  Uninstalling keeps this data.

Updates
  Telepad checks https://github.com/yohannaftali/telepad/releases at start and
  every 6 h. "Update now" downloads the new version, checks its SHA-256 and replaces
  both exes; your telepad.db and config are kept. Turn it off in
  Hosts > Software update.

Requirements
  * Windows 10/11 with the built-in OpenSSH client (C:\Windows\System32\OpenSSH\ssh.exe)
  * Your SSH keys as referenced by IdentityFile; password logins are not supported
  * Windows Terminal recommended for "Open SSH" (falls back to a console window)

Command line
  telepad-check.exe [--db path] [--json] [alias ...]   one-off check, text or JSON
  telepad-check.exe --check-update                      ask GitHub for a newer release
  telepad.exe --db path                                 use another database

Telepad only runs read-only commands on monitored hosts.
"@ | Set-Content -Encoding utf8 (Join-Path $stage 'README.txt')

# Portable marker: only in the zip (the installer copies the files it lists, not this one).
"Keep Telepad's data (telepad.db) next to telepad.exe. Delete this file to use %APPDATA%\Telepad instead.`n" |
    Set-Content -Encoding utf8 -NoNewline (Join-Path $stage 'portable')

$zip = Join-Path $dist "$name.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $stage -DestinationPath $zip
$hash = (Get-FileHash -Algorithm SHA256 $zip).Hash.ToLower()
# LF only and no BOM, like sha256sum writes it: `sha256sum -c` rejects "name.zip`r".
[IO.File]::WriteAllText("$zip.sha256", "$hash  $name.zip`n", [Text.UTF8Encoding]::new($false))

Write-Output "zip:    $zip"
Write-Output "sha256: $hash"
Write-Output ("size:   {0:N1} MB" -f ((Get-Item $zip).Length / 1MB))

if ($Installer) {
    $iscc = @(
        (Get-Command iscc -ErrorAction SilentlyContinue).Source,
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    ) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
    if (-not $iscc) { throw 'Inno Setup 6 (ISCC.exe) not found - install it or omit -Installer' }
    & $iscc /Q "/DAppVersion=$version" "/DSourceDir=$stage" "/DOutputDir=$dist" (Join-Path $root 'installer\telepad.iss')
    if ($LASTEXITCODE -ne 0) { throw 'iscc failed' }
    $setupName = "$name-setup.exe"
    $setup = Join-Path $dist $setupName
    $shash = (Get-FileHash -Algorithm SHA256 $setup).Hash.ToLower()
    [IO.File]::WriteAllText("$setup.sha256", "$shash  $setupName`n", [Text.UTF8Encoding]::new($false))
    Write-Output "setup:  $setup"
    Write-Output "sha256: $shash"
}

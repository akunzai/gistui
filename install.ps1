<#
.SYNOPSIS
  gistui installer for Windows — downloads the prebuilt release binary, verifies
  its SHA-256 checksum, installs it, and adds the install directory to your PATH.

.DESCRIPTION
  Native PowerShell installer. Unlike piping install.sh into bash from PowerShell
  (which routes through WSL when installed and lands the Linux binary inside WSL),
  this installs the real Windows build onto the Windows side.

  Usage:
    irm https://raw.githubusercontent.com/akunzai/gistui/main/install.ps1 | iex

  To pass options, invoke it as a script block:
    & ([scriptblock]::Create((irm https://raw.githubusercontent.com/akunzai/gistui/main/install.ps1))) -Version v0.9.0

.PARAMETER Version
  Install a specific release tag (default: latest). Env: GISTUI_VERSION.

.PARAMETER BinDir
  Install directory (default: ~\.local\bin). Env: GISTUI_BIN_DIR.

.PARAMETER Help
  Print this help and exit.
#>
[CmdletBinding()]
param(
  [string]$Version = $(if ($env:GISTUI_VERSION) { $env:GISTUI_VERSION } else { 'latest' }),
  [string]$BinDir  = $(if ($env:GISTUI_BIN_DIR) { $env:GISTUI_BIN_DIR } else { '' }),
  [switch]$Help
)

$ErrorActionPreference = 'Stop'
# PowerShell 5.1 on older Windows defaults to TLS 1.0/1.1, which GitHub rejects.
[Net.ServicePointManager]::SecurityProtocol = `
  [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$Repo = 'akunzai/gistui'

function Die([string]$msg) { throw "error: $msg" }

if ($Help) {
  @'
gistui installer (Windows / PowerShell)

  irm https://raw.githubusercontent.com/akunzai/gistui/main/install.ps1 | iex

Options (via env vars, or as a script block):
  -Version <tag>   install a specific release (default: latest)   [GISTUI_VERSION]
  -BinDir  <dir>   install directory (default: ~\.local\bin)       [GISTUI_BIN_DIR]

  & ([scriptblock]::Create((irm <url>))) -Version v0.9.0 -BinDir C:\tools\bin
'@ | Write-Host
  return
}

# --- Resolve install directory (expand ~ and env defaults) --------------------
if (-not $BinDir) { $BinDir = Join-Path $HOME '.local\bin' }
if ($BinDir -match '^~([\\/]|$)') { $BinDir = Join-Path $HOME ($BinDir.Substring(1).TrimStart('\', '/')) }

# --- Detect architecture → Rust target triple ---------------------------------
# PROCESSOR_ARCHITEW6432 is set when a 32-bit process runs on a 64-bit OS.
$rawArch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
switch ($rawArch) {
  'AMD64' { $cpu = 'x86_64' }
  'ARM64' { Die "no prebuilt Windows binary for ARM64 (only x86_64 is published)" }
  default { Die "unsupported architecture: $rawArch" }
}
$target = "$cpu-pc-windows-msvc"

# --- Resolve the release tag --------------------------------------------------
if ($Version -eq 'latest') {
  try {
    $rel = Invoke-RestMethod -UseBasicParsing -Headers @{ 'User-Agent' = 'gistui-installer' } `
      -Uri "https://api.github.com/repos/$Repo/releases/latest"
  } catch {
    Die "could not query the latest release: $($_.Exception.Message)"
  }
  $Version = $rel.tag_name
  if (-not $Version) { Die "could not determine the latest release tag" }
}

$pkg     = "gistui-$Version-$target"
$archive = "$pkg.zip"
$baseUrl = "https://github.com/$Repo/releases/download/$Version"

# --- Download + verify --------------------------------------------------------
$tmp = New-Item -ItemType Directory -Path (Join-Path ([IO.Path]::GetTempPath()) "gistui-$([Guid]::NewGuid())")
try {
  $zipPath    = Join-Path $tmp $archive
  $sha256Path = "$zipPath.sha256"

  Write-Host "downloading $archive ($Version)..."
  try {
    Invoke-WebRequest -UseBasicParsing -Uri "$baseUrl/$archive" -OutFile $zipPath
  } catch {
    Die "download failed; is $Version published for $target?"
  }
  try {
    Invoke-WebRequest -UseBasicParsing -Uri "$baseUrl/$archive.sha256" -OutFile $sha256Path
  } catch {
    Die "checksum download failed for $archive"
  }

  Write-Host "verifying checksum..."
  $expected = (((Get-Content $sha256Path -Raw).Trim() -split '\s+')[0])
  # Hash via .NET rather than Get-FileHash so we don't depend on the
  # Microsoft.PowerShell.Utility cmdlet being loadable.
  $stream = [System.IO.File]::OpenRead($zipPath)
  try {
    $algo   = [System.Security.Cryptography.SHA256]::Create()
    $actual = ([BitConverter]::ToString($algo.ComputeHash($stream)) -replace '-', '')
  } finally { $stream.Dispose() }
  if ($actual -ne $expected) { Die "checksum verification failed for $archive" }

  # --- Extract ----------------------------------------------------------------
  # Extract via .NET rather than Expand-Archive (Microsoft.PowerShell.Archive).
  Write-Host "extracting..."
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  [System.IO.Compression.ZipFile]::ExtractToDirectory($zipPath, $tmp)
  $src = Join-Path $tmp "$pkg\gistui.exe"
  if (-not (Test-Path $src)) { Die "binary not found in archive at $pkg\gistui.exe" }

  # --- Install ----------------------------------------------------------------
  New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
  $dest = Join-Path $BinDir 'gistui.exe'
  Copy-Item -Path $src -Destination $dest -Force
  Write-Host "installed gistui $Version to $dest"
} finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

# --- Add BinDir to the user PATH (idempotent) ---------------------------------
$target_dir = $BinDir.TrimEnd('\')
$userPath   = [Environment]::GetEnvironmentVariable('Path', 'User')
$onPath = $false
foreach ($p in (@($userPath) -split ';')) {
  if ([string]::IsNullOrWhiteSpace($p)) { continue }
  if (([Environment]::ExpandEnvironmentVariables($p).TrimEnd('\')) -ieq $target_dir) { $onPath = $true; break }
}

if (-not $onPath) {
  $newPath = if ([string]::IsNullOrEmpty($userPath)) { $target_dir } else { "$($userPath.TrimEnd(';'));$target_dir" }
  [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')

  # Broadcast so already-open shells/Explorer pick up the change. Best-effort:
  # the variable is set regardless, a new shell will see it.
  try {
    if (-not ('Win32.NativeMethods' -as [type])) {
      Add-Type -Namespace Win32 -Name NativeMethods -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("user32.dll", SetLastError=true, CharSet=System.Runtime.InteropServices.CharSet.Auto)]
public static extern System.IntPtr SendMessageTimeout(System.IntPtr hWnd, uint Msg, System.UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out System.UIntPtr lpdwResult);
'@
    }
    [Win32.NativeMethods]::SendMessageTimeout([IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]([UIntPtr]::Zero)) | Out-Null
  } catch { }

  Write-Host "added $target_dir to your user PATH (restart open shells to pick it up)"
}
# Make gistui usable in this session too.
if (($env:Path -split ';') -notcontains $target_dir) { $env:Path = "$env:Path;$target_dir" }

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
  Write-Host "note: gistui needs the GitHub CLI ('gh') at runtime — see https://cli.github.com"
}

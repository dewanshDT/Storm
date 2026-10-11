<#
.SYNOPSIS
  Installs Storm on Windows: the server, the Runtime Host, or both (decision 99).

.DESCRIPTION
  The Windows counterpart of install.sh (decision 84). It downloads the release
  zips, checks each against the release's checksums.txt, and then:

  - Server: `storm-server up` as you (a logon task, notes in %USERPROFILE%\Storm),
    one inbound firewall rule for private networks (as administrator), and a
    pairing code for your first device when the server has no account yet.
  - Runtime: `storm-runtime install` (the StormRuntime service, as administrator),
    then enrollment: straight from the server on this machine, or by pasting the
    app's enrollment string at a hidden prompt. It never reaches a command line,
    the screen or a log. Optionally a first workspace (a folder name or a git URL).

  Re-running it upgrades. It never deletes notes or workspaces, and sends nothing
  anywhere but GitHub.

    irm https://dewanshdt.github.io/Storm/install.ps1 | iex

  Or, with options:

    & ([scriptblock]::Create((irm https://dewanshdt.github.io/Storm/install.ps1))) -Runtime -Workspace scratch

.PARAMETER Server
  Install the server.
.PARAMETER Runtime
  Install the Runtime Host. Needs an elevated PowerShell.
.PARAMETER Version
  A release tag such as v0.8.0. Default: the latest release.
.PARAMETER FromDir
  Take the zips and checksums.txt from this folder instead of downloading them.
.PARAMETER Workspace
  A first workspace for the Runtime Host: a folder name, or a git URL to clone.
.PARAMETER Yes
  Ask nothing: install what was chosen, and skip what needs an answer.
.PARAMETER DryRun
  Print the plan and change nothing.
#>
[CmdletBinding()]
param(
    [switch]$Server,
    [switch]$Runtime,
    [string]$Version,
    [string]$FromDir,
    [string]$Workspace,
    [switch]$Yes,
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 3

$Repo = 'dewanshDT/Storm'
$ServerZip = 'storm-server-windows-x86_64.zip'
$RuntimeZip = 'storm-runtime-windows-x86_64.zip'
$Port = 8484

function Say($text) { Write-Host $text }
function Step($text) { Write-Host "==> $text" -ForegroundColor Cyan }
# `throw`, never `exit`: run through `irm | iex`, `exit` would close the
# person's PowerShell window.
function Fail($text) { throw "storm install: $text" }

function Test-Admin {
    $id = [Security.Principal.WindowsIdentity]::GetCurrent()
    (New-Object Security.Principal.WindowsPrincipal $id).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
}

function Ask($question, $default) {
    if ($Yes) { return $default }
    $answer = Read-Host "$question [$default]"
    if ([string]::IsNullOrWhiteSpace($answer)) { $default } else { $answer.Trim() }
}

# --- what to install -----------------------------------------------------------

if (-not $Server -and -not $Runtime) {
    if ($Yes) { Fail 'with -Yes, say what to install: -Server, -Runtime, or both' }
    Say 'Storm for Windows. What should this machine run?'
    Say '  1) the server and the Runtime Host'
    Say '  2) the server only'
    Say '  3) the Runtime Host only (for a server elsewhere)'
    switch (Ask 'Choose' '1') {
        '1' { $Server = $true; $Runtime = $true }
        '2' { $Server = $true }
        '3' { $Runtime = $true }
        default { Fail 'choose 1, 2 or 3' }
    }
}

$admin = Test-Admin
if ($Runtime -and -not $admin) {
    Fail 'the Runtime Host installs a Windows service: run this in PowerShell as administrator'
}

if (-not $Version -and -not $FromDir) {
    $Version = (Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest").tag_name
}

$serverBin = Join-Path $env:LOCALAPPDATA 'Storm\bin\storm-server.exe'
$runtimeBin = Join-Path $env:ProgramFiles 'StormRuntime\storm-runtime.exe'
$dataState = Join-Path $env:USERPROFILE 'Storm\state'

Step "Plan ($(if ($Version) { $Version } else { "from $FromDir" }))"
if ($Server) {
    Say "  server   : $ServerZip, checked; storm-server up as $env:USERNAME (logon task, notes in $env:USERPROFILE\Storm)"
    Say "  firewall : inbound TCP $Port for storm-serverw.exe, private networks only $(if (-not $admin) { '(needs administrator: printed instead)' })"
}
if ($Runtime) {
    Say "  runtime  : $RuntimeZip, checked; storm-runtime install (the StormRuntime service) [administrator]"
    Say "  enroll   : $(if ($Server) { 'with the server on this machine' } else { 'paste the enrollment string from the app (hidden)' })"
    if ($Workspace) { Say "  workspace: $Workspace" }
}
if ($DryRun) { return }

# --- download and check ----------------------------------------------------------

$work = Join-Path ([IO.Path]::GetTempPath()) ("storm-install-" + [Guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Path $work | Out-Null

function Fetch($name) {
    $dest = Join-Path $work $name
    if ($FromDir) {
        Copy-Item (Join-Path $FromDir $name) $dest
    } else {
        Invoke-WebRequest "https://github.com/$Repo/releases/download/$Version/$name" -OutFile $dest -UseBasicParsing
    }
    $dest
}

$sums = Get-Content (Fetch 'checksums.txt')
function Checked($name) {
    $path = Fetch $name
    $line = $sums | Where-Object { $_ -match "\s\*?$([regex]::Escape($name))$" } | Select-Object -First 1
    if (-not $line) { Fail "$name is not in checksums.txt" }
    $want = ($line -split '\s+')[0].ToLower()
    $got = (Get-FileHash -Algorithm SHA256 $path).Hash.ToLower()
    if ($want -ne $got) { Fail "$name does not match checksums.txt" }
    $dir = Join-Path $work ([IO.Path]::GetFileNameWithoutExtension($name))
    Expand-Archive -Path $path -DestinationPath $dir -Force
    $dir
}

# --- the server ------------------------------------------------------------------

if ($Server) {
    Step 'Server'
    $dir = Checked $ServerZip
    & (Join-Path $dir 'storm-server.exe') up --port $Port
    if ($LASTEXITCODE -ne 0) { Fail 'storm-server up failed' }
    $rule = "New-NetFirewallRule -DisplayName 'Storm Server' -Direction Inbound -Program '$(Join-Path $env:LOCALAPPDATA 'Storm\bin\storm-serverw.exe')' -Protocol TCP -LocalPort $Port -Profile Private -Action Allow"
    if ($admin) {
        if (-not (Get-NetFirewallRule -DisplayName 'Storm Server' -ErrorAction SilentlyContinue)) {
            Invoke-Expression $rule | Out-Null
        }
        Say 'Allowed inbound connections on private networks.'
    } else {
        Say "So your phone can reach it, run once in PowerShell as administrator:`n  $rule"
    }
    & $serverBin has-account --state $dataState
    if ($LASTEXITCODE -ne 0) {
        Say "`nPair your first device: in the Storm app, add a server and scan this code."
        & $serverBin pair --state $dataState --qr
    }
}

# --- the Runtime Host -----------------------------------------------------------

if ($Runtime) {
    Step 'Runtime Host'
    $dir = Checked $RuntimeZip
    & (Join-Path $dir 'storm-runtime.exe') install
    if ($LASTEXITCODE -ne 0) { Fail 'storm-runtime install failed' }

    if (Test-Path (Join-Path $env:ProgramData 'StormRuntime\state\host.json')) {
        Say 'Already enrolled; the service was upgraded in place.'
    } elseif ($Server) {
        # The enrollment string goes from one process to the other through a
        # pipe, and never reaches a command line, the screen or a log (84e).
        & $serverBin host-enrollment --state $dataState | & $runtimeBin enroll
        if ($LASTEXITCODE -ne 0) { Fail 'enrollment failed' }
    } elseif ($Yes) {
        Say "Enroll it later, in PowerShell as administrator:`n  & `"$runtimeBin`" enroll"
    } else {
        Say 'In the Storm app: Settings > Agents > Hosts > Enroll a host, and copy the string.'
        $secret = Read-Host 'Paste the enrollment string' -AsSecureString
        $plain = [Runtime.InteropServices.Marshal]::PtrToStringBSTR(
            [Runtime.InteropServices.Marshal]::SecureStringToBSTR($secret))
        $plain | & $runtimeBin enroll
        $plain = $null
        if ($LASTEXITCODE -ne 0) { Fail 'enrollment failed' }
    }

    $root = Join-Path $env:ProgramData 'StormRuntime\workspaces'
    $hasOne = @(Get-ChildItem -Directory $root -ErrorAction SilentlyContinue | Where-Object { -not $_.Name.StartsWith('.') }).Count -gt 0
    if (-not $Workspace -and -not $hasOne -and -not $Yes) {
        $Workspace = Read-Host 'A first workspace: a folder name, or a git URL to clone (Enter skips)'
    }
    if ($Workspace) {
        if ($Workspace -match '^(https?://|git@|ssh://)') {
            $env:GIT_TERMINAL_PROMPT = '0'
            git -C $root clone $Workspace
            if ($LASTEXITCODE -ne 0) { Say "warning: could not clone $Workspace (does it need a login?)" }
        } elseif ($Workspace -match '^[A-Za-z0-9._ -]+$' -and -not $Workspace.StartsWith('.')) {
            New-Item -ItemType Directory -Force -Path (Join-Path $root $Workspace) | Out-Null
            Say "Workspace: $(Join-Path $root $Workspace)"
        } else {
            Say "warning: $Workspace is neither a folder name nor a git URL; skipped"
        }
    }
}

Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
Step 'Done'

<#
    package.ps1: build a VaultForge release from this source tree.

        powershell -ExecutionPolicy Bypass -File installer\package.ps1

    Writes to installer\out\:
        VaultForge-<version>-setup.exe     the installer (needs NSIS 3: nsis.sourceforge.io)
        VaultForge-<version>-portable.zip  the same exe, no install
        SHA256SUMS.txt                     checksums for both

    Builds into target\package so it never disturbs a normal `cargo build`.
    -NoBuild reuses the exe already built there.
#>
param([string]$Makensis = "", [switch]$NoBuild)
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = (Resolve-Path (Join-Path $here "..")).Path
$out  = Join-Path $here "out"

$m = Select-String -Path (Join-Path $repo "Cargo.toml") -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1
if (-not $m) { throw "no version in Cargo.toml" }
$ver = $m.Matches[0].Groups[1].Value

$target = Join-Path $repo "target\package"
if (-not $NoBuild) {
    # Rust bakes source paths into the exe for its panic messages, including
    # the builder's cargo, rustup and project folders: C:\Users\<your name>\...
    # Rewrite them so a release never carries who built it. The first flag is
    # a catch-all; later, more specific ones win.
    #
    # CARGO_ENCODED_RUSTFLAGS replaces .cargo\config.toml's rustflags rather
    # than adding to them, so the static C runtime is requested here again.
    $cargoHome  = if ($env:CARGO_HOME)  { $env:CARGO_HOME }  else { Join-Path $env:USERPROFILE ".cargo" }
    $rustupHome = if ($env:RUSTUP_HOME) { $env:RUSTUP_HOME } else { Join-Path $env:USERPROFILE ".rustup" }
    $env:CARGO_ENCODED_RUSTFLAGS = @(
        "-Ctarget-feature=+crt-static",
        "--remap-path-prefix=$($env:USERPROFILE)=~",
        "--remap-path-prefix=$cargoHome=cargo",
        "--remap-path-prefix=$rustupHome=rustup",
        "--remap-path-prefix=$target=target",
        "--remap-path-prefix=$repo=VaultForge"
    ) -join [char]0x1f
    $env:CARGO_TARGET_DIR = $target
    Push-Location $repo
    try {
        cargo build --release --locked --bin VaultForge
        if ($LASTEXITCODE) { throw "cargo build failed" }
    } finally {
        Pop-Location
        Remove-Item Env:CARGO_ENCODED_RUSTFLAGS, Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
    }
}
$exe = Join-Path $target "release\VaultForge.exe"
if (-not (Test-Path $exe)) { throw "no $exe - build first" }

# Release gates on the exe itself.
$bin = [IO.File]::ReadAllBytes($exe)
$ascii = [Text.Encoding]::ASCII.GetString($bin)
foreach ($enc in [Text.Encoding]::ASCII, [Text.Encoding]::Unicode) {
    if ($enc.GetString($bin).IndexOf($env:USERPROFILE, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
        throw "the exe contains your user folder ($($env:USERPROFILE)) - it would leak who built it. Rebuild without -NoBuild."
    }
}
if ($ascii.IndexOf("VCRUNTIME140", [StringComparison]::OrdinalIgnoreCase) -ge 0) {
    throw "the exe imports VCRUNTIME140.dll: it would not start on PCs without the Visual C++ Redistributable. The static CRT flag was lost."
}
$version = (Get-Item $exe).VersionInfo.ProductVersion
if ($version -ne $ver) { throw "the exe reports version '$version', Cargo.toml says '$ver' (Windows resources not embedded?)" }

$icon = Get-ChildItem (Join-Path $target "release\build") -Recurse -Filter vaultforge-icon.ico |
    Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $icon) { throw "vaultforge-icon.ico not found under $target\release\build" }

if (-not $Makensis) {
    $Makensis = @("${env:ProgramFiles(x86)}\NSIS\makensis.exe", "$env:ProgramFiles\NSIS\makensis.exe") |
        Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $Makensis) { throw "NSIS 3 (makensis.exe) not found: install it from nsis.sourceforge.io or pass -Makensis <path>" }

# Stage exactly what ships.
New-Item -ItemType Directory -Force $out | Out-Null
$stageRoot = Join-Path $out "stage"
$stage = Join-Path $stageRoot "VaultForge"
if (Test-Path $stageRoot) { Remove-Item -Recurse -Force $stageRoot }
New-Item -ItemType Directory -Force (Join-Path $stage "licenses\egui-fonts") | Out-Null
Copy-Item $exe (Join-Path $stage "VaultForge.exe")
foreach ($d in "README.md", "LICENSE.md", "PRIVACY.md", "SECURITY.md", "THIRD_PARTY_NOTICES.md") { Copy-Item (Join-Path $repo $d) $stage }
Copy-Item (Join-Path $repo "licenses\egui-fonts\*.txt") (Join-Path $stage "licenses\egui-fonts")

# Portable zip (one VaultForge\ folder inside).
$zip = Join-Path $out "VaultForge-$ver-portable.zip"
if (Test-Path $zip) { Remove-Item $zip }
Add-Type -AssemblyName System.IO.Compression.FileSystem
[IO.Compression.ZipFile]::CreateFromDirectory($stage, $zip, [IO.Compression.CompressionLevel]::Optimal, $true)

# Installer.
$setup = Join-Path $out "VaultForge-$ver-setup.exe"
& $Makensis /V2 "/DVERSION=$ver" "/DSTAGE=$stage" "/DICON=$($icon.FullName)" "/DOUTFILE=$setup" (Join-Path $here "vaultforge.nsi")
if ($LASTEXITCODE) { throw "makensis failed" }

# Checksums (sha256sum format: lower-case hash, two spaces, file name).
$sums = foreach ($f in $setup, $zip) {
    "{0}  {1}" -f (Get-FileHash $f -Algorithm SHA256).Hash.ToLower(), (Split-Path $f -Leaf)
}
[IO.File]::WriteAllText((Join-Path $out "SHA256SUMS.txt"), (($sums -join "`n") + "`n"))

Remove-Item -Recurse -Force $stageRoot
Get-ChildItem $out -File | ForEach-Object { "{0,-36} {1,8:N1} MB" -f $_.Name, ($_.Length / 1MB) }
Get-Content (Join-Path $out "SHA256SUMS.txt")

# Build tools for OurStick in the Windows VM: Git, Node 24, VS Build Tools
# (C++), Rust. Run once over ssh; see docs/windows-dev.md. Versions as of
# 2026-09-29; update the URLs and the hashes together. Nothing downloaded
# runs before it is checked: Git and Node by the SHA-256 their publishers
# list, rustup by the .sha256 beside it, the Build Tools by Microsoft's
# signature.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Start-Transcript -Path C:\install-tools.log -Append
$dl = "$env:TEMP\tools"; New-Item -ItemType Directory -Force -Path $dl | Out-Null

function Get-File($url, $name) {
    $path = Join-Path $dl $name
    Invoke-WebRequest -Uri $url -OutFile $path -UseBasicParsing
    return $path
}

function Assert-Hash($path, $expected) {
    $hash = (Get-FileHash -Algorithm SHA256 $path).Hash.ToLower()
    if ($hash -ne $expected.ToLower()) { throw "$path hash mismatch: $hash" }
}

Write-Host '== Git'
$git = Get-File 'https://github.com/git-for-windows/git/releases/download/v2.56.0.windows.1/Git-2.56.0-64-bit.exe' 'git.exe'
Assert-Hash $git 'bfe94e7b419b16eee9fecbd1253a98e3d4f49ba8f029630549052278ffe286a6'
Start-Process -Wait -FilePath $git -ArgumentList '/VERYSILENT','/NORESTART','/SP-','/SUPPRESSMSGBOXES'

Write-Host '== Node 24'
$node = Get-File 'https://nodejs.org/dist/latest-v24.x/node-v24.21.0-x64.msi' 'node.msi'
Assert-Hash $node 'bb0eaee134f9357f22aea915ee793343e627aefc1e66488164bac6915bce2cac'
Start-Process -Wait -FilePath msiexec.exe -ArgumentList '/i', $node, '/qn', '/norestart'

Write-Host '== Visual Studio Build Tools (C++)'
$vs = Get-File 'https://aka.ms/vs/17/release/vs_BuildTools.exe' 'vs_BuildTools.exe'
$signature = Get-AuthenticodeSignature $vs
if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'O=Microsoft Corporation') {
    throw "vs_BuildTools.exe is not signed by Microsoft: $($signature.Status)"
}
$p = Start-Process -Wait -PassThru -FilePath $vs -ArgumentList '--quiet','--wait','--norestart','--nocache',
    '--add','Microsoft.VisualStudio.Workload.VCTools','--includeRecommended'
Write-Host "vs_BuildTools exit: $($p.ExitCode)"

Write-Host '== Rust'
$rustup = Get-File 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe' 'rustup-init.exe'
$listed = Get-File 'https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe.sha256' 'rustup-init.exe.sha256'
Assert-Hash $rustup ((Get-Content $listed -Raw).Trim() -split '\s+')[0]
Start-Process -Wait -FilePath $rustup -ArgumentList '-y','--default-toolchain','stable','--profile','default'

Write-Host '== done'
Stop-Transcript

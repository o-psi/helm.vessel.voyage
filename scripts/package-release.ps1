param([Parameter(Mandatory=$true)][string]$Version, [Parameter(Mandatory=$true)][string]$Target)
$ErrorActionPreference = "Stop"
$archive = "voyage-$Version-$Target"
$stage = Join-Path ([System.IO.Path]::GetTempPath()) $archive
Remove-Item $stage -Recurse -Force -ErrorAction SilentlyContinue
New-Item "$stage/bin", "$stage/share/man/man1", "$stage/share/completions" -ItemType Directory -Force | Out-Null
Copy-Item "target/$Target/release/helm.exe", "target/$Target/release/vessel.exe", "target/$Target/release/voyage-installer.exe" "$stage/bin"
foreach ($binary in @("helm", "vessel")) {
  & "target/$Target/release/$binary.exe" manpage | Out-File -Encoding utf8 "$stage/share/man/man1/$binary.1"
  foreach ($shell in @("bash", "zsh", "fish", "powershell", "elvish")) {
    & "target/$Target/release/$binary.exe" completions $shell | Out-File -Encoding utf8 "$stage/share/completions/$binary.$shell"
  }
}
Copy-Item README.md $stage
New-Item dist -ItemType Directory -Force | Out-Null
Compress-Archive -Path $stage -DestinationPath "dist/$archive.zip" -Force
$hash = (Get-FileHash "dist/$archive.zip" -Algorithm SHA256).Hash.ToLower()
"$hash  $archive.zip" | Out-File -Encoding ascii "dist/$archive.zip.sha256"

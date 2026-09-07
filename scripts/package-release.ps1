param([Parameter(Mandatory=$true)][string]$Version, [Parameter(Mandatory=$true)][string]$Target)
$ErrorActionPreference = "Stop"
if ($Version -notmatch '^[A-Za-z0-9._-]+$' -or $Version -in @('.', '..')) { throw 'invalid version label' }
if ($Target -notmatch '^[A-Za-z0-9_-]+$') { throw 'invalid target label' }
$archive = "voyage-$Version-$Target"
New-Item dist -ItemType Directory -Force | Out-Null
$dist = (Resolve-Path dist).Path
$lock = Join-Path $dist ".$archive.lock"
New-Item $lock -ItemType Directory -ErrorAction Stop | Out-Null
$stage = Join-Path $dist ('.package-' + [Guid]::NewGuid().ToString('N'))
$destination = Join-Path $dist "$archive.zip"
try {
  if ((Test-Path -LiteralPath $destination) -or (Test-Path -LiteralPath "$destination.sha256")) { throw 'release label already exists' }
  $content = Join-Path $stage $archive
  New-Item "$content/bin", "$content/share/man/man1", "$content/share/completions" -ItemType Directory -Force | Out-Null
  Copy-Item "target/$Target/release/helm.exe", "target/$Target/release/vessel.exe", "target/$Target/release/voyage.exe", "target/$Target/release/voyage-installer.exe" "$content/bin"
  $binaries = @{}
  foreach ($binary in @('helm', 'vessel', 'voyage', 'voyage-installer')) {
    $binaries[$binary] = @{ sha256 = (Get-FileHash "$content/bin/$binary.exe" -Algorithm SHA256).Hash.ToLower() }
  }
  $manifest = @{ schema_version = 1; version = $Version; target = $Target; binaries = $binaries } | ConvertTo-Json -Depth 4
  [IO.File]::WriteAllText((Join-Path $content 'release.json'), $manifest + "`n", [Text.UTF8Encoding]::new($false))
  foreach ($binary in @('helm', 'vessel', 'voyage')) {
    & "target/$Target/release/$binary.exe" manpage | Out-File -Encoding utf8 "$content/share/man/man1/$binary.1"
    if ($LASTEXITCODE -ne 0) { throw "$binary manpage failed" }
    foreach ($shell in @('bash', 'zsh', 'fish', 'powershell', 'elvish')) {
      & "target/$Target/release/$binary.exe" completions $shell | Out-File -Encoding utf8 "$content/share/completions/$binary.$shell"
      if ($LASTEXITCODE -ne 0) { throw "$binary completions failed" }
    }
  }
  foreach ($document in Get-Content scripts/release-documents.txt) {
    if ($document -notmatch '^[A-Za-z0-9_./-]+$' -or $document.StartsWith('/') -or $document.Split('/') -contains '..' -or $document.Split('/') -contains '.') { throw 'invalid release document path' }
    $source = Get-Item -LiteralPath $document
    if ($source.PSIsContainer -or ($source.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'invalid release document file' }
    $ancestor = $source.Directory
    while ($ancestor.FullName -ne (Get-Location).Path) {
      if (($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint) -or $null -eq $ancestor.Parent) { throw 'invalid release document ancestor' }
      $ancestor = $ancestor.Parent
    }
    $output = Join-Path $content $document
    New-Item (Split-Path $output) -ItemType Directory -Force | Out-Null
    Copy-Item -LiteralPath $document -Destination $output
  }
  $temporaryArchive = Join-Path $stage "$archive.zip"
  Compress-Archive -Path $content -DestinationPath $temporaryArchive
  $hash = (Get-FileHash $temporaryArchive -Algorithm SHA256).Hash.ToLower()
  "$hash  $archive.zip" | Out-File -Encoding ascii "$temporaryArchive.sha256"
  [IO.File]::Move($temporaryArchive, $destination)
  try { [IO.File]::Move("$temporaryArchive.sha256", "$destination.sha256") }
  catch { Remove-Item -LiteralPath $destination; throw }
  Write-Output "created $destination"
}
finally {
  if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
  Remove-Item -LiteralPath $lock
}

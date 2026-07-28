# Kemas bundle VST3 menjadi ZIP yang diterima Studio Hub.
#
# JANGAN pakai `Compress-Archive`: di Windows ia menulis pemisah path sebagai
# backslash, yang melanggar spesifikasi ZIP. Arsip seperti itu terbuka normal
# di Explorer dan ditolak launcher SETELAH pengguna selesai mengunduh — mode
# kegagalan paling mahal yang ada di alur ini. Entri di sini ditulis manual
# supaya namanya dijamin memakai '/'.
#
# Pemakaian:
#   .\scripts\make-plugin-zip.ps1 -Project "C:\...\Synth Project\Clipper"
#   .\scripts\make-plugin-zip.ps1 -Bundle "C:\...\Release\VST3\Clipper.vst3" -Version 1.2.0
#
# Dengan -Project, skrip mencari sendiri bundle `.vst3` di bawah
# `build\*_artefacts\Release\VST3` dan membaca versinya dari DLL.

[CmdletBinding(DefaultParameterSetName = "Project")]
param(
  [Parameter(ParameterSetName = "Project", Mandatory = $true)]
  [string]$Project,

  [Parameter(ParameterSetName = "Bundle", Mandatory = $true)]
  [string]$Bundle,

  # Kalau tidak diisi, diambil dari FileVersion DLL di dalam bundle.
  [string]$Version,

  # Nama berkas ZIP tanpa versi, mis. "RingMood" untuk RingMood-1.0.0-win64.zip.
  # Default: nama bundle tanpa spasi dan tanpa akhiran .vst3.
  [string]$Name,

  [string]$OutDir = (Join-Path (Split-Path $PSScriptRoot -Parent) "..\release-assets")
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem

# ── Temukan bundle ────────────────────────────────────────────────────────
if ($PSCmdlet.ParameterSetName -eq "Project") {
  $search = Join-Path $Project "build"
  if (-not (Test-Path $search)) {
    throw "Tidak ada folder build di $Project — apakah pluginnya sudah di-build Release?"
  }
  $found = Get-ChildItem $search -Recurse -Directory -Filter "*.vst3" -ErrorAction SilentlyContinue |
    Where-Object { $_.FullName -match '\\Release\\VST3\\' }
  if (-not $found) { throw "Tidak menemukan bundle .vst3 di bawah $search" }
  if ($found.Count -gt 1) {
    throw "Ditemukan lebih dari satu bundle:`n" + (($found | ForEach-Object { "  " + $_.FullName }) -join "`n") +
          "`nJalankan lagi dengan -Bundle untuk memilih salah satu."
  }
  $Bundle = $found[0].FullName
}

if (-not (Test-Path $Bundle)) { throw "Bundle tidak ada: $Bundle" }
$base = (Resolve-Path $Bundle).Path.TrimEnd('\')
$root = Split-Path $base -Leaf

# ── Versi ─────────────────────────────────────────────────────────────────
if (-not $Version) {
  $dll = Get-ChildItem (Join-Path $base "Contents") -Recurse -File -Filter "*.vst3" -ErrorAction SilentlyContinue |
    Select-Object -First 1
  if (-not $dll) { throw "Tidak menemukan DLL di dalam $root — bundle tidak lengkap." }
  $Version = $dll.VersionInfo.FileVersion
  if (-not $Version) { throw "DLL tidak punya FileVersion; isi -Version secara manual." }
}

if (-not $Name) { $Name = ($root -replace '\.vst3$', '') -replace '\s', '' }

# ── Kemas ─────────────────────────────────────────────────────────────────
$OutDir = (New-Item -ItemType Directory -Force $OutDir).FullName
$dest = Join-Path $OutDir "$Name-$Version-win64.zip"
if (Test-Path $dest) { [System.IO.File]::Delete($dest) }

$fs = [System.IO.File]::Open($dest, [System.IO.FileMode]::CreateNew)
$zip = New-Object System.IO.Compression.ZipArchive($fs, [System.IO.Compression.ZipArchiveMode]::Create)
try {
  foreach ($f in Get-ChildItem $base -Recurse -File) {
    $rel = $f.FullName.Substring($base.Length).TrimStart('\')
    $entryName = "$root/" + ($rel -replace '\\', '/')
    $entry = $zip.CreateEntry($entryName, [System.IO.Compression.CompressionLevel]::Optimal)
    # Tanpa ini .NET memakai waktu SAAT INI sebagai timestamp entri, sehingga
    # mengemas bundle yang sama dua kali menghasilkan hash berbeda. Mengambil
    # waktu berkas sumber membuat hasilnya dapat direproduksi — dan hash yang
    # stabil jauh lebih mudah dipercaya saat membandingkan dua rilis.
    $entry.LastWriteTime = [System.DateTimeOffset]::new($f.LastWriteTimeUtc, [TimeSpan]::Zero)
    $es = $entry.Open()
    $src = [System.IO.File]::OpenRead($f.FullName)
    $src.CopyTo($es)
    $src.Dispose(); $es.Dispose()
  }
} finally { $zip.Dispose(); $fs.Dispose() }

# ── Periksa hasilnya ──────────────────────────────────────────────────────
$a = [System.IO.Compression.ZipFile]::OpenRead($dest)
$names = $a.Entries | ForEach-Object { $_.FullName }
$a.Dispose()

# `@()` wajib: PowerShell mengembalikan skalar untuk satu hasil, dan
# mengindeks string dengan [0] menghasilkan satu KARAKTER, bukan elemen.
$roots = @($names | ForEach-Object { ($_ -split '/')[0] } | Sort-Object -Unique)
$hasDll = @($names | Where-Object { $_ -match 'Contents/x86_64-win/.+\.vst3$' })

if ($roots.Count -ne 1) { throw "Entri akar harus tepat satu, ditemukan: $($roots -join ', ')" }
if (-not $hasDll) { throw "Tidak ada DLL di Contents/x86_64-win/ — bundle tidak lengkap." }

Write-Host ""
Write-Host "  berkas       : $dest"
Write-Host "  archive_root : $($roots[0])"
Write-Host "  versi        : $Version"
Write-Host "  size_bytes   : $((Get-Item $dest).Length)"
Write-Host "  sha256       : $((Get-FileHash $dest -Algorithm SHA256).Hash.ToLower())"
Write-Host ""
Write-Host "  Selanjutnya: cargo test -p hub-core --test real_artifacts"
Write-Host "  Hash di atas tidak perlu kamu catat — CI katalog menghitungnya"
Write-Host "  sendiri dari asset yang benar-benar terunggah."

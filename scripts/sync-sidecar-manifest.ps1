<#
.SYNOPSIS
  Met à jour src-tauri/resources/sidecars-manifest.json avec les versions officielles les plus récentes,
  leurs URLs figées par tag et leurs empreintes SHA-256 (archive + exécutable extrait).

.NOTES
  Compatible Windows PowerShell 5.1 et PowerShell 7.
  Les URLs écrites sont celles de releases précises (immuables) : jamais « latest »,
  dont le contenu change et ferait échouer la vérification SHA-256.
  Si un outil change le nom de ses fichiers, le script s'arrête avec un message clair :
  ajustez alors l'expression régulière correspondante ci-dessous.
#>
[CmdletBinding()]
param(
  [string]$OutManifest = "",
  # L'archive CUDA de whisper.cpp est volumineuse : ce commutateur l'exclut.
  [switch]$SkipWhisperCuda
)

Set-StrictMode -Version Latest

# $PSScriptRoot est vide dans les valeurs par défaut du bloc param sur certaines versions de PowerShell :
# on le résout ici, dans le corps du script.
$ScriptDir = if ($PSScriptRoot) { $PSScriptRoot } else { Split-Path -Parent $MyInvocation.MyCommand.Path }
if (-not $OutManifest) {
  $OutManifest = Join-Path $ScriptDir "..\src-tauri\resources\sidecars-manifest.json"
}
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"   # accélère fortement Invoke-WebRequest sous PowerShell 5.1
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$TempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("clipforge-manifest-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Path $TempRoot | Out-Null

function Get-GitHubReleases {
  param([string]$Repo)
  $headers = @{ "User-Agent" = "ClipForge-ManifestSync/1.0"; "Accept" = "application/vnd.github+json" }
  if ($env:GITHUB_TOKEN) { $headers["Authorization"] = "Bearer $($env:GITHUB_TOKEN)" }
  Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases?per_page=30" -Headers $headers
}

function Find-ReleaseAsset {
  param([string]$Repo, [string]$TagRegex, [string]$AssetRegex)
  foreach ($release in (Get-GitHubReleases -Repo $Repo)) {
    if ($release.draft -or $release.prerelease) { continue }
    if ($release.tag_name -notmatch $TagRegex) { continue }
    $asset = $release.assets | Where-Object { $_.name -match $AssetRegex } | Select-Object -First 1
    if ($asset) { return @{ Release = $release; Asset = $asset } }
  }
  throw "Aucun asset '$AssetRegex' (tag '$TagRegex') parmi les 30 dernières releases de $Repo. Ouvrez https://github.com/$Repo/releases et ajustez l'expression régulière."
}

function Get-Sha256 {
  param([string]$FilePath)
  (Get-FileHash -Path $FilePath -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Resolve-ToolEntry {
  param(
    [string]$Id,
    [string]$Variant,
    [string]$Repo,
    [string]$TagRegex,
    [string]$AssetRegex,
    [ValidateSet("binary", "zip")][string]$ArchiveKind,
    [string]$ExecutableName,
    [string[]]$SelfTestArgs
  )

  Write-Host "-> $Id ($Variant) depuis $Repo" -ForegroundColor Cyan
  $found = Find-ReleaseAsset -Repo $Repo -TagRegex $TagRegex -AssetRegex $AssetRegex
  $asset = $found.Asset

  $dlPath = Join-Path $TempRoot ("$Id-$Variant-" + $asset.name)
  Write-Host ("   téléchargement de {0} ({1:N0} Mo)" -f $asset.name, ($asset.size / 1MB))
  Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $dlPath -UseBasicParsing
  $dlSha = Get-Sha256 -FilePath $dlPath

  if ($ArchiveKind -eq "binary") {
    $exeSha = $dlSha
    $exeRel = $ExecutableName
  } else {
    $extractDir = Join-Path $TempRoot "extract-$Id-$Variant"
    Expand-Archive -Path $dlPath -DestinationPath $extractDir -Force
    $exeFile = Get-ChildItem -Path $extractDir -Recurse -File -Filter $ExecutableName | Select-Object -First 1
    if (-not $exeFile) { throw "Exécutable '$ExecutableName' introuvable dans l'archive $($asset.name)" }
    $exeSha = Get-Sha256 -FilePath $exeFile.FullName
    $exeRel = $exeFile.FullName.Substring($extractDir.Length).TrimStart('\', '/').Replace('\', '/')
  }

  Remove-Item -Path $dlPath -Force -ErrorAction SilentlyContinue

  return [ordered]@{
    id                  = $Id
    variant             = $Variant
    version             = $found.Release.tag_name
    url                 = $asset.browser_download_url
    archive_kind        = $ArchiveKind
    download_sha256     = $dlSha
    executable_rel_path = $exeRel
    executable_sha256   = $exeSha
    self_test_args      = @($SelfTestArgs)
  }
}

try {
  $tools = New-Object System.Collections.ArrayList

  # FFmpeg : builds « essentials » de gyan.dev (NVENC, AMF et QSV inclus), miroir GitHub à releases versionnées.
  [void]$tools.Add((Resolve-ToolEntry -Id "ffmpeg" -Variant "default" -Repo "GyanD/codexffmpeg" `
      -TagRegex '^\d+(\.\d+)+$' -AssetRegex '^ffmpeg-.*-essentials_build\.zip$' `
      -ArchiveKind "zip" -ExecutableName "ffmpeg.exe" -SelfTestArgs @("-version")))

  [void]$tools.Add((Resolve-ToolEntry -Id "yt_dlp" -Variant "default" -Repo "yt-dlp/yt-dlp" `
      -TagRegex '.*' -AssetRegex '^yt-dlp\.exe$' `
      -ArchiveKind "binary" -ExecutableName "yt-dlp.exe" -SelfTestArgs @("--version")))

  [void]$tools.Add((Resolve-ToolEntry -Id "twitch_downloader" -Variant "default" -Repo "lay295/TwitchDownloader" `
      -TagRegex '.*' -AssetRegex '^TwitchDownloaderCLI.*Windows.*x64\.zip$' `
      -ArchiveKind "zip" -ExecutableName "TwitchDownloaderCLI.exe" -SelfTestArgs @("--version")))

  # whisper.cpp : seules les variantes CUDA et CPU existent en binaires officiels pour Windows.
  # Il n'y a pas de binaire Vulkan officiel : ne pas en déclarer sans l'avoir compilé et hébergé soi-même.
  if (-not $SkipWhisperCuda) {
    [void]$tools.Add((Resolve-ToolEntry -Id "whisper_cpp" -Variant "cuda" -Repo "ggml-org/whisper.cpp" `
        -TagRegex '^v\d+\.\d+\.\d+$' -AssetRegex '^whisper-cublas-12.*-bin-x64\.zip$' `
        -ArchiveKind "zip" -ExecutableName "whisper-cli.exe" -SelfTestArgs @("--help")))
  }

  [void]$tools.Add((Resolve-ToolEntry -Id "whisper_cpp" -Variant "cpu" -Repo "ggml-org/whisper.cpp" `
      -TagRegex '^v\d+\.\d+\.\d+$' -AssetRegex '^whisper-bin-x64\.zip$' `
      -ArchiveKind "zip" -ExecutableName "whisper-cli.exe" -SelfTestArgs @("--help")))

  $manifest = [ordered]@{
    schema_version = 1
    published_at   = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
    tools          = @($tools)
  }

  $json = $manifest | ConvertTo-Json -Depth 6
  $outPath = [System.IO.Path]::GetFullPath($OutManifest)
  # UTF-8 sans BOM : serde_json refuse un BOM en tête de fichier.
  [System.IO.File]::WriteAllText($outPath, $json, (New-Object System.Text.UTF8Encoding($false)))

  Write-Host ""
  Write-Host "Manifeste mis à jour : $outPath" -ForegroundColor Green
  Write-Host "Relancez l'application (le manifeste est compilé dans le binaire)." -ForegroundColor Green
}
finally {
  if (Test-Path $TempRoot) { Remove-Item -Path $TempRoot -Recurse -Force -ErrorAction SilentlyContinue }
}

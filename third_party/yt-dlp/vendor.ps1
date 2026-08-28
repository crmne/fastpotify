# Download one official yt-dlp asset for a cargo target. Runtime never downloads.
[CmdletBinding()]
param(
    [string]$Target,
    [switch]$Require,
    [string]$Manifest
)

$ErrorActionPreference = "Stop"
$Root = $PSScriptRoot
if (-not $Manifest) {
    $Manifest = Join-Path $Root "manifest"
}

function Get-HostTarget {
    $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
    if ($IsLinux) {
        switch ($arch) {
            "X64" { return "x86_64-unknown-linux-gnu" }
            "Arm64" { return "aarch64-unknown-linux-gnu" }
        }
    }
    elseif ($IsMacOS) {
        switch ($arch) {
            "X64" { return "x86_64-apple-darwin" }
            "Arm64" { return "aarch64-apple-darwin" }
        }
    }
    else {
        switch ($arch) {
            "X64" { return "x86_64-pc-windows-msvc" }
            "Arm64" { return "aarch64-pc-windows-msvc" }
        }
        switch ($env:PROCESSOR_ARCHITECTURE) {
            "AMD64" { return "x86_64-pc-windows-msvc" }
            "ARM64" { return "aarch64-pc-windows-msvc" }
        }
    }
    return $null
}

if (-not $Target) {
    $Target = Get-HostTarget
}
if (-not $Target) {
    throw "vendor.ps1: cannot detect host target; pass -Target"
}
if (-not (Test-Path -LiteralPath $Manifest)) {
    throw "vendor.ps1: missing manifest: $Manifest"
}

$tag = $null
$baseUrl = $null
$sha256sumsName = "SHA2-256SUMS"
$upstream = $null
$expected = $null

Get-Content -LiteralPath $Manifest | ForEach-Object {
    $line = $_.Trim()
    if ($line -eq "" -or $line.StartsWith("#")) {
        return
    }
    $eq = $line.IndexOf("=")
    if ($eq -lt 1) {
        return
    }
    $key = $line.Substring(0, $eq)
    $value = $line.Substring($eq + 1)
    switch ($key) {
        "tag" { $tag = $value }
        "base_url" { $baseUrl = $value }
        "sha256sums" { $sha256sumsName = $value }
        "asset" {
            $parts = $value.Split(@(" ", "`t"), [System.StringSplitOptions]::RemoveEmptyEntries)
            if ($parts.Length -ge 3 -and $parts[0] -eq $Target) {
                $upstream = $parts[1]
                $expected = $parts[2].ToLowerInvariant()
            }
        }
    }
}

if (-not $upstream -or -not $expected) {
    throw "vendor.ps1: no asset in manifest for $Target"
}
if (-not $baseUrl -or -not $tag) {
    throw "vendor.ps1: manifest is missing base_url or tag"
}

$destDir = Join-Path $Root "bin\$Target"
$dest = Join-Path $destDir $upstream
New-Item -ItemType Directory -Force -Path $destDir | Out-Null

function Get-Sha256Lower([string]$Path) {
    return (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Invoke-Download([string]$Url, [string]$OutFile) {
    $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
    if ($curl) {
        & curl.exe -fL --retry 3 --retry-delay 1 -A "fastpotify-ytdlp-vendor" -o $OutFile $Url
        if ($LASTEXITCODE -ne 0) {
            throw "vendor.ps1: curl failed for $Url"
        }
        return
    }
    Invoke-WebRequest -Uri $Url -OutFile $OutFile -UseBasicParsing -UserAgent "fastpotify-ytdlp-vendor"
}

if (Test-Path -LiteralPath $dest) {
    $got = Get-Sha256Lower $dest
    if ($got -eq $expected) {
        Write-Host "vendor.ps1: $Target already has $upstream ($expected)"
        exit 0
    }
    Write-Host "vendor.ps1: existing $dest has hash $got, expected $expected; re-downloading"
}

$work = Join-Path ([System.IO.Path]::GetTempPath()) ("fastpotify-ytdlp-" + [guid]::NewGuid().ToString("n"))
New-Item -ItemType Directory -Force -Path $work | Out-Null
try {
    $sumsPath = Join-Path $work $sha256sumsName
    $part = Join-Path $work ($upstream + ".part")
    $sumsUrl = "$baseUrl/$sha256sumsName"
    $assetUrl = "$baseUrl/$upstream"

    Write-Host "vendor.ps1: fetching $sha256sumsName from tag $tag"
    Invoke-Download $sumsUrl $sumsPath
    Write-Host "vendor.ps1: fetching $upstream"
    Invoke-Download $assetUrl $part

    $got = Get-Sha256Lower $part
    if ($got -ne $expected) {
        throw "vendor.ps1: hash mismatch for ${upstream}: got $got expected $expected"
    }

    $sumsHash = $null
    Get-Content -LiteralPath $sumsPath | ForEach-Object {
        $sumline = $_.Trim()
        if ($sumline -eq "") {
            return
        }
        $fields = $sumline.Split(@(" ", "`t"), [System.StringSplitOptions]::RemoveEmptyEntries)
        if ($fields.Length -lt 2) {
            return
        }
        $name = $fields[$fields.Length - 1].TrimStart("*")
        if ($name -eq $upstream) {
            $sumsHash = $fields[0].ToLowerInvariant()
        }
    }
    if (-not $sumsHash) {
        throw "vendor.ps1: $upstream is not in $sha256sumsName"
    }
    if ($sumsHash -ne $expected) {
        throw "vendor.ps1: $sha256sumsName has $sumsHash for $upstream, manifest has $expected"
    }
    if ($got -ne $sumsHash) {
        throw "vendor.ps1: downloaded $upstream does not match $sha256sumsName"
    }

    $tmpDest = "$dest.part.$pid"
    Copy-Item -LiteralPath $part -Destination $tmpDest -Force
    Move-Item -LiteralPath $tmpDest -Destination $dest -Force
}
finally {
    Remove-Item -LiteralPath $work -Recurse -Force -ErrorAction SilentlyContinue
}

if (-not (Test-Path -LiteralPath $dest)) {
    throw "vendor.ps1: failed to write $dest"
}
if ($Require -and -not (Test-Path -LiteralPath $dest)) {
    throw "vendor.ps1: required asset missing: $dest"
}

Write-Host "vendor.ps1: wrote $dest"
Write-Host "vendor.ps1: sha256 $got"

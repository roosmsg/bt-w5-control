# Renders ui\icon.svg to ui\icon.ico (16-256 px, PNG-compressed entries) with headless Chrome.
#   pwsh -File scripts\build-icon.ps1
$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
$svg = Join-Path $repo 'ui\icon.svg'
$ico = Join-Path $repo 'ui\icon.ico'
$chrome = @(
    "$env:ProgramFiles\Google\Chrome\Application\chrome.exe",
    "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
    "$env:LOCALAPPDATA\Google\Chrome\Application\chrome.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $chrome) { throw 'Chrome is required to render the icon.' }

$work = Join-Path ([IO.Path]::GetTempPath()) "bt-w5-control-icon"
New-Item -ItemType Directory -Force $work | Out-Null
$svgText = Get-Content $svg -Raw
$sizes = 16, 20, 24, 32, 40, 48, 64, 128, 256
$pngs = foreach ($size in $sizes) {
    $html = Join-Path $work "icon-$size.html"
    $png = Join-Path $work "icon-$size.png"
    $markup = $svgText -replace '<svg ', "<svg width=`"$size`" height=`"$size`" "
    Set-Content $html "<!doctype html><html><body style=`"margin:0;background:transparent`">$markup</body></html>"
    & $chrome --headless=new --disable-gpu --hide-scrollbars --default-background-color=00000000 `
        "--window-size=$size,$size" "--screenshot=$png" "file:///$($html -replace '\\','/')" 2>$null | Out-Null
    if (-not (Test-Path $png)) { throw "Rendering $size px failed." }
    [pscustomobject]@{ Size = $size; Bytes = [IO.File]::ReadAllBytes($png) }
}

# ICO container: header, one directory entry per image, then the PNG data.
$stream = New-Object IO.MemoryStream
$writer = New-Object IO.BinaryWriter($stream)
$writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$pngs.Count)
$offset = 6 + 16 * $pngs.Count
foreach ($p in $pngs) {
    $dim = if ($p.Size -ge 256) { 0 } else { $p.Size }
    $writer.Write([byte]$dim); $writer.Write([byte]$dim); $writer.Write([byte]0); $writer.Write([byte]0)
    $writer.Write([uint16]1); $writer.Write([uint16]32)
    $writer.Write([uint32]$p.Bytes.Length); $writer.Write([uint32]$offset)
    $offset += $p.Bytes.Length
}
foreach ($p in $pngs) { $writer.Write($p.Bytes) }
$writer.Flush()
[IO.File]::WriteAllBytes($ico, $stream.ToArray())
Remove-Item $work -Recurse -Force
"Wrote $ico ($($stream.Length) bytes, sizes $($sizes -join ', '))"

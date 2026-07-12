param([string]$ProjectRoot = "")

$ErrorActionPreference = "Stop"
if ([string]::IsNullOrWhiteSpace($ProjectRoot)) {
    $ProjectRoot = Join-Path $PSScriptRoot ".."
}
$ProjectRoot = (Resolve-Path -LiteralPath $ProjectRoot).ProviderPath
$iconDir = Join-Path $ProjectRoot "resources/icons"
New-Item -ItemType Directory -Force -Path $iconDir | Out-Null

Add-Type -AssemblyName System.Drawing

function New-SyncHubBitmap([int]$Size) {
    $bitmap = [System.Drawing.Bitmap]::new($Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $bitmap.SetResolution(96, 96)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $graphics.Clear([System.Drawing.Color]::Transparent)
    $scale = $Size / 256.0
    $radius = 56 * $scale
    $path = [System.Drawing.Drawing2D.GraphicsPath]::new()
    $diameter = 2 * $radius
    $path.AddArc(0, 0, $diameter, $diameter, 180, 90)
    $path.AddArc($Size - $diameter, 0, $diameter, $diameter, 270, 90)
    $path.AddArc($Size - $diameter, $Size - $diameter, $diameter, $diameter, 0, 90)
    $path.AddArc(0, $Size - $diameter, $diameter, $diameter, 90, 90)
    $path.CloseFigure()
    $graphics.FillPath([System.Drawing.SolidBrush]::new([System.Drawing.ColorTranslator]::FromHtml("#2563EB")), $path)

    $white = [System.Drawing.Pen]::new([System.Drawing.Color]::White, [Math]::Max(2, 22 * $scale))
    $white.StartCap = $white.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
    $topArc = [System.Drawing.Drawing2D.GraphicsPath]::new()
    $topArc.AddBezier(53*$scale, 112*$scale, 61*$scale, 72*$scale, 103*$scale, 44*$scale, 176*$scale, 66*$scale)
    $graphics.DrawPath($white, $topArc)
    $bottomArc = [System.Drawing.Drawing2D.GraphicsPath]::new()
    $bottomArc.AddBezier(203*$scale, 144*$scale, 195*$scale, 184*$scale, 153*$scale, 212*$scale, 80*$scale, 190*$scale)
    $graphics.DrawPath($white, $bottomArc)
    $arrowBrush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::White)
    $graphics.FillPolygon($arrowBrush, [System.Drawing.PointF[]]@(
        [System.Drawing.PointF]::new(169*$scale, 45*$scale), [System.Drawing.PointF]::new(204*$scale, 58*$scale), [System.Drawing.PointF]::new(177*$scale, 83*$scale)))
    $graphics.FillPolygon($arrowBrush, [System.Drawing.PointF[]]@(
        [System.Drawing.PointF]::new(87*$scale, 211*$scale), [System.Drawing.PointF]::new(52*$scale, 198*$scale), [System.Drawing.PointF]::new(79*$scale, 173*$scale)))
    $hubRect = [System.Drawing.RectangleF]::new(91*$scale, 91*$scale, 74*$scale, 74*$scale)
    $hubRadius = 20*$scale
    $hubPath = [System.Drawing.Drawing2D.GraphicsPath]::new()
    $hubDiameter = 2*$hubRadius
    $hubPath.AddArc($hubRect.X, $hubRect.Y, $hubDiameter, $hubDiameter, 180, 90)
    $hubPath.AddArc($hubRect.Right-$hubDiameter, $hubRect.Y, $hubDiameter, $hubDiameter, 270, 90)
    $hubPath.AddArc($hubRect.Right-$hubDiameter, $hubRect.Bottom-$hubDiameter, $hubDiameter, $hubDiameter, 0, 90)
    $hubPath.AddArc($hubRect.X, $hubRect.Bottom-$hubDiameter, $hubDiameter, $hubDiameter, 90, 90)
    $hubPath.CloseFigure()
    $graphics.FillPath([System.Drawing.SolidBrush]::new([System.Drawing.ColorTranslator]::FromHtml("#0F766E")), $hubPath)
    $graphics.DrawPath([System.Drawing.Pen]::new([System.Drawing.Color]::White, [Math]::Max(2, 10*$scale)), $hubPath)
    $graphics.FillEllipse([System.Drawing.SolidBrush]::new([System.Drawing.ColorTranslator]::FromHtml("#A7F3D0")), 118*$scale, 118*$scale, 20*$scale, 20*$scale)
    $graphics.Dispose()
    return $bitmap
}

$sizes = @(16, 24, 32, 48, 64, 128, 256)
$pngData = @()
foreach ($size in $sizes) {
    $bitmap = New-SyncHubBitmap $size
    $path = Join-Path $iconDir "icon-$size.png"
    $bitmap.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bitmap.Dispose()
    $pngData += ,([System.IO.File]::ReadAllBytes($path))
}
Copy-Item -LiteralPath (Join-Path $iconDir "icon-256.png") -Destination (Join-Path $iconDir "icon.png") -Force

$icoPath = Join-Path $iconDir "icon.ico"
$stream = [System.IO.File]::Create($icoPath)
$writer = [System.IO.BinaryWriter]::new($stream)
try {
    $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$sizes.Count)
    $offset = 6 + 16 * $sizes.Count
    for ($i = 0; $i -lt $sizes.Count; $i++) {
        $dimension = if ($sizes[$i] -eq 256) { 0 } else { $sizes[$i] }
        $writer.Write([byte]$dimension); $writer.Write([byte]$dimension)
        $writer.Write([byte]0); $writer.Write([byte]0)
        $writer.Write([uint16]1); $writer.Write([uint16]32)
        $writer.Write([uint32]$pngData[$i].Length); $writer.Write([uint32]$offset)
        $offset += $pngData[$i].Length
    }
    foreach ($bytes in $pngData) { $writer.Write($bytes) }
}
finally {
    $writer.Dispose()
    $stream.Dispose()
}

Write-Output "SyncHub icons generated in $iconDir"

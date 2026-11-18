# Builds every Docky brand image: the app icon (dock and Settings), the site
# favicons and social card, and the README banner.
#
# The logo is a geometric D monogram — a white capsule bar and an orange
# half-disc bowl on a deep purple tile — drawn as vectors at every size, so
# each output is crisp without any upscaling. Run from the repository root:
#
#   pwsh ./scripts/make_icon.ps1
#
# Writes:
#   assets/brand/docky-1024.png, assets/brand/docky.ico, assets/brand/docky-banner.png
#   apps/settings/src-tauri/icons/icon.ico
#   docky/src/assets/brand/docky-logo.png, docky-app.png
#   docky/public/favicon-16x16.png, favicon-32x32.png, apple-touch-icon.png,
#         android-chrome-192x192.png, android-chrome-512x512.png, og-image.png

Add-Type -AssemblyName System.Drawing

$Root = Split-Path -Parent $PSScriptRoot
$Brand = Join-Path $Root 'assets\brand'
$Site = Join-Path $Root 'docky'

$Purple = [System.Drawing.Color]::FromArgb(91, 46, 190)    # #5B2EBE
$Orange = [System.Drawing.Color]::FromArgb(238, 98, 52)    # #EE6234
$Ink = [System.Drawing.Color]::FromArgb(22, 18, 30)        # #16121E
$Lilac = [System.Drawing.Color]::FromArgb(156, 143, 208)   # #9C8FD0

# Tile and monogram geometry, in 1024-unit coordinates.
$MarginRatio = 72 / 1024
$RadiusRatio = 0.225
# Fine variant (>= 64 px) and a bolder one for tiny icons, where the gap
# between bar and bowl would otherwise close up.
$Variants = @{
    fine = @{ barWidth = 106; gap = 48; bowlRadius = 250; height = 500 }
    bold = @{ barWidth = 150; gap = 64; bowlRadius = 250; height = 500 }
}

function New-TilePath([single]$size) {
    $m = $size * $MarginRatio
    $t = $size - 2 * $m
    $d = $t * $RadiusRatio * 2
    $p = [System.Drawing.Drawing2D.GraphicsPath]::new()
    $p.AddArc($m, $m, $d, $d, 180, 90)
    $p.AddArc($m + $t - $d, $m, $d, $d, 270, 90)
    $p.AddArc($m + $t - $d, $m + $t - $d, $d, $d, 0, 90)
    $p.AddArc($m, $m + $t - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    return $p
}

function New-BarPath([single]$size, [hashtable]$variant) {
    $k = $size / 1024
    $w = $variant.barWidth * $k
    $h = $variant.height * $k
    $total = $w + $variant.gap * $k + $variant.bowlRadius * $k
    $left = $size / 2 - $total / 2
    $top = $size / 2 - $h / 2
    $d = $w
    $p = [System.Drawing.Drawing2D.GraphicsPath]::new()
    $p.AddArc($left, $top, $d, $d, 180, 90)
    $p.AddArc($left + $w - $d, $top, $d, $d, 270, 90)
    $p.AddArc($left + $w - $d, $top + $h - $d, $d, $d, 0, 90)
    $p.AddArc($left, $top + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    return $p
}

function New-BowlPath([single]$size, [hashtable]$variant) {
    $k = $size / 1024
    $r = $variant.bowlRadius * $k
    $h = $variant.height * $k
    $w = $variant.barWidth * $k
    $total = $w + $variant.gap * $k + $r
    $flat = $size / 2 - $total / 2 + $w + $variant.gap * $k
    $top = $size / 2 - $h / 2
    $p = [System.Drawing.Drawing2D.GraphicsPath]::new()
    # A half-disc: arc over the right side, closed by the flat left edge.
    $p.AddArc($flat - $r, $top, 2 * $r, $h, -90, 180)
    $p.CloseFigure()
    return $p
}

function New-IconBitmap([int]$size) {
    $variant = if ($size -le 48) { $Variants.bold } else { $Variants.fine }
    $bmp = [System.Drawing.Bitmap]::new($size, $size)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $tile = New-TilePath $size
    $g.FillPath([System.Drawing.SolidBrush]::new($Purple), $tile)
    $g.FillPath([System.Drawing.SolidBrush]::new($Orange), (New-BowlPath $size $variant))
    $g.FillPath([System.Drawing.SolidBrush]::new([System.Drawing.Color]::White), (New-BarPath $size $variant))
    $g.Dispose()
    return $bmp
}

function Get-PngBytes([System.Drawing.Bitmap]$bmp) {
    $ms = [System.IO.MemoryStream]::new()
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    return $ms.ToArray()
}

function Get-DibBytes([System.Drawing.Bitmap]$bmp) {
    # BMP-style ICO entry: BITMAPINFOHEADER, BGRA rows bottom-up, then the
    # (empty, since the alpha channel rules) AND mask.
    $s = $bmp.Width
    $rect = [System.Drawing.Rectangle]::new(0, 0, $s, $s)
    $data = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $stride = $data.Stride
        $buf = [byte[]]::new($stride * $s)
        [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $buf, 0, $buf.Length)
    }
    finally {
        $bmp.UnlockBits($data)
    }
    $xorRow = 4 * $s
    $andRow = [int][Math]::Ceiling([Math]::Ceiling($s / 8) / 4) * 4
    $imageSize = ($xorRow + $andRow) * $s
    $ms = [System.IO.MemoryStream]::new()
    $bw = [System.IO.BinaryWriter]::new($ms)
    $bw.Write([uint32]40)
    $bw.Write([int32]$s)
    $bw.Write([int32](2 * $s))
    $bw.Write([uint16]1)
    $bw.Write([uint16]32)
    $bw.Write([uint32]0)
    $bw.Write([uint32]$imageSize)
    $bw.Write([int32]0)
    $bw.Write([int32]0)
    $bw.Write([uint32]0)
    $bw.Write([uint32]0)
    for ($y = $s - 1; $y -ge 0; $y--) { $bw.Write($buf, $y * $stride, $xorRow) }
    $bw.Write([byte[]]::new($andRow * $s))
    $bw.Flush()
    return $ms.ToArray()
}

function Save-Ico([int[]]$sizes, [string]$path) {
    $blobs = @()
    foreach ($size in $sizes) {
        $bmp = New-IconBitmap $size
        $blob = if ($size -ge 256) { Get-PngBytes $bmp } else { Get-DibBytes $bmp }
        $bmp.Dispose()
        $blobs += , @($size, $blob)
    }
    $ms = [System.IO.MemoryStream]::new()
    $bw = [System.IO.BinaryWriter]::new($ms)
    $bw.Write([uint16]0)
    $bw.Write([uint16]1)
    $bw.Write([uint16]$blobs.Count)
    $offset = 6 + 16 * $blobs.Count
    foreach ($entry in $blobs) {
        $size = $entry[0]
        $blob = $entry[1]
        $bw.Write([byte]($(if ($size -ge 256) { 0 } else { $size })))
        $bw.Write([byte]($(if ($size -ge 256) { 0 } else { $size })))
        $bw.Write([byte]0)
        $bw.Write([byte]0)
        $bw.Write([uint16]1)
        $bw.Write([uint16]32)
        $bw.Write([uint32]$blob.Length)
        $bw.Write([uint32]$offset)
        $offset += $blob.Length
    }
    foreach ($entry in $blobs) { $bw.Write([byte[]]$entry[1]) }
    $bw.Flush()
    [IO.File]::WriteAllBytes($path, $ms.ToArray())
}

function New-CardBitmap([int]$width, [int]$height, [int]$iconSize, [scriptblock]$drawText) {
    # A self-contained rounded panel, so it reads the same on light and dark
    # GitHub themes.
    $bmp = [System.Drawing.Bitmap]::new($width, $height)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    $panel = [System.Drawing.Drawing2D.GraphicsPath]::new()
    $d = 2 * 48
    $panel.AddArc(0, 0, $d, $d, 180, 90)
    $panel.AddArc($width - $d, 0, $d, $d, 270, 90)
    $panel.AddArc($width - $d, $height - $d, $d, $d, 0, 90)
    $panel.AddArc(0, $height - $d, $d, $d, 90, 90)
    $panel.CloseFigure()
    $g.FillPath([System.Drawing.SolidBrush]::new($Ink), $panel)
    $icon = New-IconBitmap $iconSize
    $y = [int](($height - $iconSize) / 2)
    $g.DrawImage($icon, 48, $y, $iconSize, $iconSize)
    $icon.Dispose()
    & $drawText $g
    $g.Dispose()
    return $bmp
}

function New-LogoBitmap {
    # Header/footer wordmark: icon plus "Docky", transparent background so it
    # sits on the dark site chrome. Measured typographically, with no padding.
    $iconSize = 512
    $font = [System.Drawing.Font]::new('Segoe UI', [single]280, [System.Drawing.FontStyle]::Bold)
    $fmt = [System.Drawing.StringFormat]::GenericTypographic
    $probe = [System.Drawing.Bitmap]::new(8, 8)
    $pg = [System.Drawing.Graphics]::FromImage($probe)
    $sz = $pg.MeasureString('Docky', $font, [System.Drawing.PointF]::new(0, 0), $fmt)
    $pg.Dispose()
    $probe.Dispose()
    $gap = 48
    $pad = 16
    $width = [int]($iconSize + $gap + $sz.Width + $pad)
    $bmp = [System.Drawing.Bitmap]::new($width, $iconSize)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
    $icon = New-IconBitmap $iconSize
    $g.DrawImage($icon, 0, 0, $iconSize, $iconSize)
    $icon.Dispose()
    $point = [System.Drawing.PointF]::new($iconSize + $gap, ($iconSize - $sz.Height) / 2)
    $g.DrawString('Docky', $font, [System.Drawing.SolidBrush]::new([System.Drawing.Color]::White), $point, $fmt)
    $g.Dispose()
    $font.Dispose()
    return $bmp
}

function Save-Png([System.Drawing.Bitmap]$bmp, [string]$path) {
    $dir = Split-Path -Parent $path
    if (-not (Test-Path $dir)) { New-Item -ItemType Directory -Force $dir | Out-Null }
    $bmp.Save($path, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    "wrote $path"
}

$IcoSizes = @(16, 20, 24, 32, 40, 48, 64, 96, 128, 256)

# 1. App icon: master PNG, the dock's .ico, and the Settings app's .ico.
Save-Png (New-IconBitmap 1024) (Join-Path $Brand 'docky-1024.png')
Save-Ico $IcoSizes (Join-Path $Brand 'docky.ico')
Copy-Item (Join-Path $Brand 'docky.ico') (Join-Path $Root 'apps\settings\src-tauri\icons\icon.ico') -Force
"wrote docky.ico and the Settings icon"

# 2. Site brand images and favicons.
Save-Png (New-LogoBitmap) (Join-Path $Site 'src\assets\brand\docky-logo.png')
Save-Png (New-IconBitmap 512) (Join-Path $Site 'src\assets\brand\docky-app.png')
Save-Png (New-IconBitmap 16) (Join-Path $Site 'public\favicon-16x16.png')
Save-Png (New-IconBitmap 32) (Join-Path $Site 'public\favicon-32x32.png')
Save-Png (New-IconBitmap 180) (Join-Path $Site 'public\apple-touch-icon.png')
Save-Png (New-IconBitmap 192) (Join-Path $Site 'public\android-chrome-192x192.png')
Save-Png (New-IconBitmap 512) (Join-Path $Site 'public\android-chrome-512x512.png')

# 3. Social card: icon, wordmark and tagline.
$og = New-CardBitmap 1200 630 240 {
    param($g)
    $wordmark = [System.Drawing.Font]::new('Segoe UI', 108, [System.Drawing.FontStyle]::Bold)
    $tagline = [System.Drawing.Font]::new('Segoe UI', 33)
    $g.DrawString('Docky', $wordmark, [System.Drawing.SolidBrush]::new([System.Drawing.Color]::White), 400, 205)
    $g.DrawString('The macOS Dock, made for Windows.', $tagline, [System.Drawing.SolidBrush]::new($Lilac), 408, 388)
    $wordmark.Dispose(); $tagline.Dispose()
}
Save-Png $og (Join-Path $Site 'public\og-image.png')

# 4. README banner.
$banner = New-CardBitmap 1200 320 224 {
    param($g)
    $wordmark = [System.Drawing.Font]::new('Segoe UI', 84, [System.Drawing.FontStyle]::Bold)
    $tagline = [System.Drawing.Font]::new('Segoe UI', 27)
    $g.DrawString('Docky', $wordmark, [System.Drawing.SolidBrush]::new([System.Drawing.Color]::White), 336, 66)
    $g.DrawString('The macOS Dock, made for Windows.', $tagline, [System.Drawing.SolidBrush]::new($Lilac), 342, 208)
    $wordmark.Dispose(); $tagline.Dispose()
}
Save-Png $banner (Join-Path $Brand 'docky-banner.png')

# Copyright (C) 2026 SkapaCraft <https://skapacraft.com>
# SPDX-License-Identifier: GPL-3.0-or-later

<#
.SYNOPSIS
Starts the application on Windows and checks that the page actually renders.

.DESCRIPTION
This lives in a file rather than inline in the workflow because it needs a
PowerShell here-string, whose terminator has to sit at column zero: inside a
YAML block scalar that breaks the indentation and the workflow stops parsing.

Two things are worth knowing about what it checks.

The window title proves nothing. Tauri sets it from the configuration before
the webview does anything, so it appears identically whether the page loaded or
not. What proves the page rendered is that WebView2 spawned a renderer child
process, which only happens once there is content to draw.

The screenshot uses PrintWindow with PW_RENDERFULLCONTENT. WebView2 draws
through DirectComposition, and a plain BitBlt (which is what CopyFromScreen
does) returns a blank rectangle for that kind of surface: the window frame and
the menu appear, the content does not. That is a limitation of the capture, not
of the application, and it is exactly what made an earlier version of this check
produce a white screenshot from a working application.
#>

$ErrorActionPreference = "Stop"

$bin = "src-tauri\target\release\open-takeout-hub.exe"
if (-not (Test-Path $bin)) { throw "binary not found at $bin" }

$app = Start-Process -FilePath $bin -PassThru

# Wait for the window.
$deadline = (Get-Date).AddSeconds(60)
$title = ""
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 2
    $app.Refresh()
    if ($app.HasExited) { throw "the application exited during startup" }
    if ($app.MainWindowTitle) { $title = $app.MainWindowTitle; break }
}
if (-not $title) {
    Stop-Process -Id $app.Id -Force
    throw "no window title appeared within 60 s"
}
Write-Host "Window title: $title"

# Wait for the page.
$deadline = (Get-Date).AddSeconds(60)
$renderer = $null
while ((Get-Date) -lt $deadline) {
    Start-Sleep -Seconds 2
    $renderer = Get-CimInstance Win32_Process -Filter "Name = 'msedgewebview2.exe'" |
        Where-Object { $_.CommandLine -like "*--type=renderer*" }
    if ($renderer) { break }
}
if (-not $renderer) {
    Stop-Process -Id $app.Id -Force
    throw "WebView2 never started a renderer: the page did not load"
}
Write-Host "WebView2 renderer processes: $(@($renderer).Count)"

# A window that appears and then dies is not a working application.
Start-Sleep -Seconds 10
$app.Refresh()
if ($app.HasExited) { throw "the application exited ten seconds after opening its window" }

Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;

public class WindowShot
{
    [DllImport("user32.dll")]
    public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);

    public struct Rect { public int Left, Top, Right, Bottom; }

    public const uint RenderFullContent = 2;
}
"@

$handle = $app.MainWindowHandle
$rect = New-Object WindowShot+Rect
[void][WindowShot]::GetWindowRect($handle, [ref]$rect)

$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
if ($width -le 0 -or $height -le 0) { throw "the window reported a size of $width x $height" }

$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$hdc = $graphics.GetHdc()
[void][WindowShot]::PrintWindow($handle, $hdc, [WindowShot]::RenderFullContent)
$graphics.ReleaseHdc($hdc)
$bitmap.Save((Join-Path (Get-Location) "smoke-windows.png"))

Stop-Process -Id $app.Id -Force
Write-Host "The page rendered and the application stayed alive."

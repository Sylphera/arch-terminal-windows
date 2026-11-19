# Fastfetch presentation for the native animated Arch startup screen.
function global:Show-IiStartupFetch {
    $nativeFetch = Join-Path $PSScriptRoot 'fetch-arch-personalized.exe'
    if (Test-Path -LiteralPath $nativeFetch) {
        $shellName = 'PowerShell ' + $PSVersionTable.PSVersion.ToString()
        & $nativeFetch ('--shell=' + $shellName)
    } else {
        $originalSplash = Join-Path $env:LOCALAPPDATA 'ii-terminal-optimizations\splash.ps1'
        if (Test-Path -LiteralPath $originalSplash) {
            . $originalSplash
            Show-IiStartupFetch
        }
    }
}
